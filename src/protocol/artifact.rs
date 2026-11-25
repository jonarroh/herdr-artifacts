//! The artifact contract: envelope, per-type content and validation.
//!
//! The envelope keeps `type` as a plain string and `content` as opaque JSON,
//! so storage, transport and history never need to know about a specific
//! type. Only [`Content::parse`] and the renderer do; adding a type means one
//! new `Content` variant plus one renderer.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{
    config::Limits,
    error::{Error, Result},
    time,
};

pub const ARTIFACT_TYPES: [&str; 7] = ["image", "qr", "message", "markdown", "table", "chart", "code"];
pub const ACTION_TYPES: [&str; 3] = ["respond", "open_url", "dismiss"];

const MAX_ID: usize = 64;
const MAX_TITLE: usize = 200;
const MAX_DESCRIPTION: usize = 1000;
const MAX_SHORT: usize = 64;
const MAX_ACTIONS: usize = 9;
const MAX_URL: usize = 2048;
const MAX_QR_BYTES: usize = 2000;
const MAX_COLUMNS: usize = 32;

/// A stored artifact (the full JSON contract).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Artifact {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Free-form short state shown in the history, e.g. `running`, `done`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    pub content: Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<Action>,
    /// Incremented on every update; lets agents correlate responses.
    #[serde(default)]
    pub revision: u32,
    /// Id of the last action the user invoked, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Action {
    pub id: String,
    pub label: String,
    /// `respond` (default), `open_url` or `dismiss`.
    #[serde(rename = "type", default = "default_action_type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub payload: Value,
    /// Ask the user before running. `open_url` and `dismiss` always ask.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub confirm: bool,
}

fn default_action_type() -> String {
    "respond".to_string()
}

impl Action {
    pub fn needs_confirmation(&self) -> bool {
        self.confirm || self.kind != "respond"
    }

    pub fn url(&self) -> Option<&str> {
        self.payload.get("url").and_then(Value::as_str)
    }
}

/// What an agent sends to publish. Missing fields get defaults.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ArtifactInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default)]
    pub content: Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<Action>,
}

/// Partial update. `content` is shallow-merged into the existing object.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ArtifactPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Map<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions: Option<Vec<Action>>,
}

impl ArtifactPatch {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub fn apply(self, artifact: &mut Artifact) {
        if let Some(v) = self.title {
            artifact.title = v;
        }
        if let Some(v) = self.description {
            artifact.description = Some(v);
        }
        if let Some(v) = self.status {
            artifact.status = Some(v);
        }
        if let Some(patch) = self.content {
            match artifact.content.as_object_mut() {
                Some(obj) => obj.extend(patch),
                None => artifact.content = Value::Object(patch),
            }
        }
        if let Some(v) = self.actions {
            artifact.actions = v;
        }
    }
}

/// The small slice of an artifact the history list keeps in memory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactMeta {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub title: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default)]
    pub revision: u32,
    #[serde(default)]
    pub actions: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<String>,
}

impl From<&Artifact> for ArtifactMeta {
    fn from(a: &Artifact) -> Self {
        Self {
            id: a.id.clone(),
            kind: a.kind.clone(),
            title: a.title.clone(),
            created_at: a.created_at.clone(),
            project_id: a.project_id.clone(),
            agent_id: a.agent_id.clone(),
            status: a.status.clone(),
            revision: a.revision,
            actions: a.actions.len(),
            response: a.response.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    #[default]
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChartStyle {
    #[default]
    Bar,
    Line,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ChartPoint {
    pub label: String,
    pub value: f64,
}

/// Typed, validated view of `Artifact::content`.
#[derive(Debug, Clone, PartialEq)]
pub enum Content {
    /// `source` is a bare file name inside the store's `files/` directory.
    Image { source: String },
    Qr { data: String },
    Message { body: String, level: Level },
    Markdown { text: String },
    Table { columns: Vec<String>, rows: Vec<Vec<String>> },
    Chart { style: ChartStyle, points: Vec<ChartPoint>, unit: Option<String> },
    Code { language: Option<String>, text: String },
}

#[derive(Deserialize)]
struct ImageC {
    source: String,
}
#[derive(Deserialize)]
struct QrC {
    data: String,
}
#[derive(Deserialize)]
struct MessageC {
    body: String,
    #[serde(default)]
    level: Level,
}
#[derive(Deserialize)]
struct TextC {
    text: String,
    #[serde(default)]
    language: Option<String>,
}
#[derive(Deserialize)]
struct TableC {
    columns: Vec<String>,
    #[serde(default)]
    rows: Vec<Vec<Value>>,
}
#[derive(Deserialize)]
struct ChartC {
    #[serde(default)]
    style: ChartStyle,
    points: Vec<ChartPoint>,
    #[serde(default)]
    unit: Option<String>,
}

impl Content {
    /// The plain text the "Copy" button puts on the clipboard, for the types
    /// that are textual: the raw message/markdown/code, the QR payload, or a
    /// table as tab-separated values. Images and charts have none.
    pub fn copy_text(&self) -> Option<String> {
        match self {
            Content::Message { body, .. } => Some(body.clone()),
            Content::Markdown { text } | Content::Code { text, .. } => Some(text.clone()),
            Content::Qr { data } => Some(data.clone()),
            Content::Table { columns, rows } => {
                let mut out = columns.join("\t");
                for row in rows {
                    out.push('\n');
                    out.push_str(&row.join("\t"));
                }
                Some(out)
            }
            Content::Image { .. } | Content::Chart { .. } => None,
        }
    }

    /// Parse and validate `content` for the given artifact type.
    pub fn parse(kind: &str, content: &Value, limits: &Limits) -> Result<Self> {
        fn de<T: serde::de::DeserializeOwned>(kind: &str, v: &Value) -> Result<T> {
            serde_json::from_value(v.clone()).map_err(|e| Error::Invalid(format!("content for type '{kind}': {e}")))
        }
        Ok(match kind {
            "image" => {
                let c: ImageC = de(kind, content)?;
                if !is_store_file_name(&c.source) {
                    return Err(Error::invalid(
                        "image content.source must be a file name inside the store (publish images through the CLI)",
                    ));
                }
                Content::Image { source: c.source }
            }
            "qr" => {
                let c: QrC = de(kind, content)?;
                if c.data.is_empty() || c.data.len() > MAX_QR_BYTES {
                    return Err(Error::Invalid(format!("qr data must be 1..={MAX_QR_BYTES} bytes")));
                }
                if c.data.chars().any(char::is_control) {
                    return Err(Error::invalid("qr data must not contain control characters"));
                }
                Content::Qr { data: c.data }
            }
            "message" => {
                let c: MessageC = de(kind, content)?;
                Content::Message { body: clean_text(&c.body), level: c.level }
            }
            "markdown" => {
                let c: TextC = de(kind, content)?;
                Content::Markdown { text: clean_text(&c.text) }
            }
            "code" => {
                let c: TextC = de(kind, content)?;
                Content::Code {
                    language: c.language.map(|l| clean_line(&l)).filter(|l| !l.is_empty()),
                    text: clean_text(&c.text),
                }
            }
            "table" => {
                let c: TableC = de(kind, content)?;
                if c.columns.is_empty() || c.columns.len() > MAX_COLUMNS {
                    return Err(Error::Invalid(format!("table needs 1..={MAX_COLUMNS} columns")));
                }
                if c.rows.len() > limits.max_rows {
                    return Err(Error::TooLarge(format!("table has more than {} rows", limits.max_rows)));
                }
                let n = c.columns.len();
                let mut rows = Vec::with_capacity(c.rows.len());
                for (i, row) in c.rows.iter().enumerate() {
                    if row.len() != n {
                        return Err(Error::Invalid(format!("table row {i} has {} cells, expected {n}", row.len())));
                    }
                    rows.push(row.iter().map(cell_text).collect());
                }
                Content::Table { columns: c.columns.iter().map(|s| clean_line(s)).collect(), rows }
            }
            "chart" => {
                let c: ChartC = de(kind, content)?;
                if c.points.is_empty() || c.points.len() > limits.max_rows {
                    return Err(Error::Invalid(format!("chart needs 1..={} points", limits.max_rows)));
                }
                if c.points.iter().any(|p| !p.value.is_finite()) {
                    return Err(Error::invalid("chart values must be finite numbers"));
                }
                let points = c
                    .points
                    .into_iter()
                    .map(|p| ChartPoint { label: clean_line(&p.label), value: p.value })
                    .collect();
                Content::Chart { style: c.style, points, unit: c.unit.map(|u| clean_line(&u)) }
            }
            other => {
                return Err(Error::Invalid(format!(
                    "unknown artifact type '{other}' (supported: {})",
                    ARTIFACT_TYPES.join(", ")
                )));
            }
        })
    }
}

impl Artifact {
    /// Normalise text fields and validate the whole artifact. Returns the
    /// typed content so callers do not parse twice.
    pub fn validate(&mut self, limits: &Limits) -> Result<Content> {
        if !is_valid_id(&self.id) {
            return Err(Error::Invalid(format!(
                "id must be 1..={MAX_ID} chars of [A-Za-z0-9._-] and not start with '.'"
            )));
        }
        self.title = clean_line(&self.title);
        if self.title.is_empty() || self.title.chars().count() > MAX_TITLE {
            return Err(Error::Invalid(format!("title must be 1..={MAX_TITLE} characters")));
        }
        if let Some(d) = &self.description {
            let d = clean_line(d);
            if d.chars().count() > MAX_DESCRIPTION {
                return Err(Error::Invalid(format!("description is longer than {MAX_DESCRIPTION} characters")));
            }
            self.description = Some(d).filter(|d| !d.is_empty());
        }
        for (name, field) in [
            ("project_id", &mut self.project_id),
            ("agent_id", &mut self.agent_id),
            ("status", &mut self.status),
        ] {
            if let Some(v) = field {
                let v = clean_line(v);
                if v.chars().count() > MAX_SHORT {
                    return Err(Error::Invalid(format!("{name} is longer than {MAX_SHORT} characters")));
                }
                *field = Some(v).filter(|v| !v.is_empty());
            }
        }
        let created = time::parse_rfc3339_millis(&self.created_at)
            .ok_or_else(|| Error::invalid("created_at must be an RFC 3339 timestamp"))?;
        self.created_at = time::rfc3339_millis(created);

        if self.actions.len() > MAX_ACTIONS {
            return Err(Error::Invalid(format!("at most {MAX_ACTIONS} actions are allowed")));
        }
        for i in 0..self.actions.len() {
            let action = &mut self.actions[i];
            if !is_valid_id(&action.id) {
                return Err(Error::Invalid(format!("action {i}: invalid id")));
            }
            action.label = clean_line(&action.label);
            if action.label.is_empty() || action.label.chars().count() > MAX_SHORT {
                return Err(Error::Invalid(format!("action '{}': label must be 1..={MAX_SHORT} characters", action.id)));
            }
            match action.kind.as_str() {
                "respond" | "dismiss" => {}
                "open_url" => {
                    let url = action.url().unwrap_or_default();
                    validate_url(url).map_err(|e| Error::Invalid(format!("action '{}': {e}", action.id)))?;
                }
                other => {
                    return Err(Error::Invalid(format!(
                        "action '{}': unknown type '{other}' (supported: {})",
                        action.id,
                        ACTION_TYPES.join(", ")
                    )));
                }
            }
            let id = self.actions[i].id.clone();
            if self.actions[..i].iter().any(|a| a.id == id) {
                return Err(Error::Invalid(format!("duplicate action id '{id}'")));
            }
        }

        let size = serde_json::to_vec(&self.content)?.len();
        if size > limits.max_payload_bytes {
            return Err(Error::TooLarge(format!(
                "content is {size} bytes, limit is {}",
                limits.max_payload_bytes
            )));
        }
        Content::parse(&self.kind, &self.content, limits)
    }
}

pub fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID
        && !id.starts_with('.')
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// A bare file name that can only resolve inside the store's `files/` dir.
pub fn is_store_file_name(name: &str) -> bool {
    is_valid_id(name) && !name.contains("..")
}

/// Only plain `http`/`https` URLs without whitespace, control characters or
/// embedded credentials may be opened.
pub fn validate_url(url: &str) -> std::result::Result<(), String> {
    if url.is_empty() {
        return Err("open_url needs payload.url".into());
    }
    if url.len() > MAX_URL {
        return Err("url is too long".into());
    }
    let lower = url.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
        .ok_or("only http:// and https:// urls are allowed")?;
    if url.chars().any(|c| c.is_control() || c.is_whitespace() || matches!(c, '"' | '\'' | '`' | '<' | '>' | '\\')) {
        return Err("url contains forbidden characters".into());
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.starts_with(':') {
        return Err("url has no host".into());
    }
    if authority.contains('@') {
        return Err("urls with embedded credentials are not allowed".into());
    }
    Ok(())
}

/// Strip control characters (terminal escape injection) keeping newlines;
/// tabs become spaces.
pub fn clean_text(s: &str) -> String {
    s.chars()
        .filter_map(|c| match c {
            '\n' => Some('\n'),
            '\t' => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect()
}

/// Like [`clean_text`] but single-line and trimmed.
pub fn clean_line(s: &str) -> String {
    let flat: String = s
        .chars()
        .filter_map(|c| match c {
            '\n' | '\t' => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect();
    flat.trim().to_string()
}

fn cell_text(v: &Value) -> String {
    match v {
        Value::String(s) => clean_line(s),
        Value::Null => String::new(),
        other => clean_line(&other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn base(kind: &str, content: Value) -> Artifact {
        Artifact {
            id: "a-1".into(),
            kind: kind.into(),
            title: "Title".into(),
            description: None,
            created_at: "2026-11-24T20:00:00Z".into(),
            updated_at: None,
            project_id: None,
            agent_id: None,
            status: None,
            content,
            actions: vec![],
            revision: 0,
            response: None,
        }
    }

    #[test]
    fn spec_example_roundtrips() {
        let raw = json!({
            "id": "artifact-001", "type": "image", "title": "Expo Development QR",
            "description": "Escanea para abrir la aplicación",
            "created_at": "2026-11-24T20:00:00Z", "project_id": "my-project", "agent_id": "claude",
            "content": {"source": "expo-qr.png"},
            "actions": [{"id": "open", "label": "Abrir", "type": "open_url", "payload": {"url": "http://localhost:8081"}}]
        });
        let mut a: Artifact = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(a.validate(&Limits::default()).unwrap(), Content::Image { source: "expo-qr.png".into() });
        let back = serde_json::to_value(&a).unwrap();
        for key in ["id", "type", "title", "description", "created_at", "project_id", "agent_id", "content", "actions"] {
            assert_eq!(back[key], raw[key], "{key}");
        }
    }

    #[test]
    fn every_type_parses() {
        let l = Limits::default();
        let cases = [
            ("qr", json!({"data": "exp://192.168.1.100:8081"})),
            ("message", json!({"body": "done", "level": "success"})),
            ("markdown", json!({"text": "# hi"})),
            ("code", json!({"text": "fn main() {}", "language": "rust"})),
            ("table", json!({"columns": ["a", "b"], "rows": [[1, "x"], [null, true]]})),
            ("chart", json!({"style": "line", "points": [{"label": "a", "value": 1.5}]})),
        ];
        for (kind, content) in cases {
            base(kind, content).validate(&l).unwrap_or_else(|e| panic!("{kind}: {e}"));
        }
    }

    #[test]
    fn rejects_bad_input() {
        let l = Limits::default();
        assert!(base("html", json!({})).validate(&l).is_err());
        assert!(base("qr", json!({})).validate(&l).is_err());
        assert!(base("qr", json!({"data": "a\u{1b}[2J"})).validate(&l).is_err());
        assert!(base("table", json!({"columns": ["a"], "rows": [[1, 2]]})).validate(&l).is_err());
        assert!(base("chart", json!({"points": []})).validate(&l).is_err());
        for source in ["../secret.png", "/etc/passwd", "C:\\x.png", "a/b.png", ".hidden", ""] {
            assert!(base("image", json!({"source": source})).validate(&l).is_err(), "{source}");
        }
        let mut a = base("message", json!({"body": "x"}));
        a.id = "../../x".into();
        assert!(a.validate(&l).is_err());
        let mut a = base("message", json!({"body": "x"}));
        a.created_at = "yesterday".into();
        assert!(a.validate(&l).is_err());
    }

    #[test]
    fn strips_escape_sequences() {
        let mut a = base("message", json!({"body": "ok\u{1b}[31m\nline\u{7}"}));
        a.title = "\u{1b}]0;pwn\u{7}Title\n".into();
        let content = a.validate(&Limits::default()).unwrap();
        assert_eq!(a.title, "]0;pwnTitle");
        assert_eq!(content, Content::Message { body: "ok[31m\nline".into(), level: Level::Info });
    }

    #[test]
    fn url_rules() {
        for ok in ["http://localhost:8081", "https://example.com/a?b=1#c", "HTTPS://Example.com"] {
            assert!(validate_url(ok).is_ok(), "{ok}");
        }
        for bad in [
            "", "file:///etc/passwd", "javascript:alert(1)", "exp://1.2.3.4", "http://", "http://user:pw@host/",
            "http://a b", "http://a\"&calc", "https://x\n", "ftp://x",
        ] {
            assert!(validate_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn action_rules() {
        let l = Limits::default();
        let act = |v: Value| -> Action { serde_json::from_value(v).unwrap() };
        let mut a = base("message", json!({"body": "x"}));
        a.actions = vec![act(json!({"id": "ok", "label": "Aprobar"})), act(json!({"id": "no", "label": "Rechazar", "confirm": true}))];
        a.validate(&l).unwrap();
        assert!(!a.actions[0].needs_confirmation());
        assert!(a.actions[1].needs_confirmation());

        a.actions = vec![act(json!({"id": "ok", "label": "A"})), act(json!({"id": "ok", "label": "B"}))];
        assert!(a.validate(&l).is_err());
        a.actions = vec![act(json!({"id": "x", "label": "Run", "type": "exec", "payload": {"cmd": "rm -rf /"}}))];
        assert!(a.validate(&l).is_err());
        a.actions = vec![act(json!({"id": "x", "label": "Open", "type": "open_url", "payload": {"url": "file:///x"}}))];
        assert!(a.validate(&l).is_err());
        a.actions = vec![act(json!({"id": "x", "label": "Open", "type": "open_url", "payload": {"url": "https://x.dev"}}))];
        a.validate(&l).unwrap();
        assert!(a.actions[0].needs_confirmation());
    }

    #[test]
    fn patch_merges_content() {
        let mut a = base("message", json!({"body": "x", "level": "info"}));
        let patch: ArtifactPatch = serde_json::from_value(json!({"status": "done", "content": {"level": "success"}})).unwrap();
        patch.apply(&mut a);
        assert_eq!(a.content, json!({"body": "x", "level": "success"}));
        assert_eq!(a.status.as_deref(), Some("done"));
    }

    #[test]
    fn copy_text_only_for_textual_types() {
        let l = Limits::default();
        let text = |kind: &str, content: Value| Content::parse(kind, &content, &l).unwrap().copy_text();
        assert_eq!(text("message", json!({"body": "hola\nmundo"})).as_deref(), Some("hola\nmundo"));
        assert_eq!(text("markdown", json!({"text": "# T"})).as_deref(), Some("# T"));
        assert_eq!(text("code", json!({"text": "fn x() {}", "language": "rust"})).as_deref(), Some("fn x() {}"));
        assert_eq!(text("qr", json!({"data": "exp://x"})).as_deref(), Some("exp://x"));
        assert_eq!(text("table", json!({"columns": ["a", "b"], "rows": [[1, 2]]})).as_deref(), Some("a\tb\n1\t2"));
        assert_eq!(text("chart", json!({"points": [{"label": "a", "value": 1}]})), None);
        assert_eq!(text("image", json!({"source": "x.png"})), None);
    }

    #[test]
    fn payload_limit() {
        let limits = Limits { max_payload_bytes: 64, ..Limits::default() };
        let err = base("markdown", json!({"text": "x".repeat(200)})).validate(&limits).unwrap_err();
        assert_eq!(err.code(), "too_large");
    }
}
