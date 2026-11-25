//! Wire messages.
//!
//! * Agent -> plugin: one JSON [`Envelope`] per line, answered by one JSON
//!   [`Response`] line.
//! * Plugin -> agent: [`UserEvent`]s appended to `events.jsonl`, which the
//!   `events` / `wait` CLI commands read.
//!
//! Both directions are tagged (`op` / `type`) so new variants can be added
//! without breaking old peers.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::artifact::{ArtifactInput, ArtifactPatch};
use crate::error::Error;

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    pub v: u32,
    pub token: String,
    #[serde(flatten)]
    pub request: Request,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// Create an artifact, or replace the one with the same id.
    Publish { artifact: ArtifactInput },
    /// Patch an existing artifact.
    Update { id: String, patch: ArtifactPatch },
    Remove { id: String },
    Ping,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Response {
    Ok {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        revision: Option<u32>,
    },
    Error {
        code: String,
        message: String,
    },
}

impl Response {
    pub fn ok() -> Self {
        Response::Ok { id: None, revision: None }
    }
}

impl From<Error> for Response {
    fn from(e: Error) -> Self {
        Response::Error { code: e.code().to_string(), message: e.to_string() }
    }
}

/// Something the user did in the TUI that an agent may want to react to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserEvent {
    pub event_id: String,
    /// `action_invoked` or `artifact_dismissed`.
    #[serde(rename = "type")]
    pub kind: String,
    pub artifact_id: String,
    /// Revision of the artifact the user was looking at.
    pub revision: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_type: Option<String>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub payload: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub timestamp: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn envelope_wire_format() {
        let env = Envelope {
            v: 1,
            token: "t".into(),
            request: Request::Update { id: "a".into(), patch: ArtifactPatch { status: Some("done".into()), ..Default::default() } },
        };
        let v = serde_json::to_value(&env).unwrap();
        assert_eq!(v, json!({"v": 1, "token": "t", "op": "update", "id": "a", "patch": {"status": "done"}}));
        assert_eq!(serde_json::from_value::<Envelope>(v).unwrap(), env);

        let ping: Envelope = serde_json::from_str(r#"{"v":1,"token":"t","op":"ping"}"#).unwrap();
        assert_eq!(ping.request, Request::Ping);
        assert!(serde_json::from_str::<Envelope>(r#"{"v":1,"token":"t","op":"exec"}"#).is_err());
    }

    #[test]
    fn response_wire_format() {
        let ok = Response::Ok { id: Some("a".into()), revision: Some(2) };
        assert_eq!(serde_json::to_value(&ok).unwrap(), json!({"status": "ok", "id": "a", "revision": 2}));
        let err: Response = Error::NotFound("x".into()).into();
        assert_eq!(serde_json::to_value(&err).unwrap()["code"], "not_found");
    }
}
