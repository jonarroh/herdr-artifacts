//! What the native window does, without any drawing: selection, incoming
//! requests, and actions that wait for the user's confirmation. Kept free of
//! egui so it can be tested without opening a window.

use std::io;

use crate::{
    config::Config,
    error::Result,
    interaction::actions::{self, Outcome},
    protocol::{Artifact, Content, Request, Response},
    storage::Repository,
    transport::{self, Change},
};

/// Something that must be confirmed before it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pending {
    Invoke(usize),
    Dismiss,
}

/// A confirmation in progress. It remembers exactly what the user was shown:
/// agents can keep publishing while the dialog is open, and a "yes" must never
/// apply to something the user did not see.
#[derive(Debug, Clone, PartialEq)]
pub struct Ask {
    pub artifact_id: String,
    pub revision: u32,
    pub pending: Pending,
    pub question: String,
}

/// The artifact on screen, loaded and parsed.
pub struct Current {
    pub artifact: Artifact,
    pub content: std::result::Result<Content, String>,
}

/// What the window has to do after a request from an agent.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    None,
    /// Something new arrived: flash the window without stealing focus.
    Attention,
    /// An agent asked for the window to come to the front.
    Raise,
}

pub struct Session {
    repo: Repository,
    cfg: Config,
    current: Option<Current>,
    selected: Option<String>,
    /// Set while a confirmation dialog is open.
    asking: bool,
    /// Result of the last user action: text and whether it is an error.
    pub status: Option<(String, bool)>,
    opener: fn(&str) -> io::Result<()>,
}

impl Session {
    pub fn new(repo: Repository, cfg: Config) -> Self {
        // Start on the newest artifact.
        let selected = repo.metas().last().map(|m| m.id.clone());
        let mut session = Self {
            repo,
            cfg,
            current: None,
            selected,
            asking: false,
            status: None,
            opener: actions::open_url,
        };
        session.load();
        session
    }

    /// Replace the URL opener (tests must not launch a browser).
    pub fn set_opener(&mut self, opener: fn(&str) -> io::Result<()>) {
        self.opener = opener;
    }

    pub fn repo(&self) -> &Repository {
        &self.repo
    }

    pub fn cfg(&self) -> &Config {
        &self.cfg
    }

    pub fn current(&self) -> Option<&Current> {
        self.current.as_ref()
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.repo.position(self.selected.as_deref()?)
    }

    pub fn is_asking(&self) -> bool {
        self.asking
    }

    pub fn select(&mut self, index: usize) {
        if let Some(meta) = self.repo.metas().get(index) {
            self.selected = Some(meta.id.clone());
            self.status = None;
            self.load();
        }
    }

    /// Move the selection by `delta`, wrapping around.
    pub fn move_by(&mut self, delta: isize) {
        let count = self.repo.metas().len() as isize;
        if count > 0 {
            let index = self.selected_index().unwrap_or(0) as isize;
            self.select((index + delta).rem_euclid(count) as usize);
        }
    }

    fn load(&mut self) {
        self.current = self.selected.as_deref().and_then(|id| self.repo.load(id).ok()).map(|artifact| {
            let content =
                Content::parse(&artifact.kind, &artifact.content, &self.cfg.limits).map_err(|e| e.to_string());
            Current { artifact, content }
        });
    }

    /// Apply a request that arrived over IPC.
    pub fn handle(&mut self, request: Request) -> (Response, Effect) {
        let before = self.repo.metas().len();
        let previous = self.selected_index().unwrap_or(0);
        let (response, change) = transport::handle(&mut self.repo, request);
        let effect = match change {
            Change::None => return (response, Effect::None),
            Change::Show(id) => {
                if !self.asking
                    && let Some(index) = id.as_deref().and_then(|id| self.repo.position(id))
                {
                    self.select(index);
                }
                return (response, Effect::Raise);
            }
            Change::Upserted(id) => {
                // Jump to brand-new artifacts unless the user is mid-answer.
                let is_new = self.repo.metas().len() > before;
                if self.selected.is_none() || (is_new && !self.asking) {
                    self.selected = Some(id);
                    Effect::Attention
                } else {
                    Effect::None
                }
            }
            Change::Removed(_) => Effect::None,
        };
        self.keep_selection(previous);
        (response, effect)
    }

    /// If the selected artifact is gone, select its neighbour; then reload.
    fn keep_selection(&mut self, previous: usize) {
        if self.selected_index().is_none() {
            let metas = self.repo.metas();
            self.selected = metas.get(previous.min(metas.len().saturating_sub(1))).map(|m| m.id.clone());
        }
        self.load();
    }

    /// The user chose action `index`. Returns the question to confirm, or
    /// runs the action right away when it needs no confirmation.
    pub fn request_action(&mut self, index: usize) -> Option<Ask> {
        if self.asking {
            return None;
        }
        let artifact = &self.current.as_ref()?.artifact;
        let action = artifact.actions.get(index)?;
        if !action.needs_confirmation() {
            let artifact = artifact.clone();
            self.run(&artifact, Pending::Invoke(index));
            return None;
        }
        let question = match action.kind.as_str() {
            "open_url" => format!("Open this URL in your browser?\n\n{}", action.url().unwrap_or_default()),
            "dismiss" => format!("'{}' will delete this artifact. Continue?", action.label),
            _ => format!("Send '{}' to {}?", action.label, artifact.agent_id.as_deref().unwrap_or("the agent")),
        };
        self.ask(Pending::Invoke(index), question)
    }

    /// The user wants to delete the selected artifact; always confirmed.
    pub fn request_dismiss(&mut self) -> Option<Ask> {
        if self.asking {
            return None;
        }
        let question = format!(
            "Delete '{}'?\nThis removes it from the history.",
            self.current.as_ref()?.artifact.title
        );
        self.ask(Pending::Dismiss, question)
    }

    fn ask(&mut self, pending: Pending, question: String) -> Option<Ask> {
        let artifact = &self.current.as_ref()?.artifact;
        let ask = Ask { artifact_id: artifact.id.clone(), revision: artifact.revision, pending, question };
        self.asking = true;
        self.status = None;
        Some(ask)
    }

    /// The confirmation dialog was answered.
    pub fn resolve(&mut self, ask: &Ask, yes: bool) {
        self.asking = false;
        if !yes {
            return;
        }
        match self.repo.load(&ask.artifact_id) {
            Ok(artifact) if artifact.revision == ask.revision => self.run(&artifact, ask.pending),
            _ => {
                self.status =
                    Some(("The artifact changed while you were deciding; nothing was done.".to_string(), true));
            }
        }
    }

    fn run(&mut self, artifact: &Artifact, pending: Pending) {
        let previous = self.selected_index().unwrap_or(0);
        let result: Result<Outcome> = match pending {
            Pending::Invoke(index) => actions::execute(&mut self.repo, artifact, index, &self.opener),
            Pending::Dismiss => actions::dismiss(&mut self.repo, artifact),
        };
        self.status = Some(match result {
            Ok(outcome) => (outcome.describe(artifact), false),
            Err(e) => (e.to_string(), true),
        });
        self.keep_selection(previous);
    }
}
