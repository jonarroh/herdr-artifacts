//! Persistence: artifact metadata, history, ingested files and the event log.

pub mod repository;

pub use repository::{Repository, discard_file, ingest_file, random_hex, read_events};
