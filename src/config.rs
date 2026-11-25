//! Paths and resource limits.
//!
//! Everything lives under one "home" directory so the TUI and the publishing
//! CLI (run by agents from any pane, with no plugin environment) agree on where
//! the store is:
//!
//! ```text
//! <home>/
//!   config.json    optional limit overrides
//!   ipc.token      shared secret for the local socket
//!   ipc.sock       Unix domain socket (Linux/macOS only)
//!   events.jsonl   user -> agent events
//!   meta/<id>.json small per-artifact metadata (what the history list needs)
//!   data/<id>.json full artifact, loaded on demand
//!   files/<name>   ingested image files
//! ```

use std::{
    env, fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::error::Result;

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default)]
pub struct Limits {
    /// Largest image file accepted by `publish image`.
    pub max_file_bytes: u64,
    /// Largest JSON request / artifact document.
    pub max_payload_bytes: usize,
    /// Largest accepted image width or height in pixels.
    pub max_image_dimension: u32,
    /// Number of encoded images kept in memory.
    pub image_cache_entries: usize,
    /// Artifacts kept on disk; the oldest are pruned beyond this.
    pub history_limit: usize,
    /// Largest accepted table (rows) and chart (points).
    pub max_rows: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_file_bytes: 10 * 1024 * 1024,
            max_payload_bytes: 1024 * 1024,
            max_image_dimension: 4096,
            image_cache_entries: 4,
            history_limit: 500,
            max_rows: 2000,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub home: PathBuf,
    pub limits: Limits,
}

impl Config {
    /// Resolve the home directory from the environment and read `config.json`
    /// if present. A malformed config file is an error rather than silently
    /// ignored, so limits never fall back to defaults unnoticed.
    pub fn load() -> Result<Self> {
        Self::with_home(default_home())
    }

    pub fn with_home(home: impl Into<PathBuf>) -> Result<Self> {
        let home = home.into();
        let limits = match fs::read(home.join("config.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(_) => Limits::default(),
        };
        Ok(Self { home, limits })
    }

    pub fn meta_dir(&self) -> PathBuf {
        self.home.join("meta")
    }
    pub fn data_dir(&self) -> PathBuf {
        self.home.join("data")
    }
    pub fn files_dir(&self) -> PathBuf {
        self.home.join("files")
    }
    pub fn events_path(&self) -> PathBuf {
        self.home.join("events.jsonl")
    }
    pub fn token_path(&self) -> PathBuf {
        self.home.join("ipc.token")
    }
    pub fn socket_path(&self) -> PathBuf {
        self.home.join("ipc.sock")
    }

    /// Create the directory layout. On Unix the home is made private (0700) so
    /// other users can reach neither the socket nor the token.
    pub fn ensure_dirs(&self) -> Result<()> {
        for dir in [self.meta_dir(), self.data_dir(), self.files_dir()] {
            fs::create_dir_all(dir)?;
        }
        restrict(&self.home, 0o700);
        Ok(())
    }
}

/// Best-effort owner-only permissions (no-op on Windows, where the per-user
/// profile directory ACL already restricts access).
pub fn restrict(path: &Path, mode: u32) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
    }
}

fn default_home() -> PathBuf {
    if let Some(home) = env::var_os("HERDR_ARTIFACTS_HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(home);
    }
    #[cfg(windows)]
    let base = env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let base = env::var_os("XDG_STATE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("state")));
    base.unwrap_or_else(env::temp_dir).join("herdr-artifacts")
}
