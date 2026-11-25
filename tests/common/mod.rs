//! Shared helpers for integration tests.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use herdr_artifacts::{
    config::Config,
    protocol::{Action, ArtifactInput},
    storage::random_hex,
};
use serde_json::{Value, json};

/// A store in a fresh temporary directory, removed on drop.
pub struct TempHome {
    pub cfg: Config,
}

impl TempHome {
    pub fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("herdr-artifacts-test-{}", random_hex(8)));
        std::fs::create_dir_all(&dir).unwrap();
        Self { cfg: Config::with_home(dir).unwrap() }
    }

    pub fn path(&self) -> &Path {
        &self.cfg.home
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.cfg.home);
    }
}

pub fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join(name)
}

pub fn input(kind: &str, id: &str, title: &str, content: Value) -> ArtifactInput {
    ArtifactInput {
        id: Some(id.to_string()),
        kind: kind.to_string(),
        title: Some(title.to_string()),
        agent_id: Some("claude".to_string()),
        project_id: Some("demo".to_string()),
        content,
        ..Default::default()
    }
}

pub fn message(id: &str, title: &str, body: &str) -> ArtifactInput {
    input("message", id, title, json!({"body": body}))
}

pub fn action(id: &str, label: &str, kind: &str, payload: Value, confirm: bool) -> Action {
    Action { id: id.into(), label: label.into(), kind: kind.into(), payload, confirm }
}

/// Write a small checkerboard PNG and return its path.
pub fn write_png(dir: &Path, name: &str, size: u32) -> PathBuf {
    let img = image::RgbImage::from_fn(size, size, |x, y| {
        if (x / 4 + y / 4) % 2 == 0 { image::Rgb([220, 60, 60]) } else { image::Rgb([60, 60, 220]) }
    });
    let path = dir.join(name);
    img.save(&path).unwrap();
    path
}
