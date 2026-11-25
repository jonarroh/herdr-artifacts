//! Artifact core: models, contracts and validation. No I/O.

pub mod artifact;
pub mod events;

pub use artifact::{Action, Artifact, ArtifactInput, ArtifactMeta, ArtifactPatch, ChartPoint, ChartStyle, Content, Level};
pub use events::{Envelope, PROTOCOL_VERSION, Request, Response, UserEvent};
