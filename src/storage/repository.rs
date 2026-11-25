//! File-backed artifact store.
//!
//! Only the small [`ArtifactMeta`] records are held in memory; full artifacts
//! are read from `data/<id>.json` when they are displayed. All writes are
//! atomic (temp file + rename), so a crash never leaves a half-written file.

use std::{
    collections::hash_map::RandomState,
    fs,
    hash::{BuildHasher, Hasher},
    io::{BufRead, BufReader, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use crate::{
    config::Config,
    error::{Error, Result},
    protocol::{Artifact, ArtifactInput, ArtifactMeta, ArtifactPatch, Content, UserEvent},
    time,
};

const EVENTS_ROTATE_BYTES: u64 = 2 * 1024 * 1024;
const EVENTS_KEEP_LINES: usize = 1000;

pub struct Repository {
    cfg: Config,
    metas: Vec<ArtifactMeta>,
}

impl Repository {
    pub fn open(cfg: Config) -> Result<Self> {
        cfg.ensure_dirs()?;
        let mut repo = Self { cfg, metas: Vec::new() };
        repo.reload()?;
        Ok(repo)
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// Re-read the metadata directory (used when another process may have
    /// written to the store). Unreadable entries are skipped, not fatal.
    pub fn reload(&mut self) -> Result<()> {
        let mut metas = Vec::new();
        for entry in fs::read_dir(self.cfg.meta_dir())? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(bytes) = fs::read(&path)
                && let Ok(meta) = serde_json::from_slice::<ArtifactMeta>(&bytes) {
                    metas.push(meta);
                }
        }
        metas.sort_by_cached_key(sort_key);
        self.metas = metas;
        Ok(())
    }

    /// Artifacts in chronological order (oldest first).
    pub fn metas(&self) -> &[ArtifactMeta] {
        &self.metas
    }

    pub fn position(&self, id: &str) -> Option<usize> {
        self.metas.iter().position(|m| m.id == id)
    }

    /// Load one full artifact from disk.
    pub fn load(&self, id: &str) -> Result<Artifact> {
        if !crate::protocol::artifact::is_valid_id(id) {
            return Err(Error::invalid("bad artifact id"));
        }
        let path = self.data_path(id);
        let len = fs::metadata(&path).map_err(|_| Error::NotFound(id.to_string()))?.len();
        // Envelope overhead on top of the content limit.
        if len > self.cfg.limits.max_payload_bytes as u64 + 64 * 1024 {
            return Err(Error::TooLarge(format!("stored artifact {id} is {len} bytes")));
        }
        Ok(serde_json::from_slice(&fs::read(path)?)?)
    }

    /// Create an artifact, or replace the one with the same id (keeping its
    /// creation time and bumping its revision).
    pub fn publish(&mut self, input: ArtifactInput) -> Result<ArtifactMeta> {
        let now = time::rfc3339_millis(time::now_millis());
        let id = input.id.unwrap_or_else(|| format!("art-{}", random_hex(4)));
        let previous = self.load(&id).ok();
        let mut artifact = Artifact {
            id,
            title: input.title.unwrap_or_else(|| input.kind.clone()),
            kind: input.kind,
            description: input.description,
            created_at: match &previous {
                Some(p) => p.created_at.clone(),
                None => input.created_at.unwrap_or_else(|| now.clone()),
            },
            updated_at: previous.as_ref().map(|_| now),
            project_id: input.project_id,
            agent_id: input.agent_id,
            status: input.status,
            content: input.content,
            actions: input.actions,
            revision: previous.as_ref().map_or(1, |p| p.revision + 1),
            response: None,
        };
        self.store(&mut artifact, previous.as_ref())
    }

    pub fn update(&mut self, id: &str, patch: ArtifactPatch) -> Result<ArtifactMeta> {
        if patch.is_empty() {
            return Err(Error::invalid("update has no fields to change"));
        }
        let previous = self.load(id)?;
        let mut artifact = previous.clone();
        patch.apply(&mut artifact);
        artifact.revision += 1;
        artifact.updated_at = Some(time::rfc3339_millis(time::now_millis()));
        artifact.response = None;
        self.store(&mut artifact, Some(&previous))
    }

    /// Record which action the user chose, without bumping the revision.
    pub fn set_response(&mut self, id: &str, action_id: &str) -> Result<ArtifactMeta> {
        let mut artifact = self.load(id)?;
        artifact.response = Some(action_id.to_string());
        self.write_artifact(&artifact)
    }

    pub fn remove(&mut self, id: &str) -> Result<()> {
        let artifact = self.load(id)?;
        self.delete_files(&artifact);
        self.metas.retain(|m| m.id != id);
        Ok(())
    }

    fn store(&mut self, artifact: &mut Artifact, previous: Option<&Artifact>) -> Result<ArtifactMeta> {
        let content = artifact.validate(&self.cfg.limits)?;
        if let Content::Image { source } = &content {
            self.check_image(source)?;
        }
        let meta = self.write_artifact(artifact)?;
        // The old image file is unreferenced once the new version is stored.
        if let Some(old) = previous.and_then(image_source)
            && image_source(artifact) != Some(old) {
                let _ = fs::remove_file(self.cfg.files_dir().join(old));
            }
        self.prune();
        Ok(meta)
    }

    fn write_artifact(&mut self, artifact: &Artifact) -> Result<ArtifactMeta> {
        let meta = ArtifactMeta::from(artifact);
        write_atomic(&self.data_path(&artifact.id), &serde_json::to_vec(artifact)?)?;
        write_atomic(&self.meta_path(&artifact.id), &serde_json::to_vec(&meta)?)?;
        match self.position(&meta.id) {
            Some(i) => self.metas[i] = meta.clone(),
            None => {
                let at = self
                    .metas
                    .partition_point(|m| sort_key(m) <= sort_key(&meta));
                self.metas.insert(at, meta.clone());
            }
        }
        Ok(meta)
    }

    fn prune(&mut self) {
        let limit = self.cfg.limits.history_limit.max(1);
        while self.metas.len() > limit {
            let oldest = self.metas.remove(0);
            match self.load(&oldest.id) {
                Ok(artifact) => self.delete_files(&artifact),
                Err(_) => {
                    let _ = fs::remove_file(self.meta_path(&oldest.id));
                    let _ = fs::remove_file(self.data_path(&oldest.id));
                }
            }
        }
    }

    fn delete_files(&self, artifact: &Artifact) {
        if let Some(source) = image_source(artifact) {
            let _ = fs::remove_file(self.cfg.files_dir().join(source));
        }
        let _ = fs::remove_file(self.meta_path(&artifact.id));
        let _ = fs::remove_file(self.data_path(&artifact.id));
    }

    /// An image artifact must point at a real, decodable, size-bounded file
    /// inside `files/`. Only the header is read here.
    fn check_image(&self, source: &str) -> Result<()> {
        let path = self.cfg.files_dir().join(source);
        let meta = fs::symlink_metadata(&path).map_err(|_| Error::Invalid(format!("image file '{source}' is not in the store")))?;
        if !meta.is_file() {
            return Err(Error::invalid("image source is not a regular file"));
        }
        if meta.len() > self.cfg.limits.max_file_bytes {
            return Err(Error::TooLarge(format!("image is {} bytes, limit is {}", meta.len(), self.cfg.limits.max_file_bytes)));
        }
        let (w, h) = image::ImageReader::open(&path)?
            .with_guessed_format()?
            .into_dimensions()
            .map_err(|e| Error::Invalid(format!("not a supported image: {e}")))?;
        let max = self.cfg.limits.max_image_dimension;
        if w > max || h > max {
            return Err(Error::TooLarge(format!("image is {w}x{h}, limit is {max}x{max}")));
        }
        Ok(())
    }

    /// Append a user event for agents to read.
    pub fn append_event(&self, event: &UserEvent) -> Result<()> {
        let path = self.cfg.events_path();
        if fs::metadata(&path).map(|m| m.len()).unwrap_or(0) > EVENTS_ROTATE_BYTES {
            rotate_events(&path);
        }
        let mut line = serde_json::to_vec(event)?;
        line.push(b'\n');
        let mut file = fs::OpenOptions::new().create(true).append(true).open(path)?;
        file.write_all(&line)?;
        Ok(())
    }

    fn meta_path(&self, id: &str) -> PathBuf {
        self.cfg.meta_dir().join(format!("{id}.json"))
    }
    fn data_path(&self, id: &str) -> PathBuf {
        self.cfg.data_dir().join(format!("{id}.json"))
    }
}

/// Chronological order by the parsed timestamp (millisecond precision), not
/// by its text: `…00.500Z` must sort after `…00Z`. Ties fall back to the id.
fn sort_key(meta: &ArtifactMeta) -> (u64, String) {
    (time::parse_rfc3339_millis(&meta.created_at).unwrap_or(0), meta.id.clone())
}

fn image_source(artifact: &Artifact) -> Option<&str> {
    (artifact.kind == "image").then(|| artifact.content.get("source")?.as_str()).flatten()
}

/// Copy a user-supplied image into the store and return its new file name.
/// The source must be a regular file within the size limit; its content is
/// verified later by [`Repository::publish`].
pub fn ingest_file(cfg: &Config, source: &Path) -> Result<String> {
    cfg.ensure_dirs()?;
    let meta = fs::metadata(source).map_err(|e| Error::Invalid(format!("cannot read {}: {e}", source.display())))?;
    if !meta.is_file() {
        return Err(Error::Invalid(format!("{} is not a regular file", source.display())));
    }
    if meta.len() > cfg.limits.max_file_bytes {
        return Err(Error::TooLarge(format!("{} is {} bytes, limit is {}", source.display(), meta.len(), cfg.limits.max_file_bytes)));
    }
    let ext: String = source
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.chars().filter(char::is_ascii_alphanumeric).take(8).collect::<String>().to_ascii_lowercase())
        .filter(|e| !e.is_empty())
        .unwrap_or_else(|| "bin".to_string());
    let name = format!("img-{}.{ext}", random_hex(8));
    fs::copy(source, cfg.files_dir().join(&name))?;
    Ok(name)
}

/// Remove an ingested file that ended up unused (e.g. publish was rejected).
pub fn discard_file(cfg: &Config, name: &str) {
    if crate::protocol::artifact::is_store_file_name(name) {
        let _ = fs::remove_file(cfg.files_dir().join(name));
    }
}

/// Read events starting at byte `offset`. Returns the events and the offset
/// just past the last complete line, so callers can tail the file cheaply.
pub fn read_events(cfg: &Config, offset: u64) -> Result<(Vec<UserEvent>, u64)> {
    let mut file = match fs::File::open(cfg.events_path()) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((Vec::new(), 0)),
        Err(e) => return Err(e.into()),
    };
    // The file was rotated underneath us: start over.
    let mut pos = if file.metadata()?.len() < offset { 0 } else { offset };
    file.seek(SeekFrom::Start(pos))?;
    let mut reader = BufReader::new(file);
    let mut events = Vec::new();
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line)?;
        if n == 0 || !line.ends_with('\n') {
            break; // EOF or a partially written line; retry it next time.
        }
        pos += n as u64;
        if let Ok(event) = serde_json::from_str(&line) {
            events.push(event);
        }
    }
    Ok((events, pos))
}

fn rotate_events(path: &Path) {
    let Ok(text) = fs::read_to_string(path) else { return };
    let lines: Vec<&str> = text.lines().collect();
    let keep = &lines[lines.len().saturating_sub(EVENTS_KEEP_LINES)..];
    let mut out = keep.join("\n");
    out.push('\n');
    let _ = write_atomic(path, out.as_bytes());
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension(format!("tmp-{}-{}", std::process::id(), random_hex(2)));
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })?;
    Ok(())
}

/// Random hex from the OS-seeded SipHash keys std already carries; good
/// enough for ids and the local IPC token without pulling in a RNG crate.
pub fn random_hex(bytes: usize) -> String {
    let mut out = String::with_capacity(bytes * 2);
    while out.len() < bytes * 2 {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(time::now_millis());
        out.push_str(&format!("{:016x}", hasher.finish()));
    }
    out.truncate(bytes * 2);
    out
}
