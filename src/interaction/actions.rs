//! Executing artifact actions and turning them into events for agents.
//!
//! An artifact can never run a command: the only side effects are appending
//! an event, deleting the artifact, or handing a validated http(s) URL to the
//! OS opener (without a shell).

use std::{
    io,
    process::{Command, Stdio},
};

use crate::{
    error::{Error, Result},
    protocol::{Action, Artifact, UserEvent, artifact::validate_url},
    storage::{Repository, random_hex},
    time,
};

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Responded { label: String },
    Opened { url: String },
    Dismissed,
}

impl Outcome {
    pub fn describe(&self, artifact: &Artifact) -> String {
        let agent = artifact.agent_id.as_deref().unwrap_or("agent");
        match self {
            Outcome::Responded { label } => format!("Sent '{label}' to {agent}"),
            Outcome::Opened { url } => format!("Opened {url}"),
            Outcome::Dismissed => format!("Dismissed '{}'", artifact.title),
        }
    }
}

pub fn build_event(artifact: &Artifact, kind: &str, action: Option<&Action>) -> UserEvent {
    UserEvent {
        event_id: format!("evt-{}", random_hex(6)),
        kind: kind.to_string(),
        artifact_id: artifact.id.clone(),
        revision: artifact.revision,
        action_id: action.map(|a| a.id.clone()),
        action_type: action.map(|a| a.kind.clone()),
        payload: action.map(|a| a.payload.clone()).unwrap_or_default(),
        project_id: artifact.project_id.clone(),
        agent_id: artifact.agent_id.clone(),
        timestamp: time::rfc3339(time::now_unix()),
    }
}

/// Run action `index` of `artifact`. `opener` receives URLs to open; it is a
/// parameter so tests never launch a browser.
pub fn execute(
    repo: &mut Repository,
    artifact: &Artifact,
    index: usize,
    opener: &dyn Fn(&str) -> io::Result<()>,
) -> Result<Outcome> {
    let action = artifact.actions.get(index).ok_or_else(|| Error::invalid("no such action"))?;
    match action.kind.as_str() {
        "respond" => {
            repo.append_event(&build_event(artifact, "action_invoked", Some(action)))?;
            repo.set_response(&artifact.id, &action.id)?;
            Ok(Outcome::Responded { label: action.label.clone() })
        }
        "open_url" => {
            // Validated at publish time; checked again right before use.
            let url = action.url().unwrap_or_default();
            validate_url(url).map_err(Error::Invalid)?;
            opener(url)?;
            repo.append_event(&build_event(artifact, "action_invoked", Some(action)))?;
            repo.set_response(&artifact.id, &action.id)?;
            Ok(Outcome::Opened { url: url.to_string() })
        }
        "dismiss" => {
            repo.append_event(&build_event(artifact, "artifact_dismissed", Some(action)))?;
            repo.remove(&artifact.id)?;
            Ok(Outcome::Dismissed)
        }
        other => Err(Error::Invalid(format!("unsupported action type '{other}'"))),
    }
}

/// The user deleted the artifact from the viewer.
pub fn dismiss(repo: &mut Repository, artifact: &Artifact) -> Result<Outcome> {
    repo.append_event(&build_event(artifact, "artifact_dismissed", None))?;
    repo.remove(&artifact.id)?;
    Ok(Outcome::Dismissed)
}

/// Hand a URL to the platform opener. The URL is passed as a single argv
/// entry, never through a shell.
pub fn open_url(url: &str) -> io::Result<()> {
    #[cfg(windows)]
    let mut cmd = {
        let mut c = Command::new("rundll32.exe");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    };
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = Command::new("open");
        c.arg(url);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map(drop)
}
