//! Transport: how agents reach the plugin.
//!
//! [`handle`] is the single place a request is applied to the store. The
//! viewer calls it for requests arriving over IPC; the CLI calls it directly
//! when no viewer is running, so both paths validate identically.

pub mod ipc;

use crate::{
    protocol::{Request, Response},
    storage::Repository,
};

/// What a handled request changed, so the UI knows what to refresh.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    None,
    Upserted(String),
    Removed(String),
}

pub fn handle(repo: &mut Repository, request: Request) -> (Response, Change) {
    let result = match request {
        Request::Ping => return (Response::ok(), Change::None),
        Request::Publish { artifact } => repo.publish(artifact),
        Request::Update { id, patch } => repo.update(&id, patch),
        Request::Remove { id } => {
            return match repo.remove(&id) {
                Ok(()) => (Response::Ok { id: Some(id.clone()), revision: None }, Change::Removed(id)),
                Err(e) => (e.into(), Change::None),
            };
        }
    };
    match result {
        Ok(meta) => (
            Response::Ok { id: Some(meta.id.clone()), revision: Some(meta.revision) },
            Change::Upserted(meta.id),
        ),
        Err(e) => (e.into(), Change::None),
    }
}
