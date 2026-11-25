//! Persistence, updates, limits and file safety.

mod common;

use common::{TempHome, input, message, write_png};
use herdr_artifacts::{
    config::Limits,
    interaction::actions::build_event,
    protocol::ArtifactPatch,
    storage::{Repository, ingest_file, read_events},
};
use serde_json::json;

#[test]
fn artifacts_survive_reopening() {
    let home = TempHome::new();
    {
        let mut repo = Repository::open(home.cfg.clone()).unwrap();
        repo.publish(message("m1", "First", "hello")).unwrap();
        repo.publish(input("qr", "q1", "Expo QR", json!({"data": "exp://192.168.1.100:8081"}))).unwrap();
    }
    let repo = Repository::open(home.cfg.clone()).unwrap();
    let ids: Vec<&str> = repo.metas().iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&"m1") && ids.contains(&"q1"));
    let qr = repo.load("q1").unwrap();
    assert_eq!(qr.title, "Expo QR");
    assert_eq!(qr.content, json!({"data": "exp://192.168.1.100:8081"}));
    assert_eq!(qr.revision, 1);
    assert_eq!(qr.agent_id.as_deref(), Some("claude"));
}

#[test]
fn history_is_chronological() {
    let home = TempHome::new();
    let mut repo = Repository::open(home.cfg.clone()).unwrap();
    for (id, at) in [("late", "2026-11-24T20:00:00Z"), ("early", "2026-11-24T08:00:00Z"), ("mid", "2026-11-24T10:00:00-05:00")] {
        let mut a = message(id, id, "x");
        a.created_at = Some(at.to_string());
        repo.publish(a).unwrap();
    }
    let order: Vec<&str> = repo.metas().iter().map(|m| m.id.as_str()).collect();
    assert_eq!(order, ["early", "mid", "late"], "sorted by time, offsets normalised to UTC");
    let reopened = Repository::open(home.cfg.clone()).unwrap();
    assert_eq!(reopened.metas().iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), order);
}

#[test]
fn same_second_publishes_keep_arrival_order() {
    let home = TempHome::new();
    let mut repo = Repository::open(home.cfg.clone()).unwrap();
    // Ids chosen so that sorting by id would give the opposite order.
    for id in ["zeta", "mid", "alpha"] {
        repo.publish(message(id, id, "x")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
    }
    let order = |repo: &Repository| repo.metas().iter().map(|m| m.id.clone()).collect::<Vec<_>>();
    assert_eq!(order(&repo), ["zeta", "mid", "alpha"]);
    assert_eq!(order(&Repository::open(home.cfg.clone()).unwrap()), ["zeta", "mid", "alpha"], "and after reopening");

    // Whole-second and fractional timestamps compare by value, not as text.
    let mut whole = message("whole", "w", "x");
    whole.created_at = Some("2026-01-01T10:00:00Z".into());
    let mut frac = message("frac", "f", "x");
    frac.created_at = Some("2026-01-01T10:00:00.500Z".into());
    repo.publish(frac).unwrap();
    repo.publish(whole).unwrap();
    assert_eq!(&order(&repo)[..2], ["whole", "frac"]);
}

#[test]
fn update_patches_and_bumps_revision() {
    let home = TempHome::new();
    let mut repo = Repository::open(home.cfg.clone()).unwrap();
    repo.publish(input("message", "build", "Build", json!({"body": "running", "level": "info"}))).unwrap();
    repo.set_response("build", "ok").unwrap();

    let patch: ArtifactPatch =
        serde_json::from_value(json!({"status": "done", "content": {"body": "finished", "level": "success"}})).unwrap();
    let meta = repo.update("build", patch).unwrap();
    assert_eq!(meta.revision, 2);
    assert_eq!(meta.status.as_deref(), Some("done"));
    assert_eq!(meta.response, None, "a new revision is a new question");

    let stored = repo.load("build").unwrap();
    assert_eq!(stored.content, json!({"body": "finished", "level": "success"}));
    assert_eq!(stored.title, "Build");
    assert!(stored.updated_at.is_some());

    assert_eq!(repo.update("missing", ArtifactPatch { title: Some("x".into()), ..Default::default() }).unwrap_err().code(), "not_found");
    assert_eq!(repo.update("build", ArtifactPatch::default()).unwrap_err().code(), "invalid");
    // An update that would make the artifact invalid is rejected and changes nothing.
    let bad: ArtifactPatch = serde_json::from_value(json!({"content": {"level": "catastrophic"}})).unwrap();
    assert_eq!(repo.update("build", bad).unwrap_err().code(), "invalid");
    assert_eq!(repo.load("build").unwrap().revision, 2);
}

#[test]
fn republishing_an_id_replaces_it() {
    let home = TempHome::new();
    let mut repo = Repository::open(home.cfg.clone()).unwrap();
    let first = repo.publish(message("same", "v1", "one")).unwrap();
    let second = repo.publish(message("same", "v2", "two")).unwrap();
    assert_eq!(repo.metas().len(), 1);
    assert_eq!(second.revision, 2);
    assert_eq!(second.created_at, first.created_at);
    assert_eq!(repo.load("same").unwrap().title, "v2");
}

#[test]
fn generated_ids_are_unique() {
    let home = TempHome::new();
    let mut repo = Repository::open(home.cfg.clone()).unwrap();
    let mut a = message("x", "A", "a");
    a.id = None;
    let one = repo.publish(a.clone()).unwrap();
    let two = repo.publish(a).unwrap();
    assert_ne!(one.id, two.id);
    assert!(one.id.starts_with("art-"));
}

#[test]
fn remove_deletes_everything() {
    let home = TempHome::new();
    let mut repo = Repository::open(home.cfg.clone()).unwrap();
    let png = write_png(home.path(), "src.png", 32);
    let name = ingest_file(&home.cfg, &png).unwrap();
    repo.publish(input("image", "img", "Image", json!({"source": name}))).unwrap();
    assert!(home.cfg.files_dir().join(&name).is_file());

    repo.remove("img").unwrap();
    assert!(repo.metas().is_empty());
    assert!(!home.cfg.files_dir().join(&name).exists(), "image file is removed with its artifact");
    assert_eq!(repo.load("img").unwrap_err().code(), "not_found");
    assert_eq!(repo.remove("img").unwrap_err().code(), "not_found");
    assert!(png.is_file(), "the user's original file is never touched");
}

#[test]
fn history_limit_prunes_oldest() {
    let home = TempHome::new();
    let mut cfg = home.cfg.clone();
    cfg.limits = Limits { history_limit: 3, ..Limits::default() };
    let mut repo = Repository::open(cfg.clone()).unwrap();
    for i in 0..5 {
        let mut a = message(&format!("m{i}"), "t", "b");
        a.created_at = Some(format!("2026-11-24T10:00:0{i}Z"));
        repo.publish(a).unwrap();
    }
    let ids: Vec<&str> = repo.metas().iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["m2", "m3", "m4"]);
    assert_eq!(std::fs::read_dir(cfg.data_dir()).unwrap().count(), 3);
    assert_eq!(std::fs::read_dir(cfg.meta_dir()).unwrap().count(), 3);
}

#[test]
fn image_sources_cannot_escape_the_store() {
    let home = TempHome::new();
    let mut repo = Repository::open(home.cfg.clone()).unwrap();
    let outside = write_png(home.path(), "outside.png", 16);

    for source in ["../outside.png", "..\\outside.png", outside.to_str().unwrap(), "files/../../outside.png"] {
        let err = repo.publish(input("image", "evil", "Evil", json!({"source": source}))).unwrap_err();
        assert_eq!(err.code(), "invalid", "{source}");
    }
    // A valid name that is not in the store is rejected too.
    assert_eq!(repo.publish(input("image", "ghost", "Ghost", json!({"source": "ghost.png"}))).unwrap_err().code(), "invalid");
    assert!(repo.metas().is_empty());
    assert!(repo.load("../meta/x").is_err());
}

#[test]
fn image_limits_are_enforced() {
    let home = TempHome::new();
    let mut cfg = home.cfg.clone();
    let png = write_png(home.path(), "big.png", 64);

    cfg.limits = Limits { max_file_bytes: 50, ..Limits::default() };
    assert_eq!(ingest_file(&cfg, &png).unwrap_err().code(), "too_large");

    cfg.limits = Limits { max_image_dimension: 32, ..Limits::default() };
    let name = ingest_file(&cfg, &png).unwrap();
    let mut repo = Repository::open(cfg.clone()).unwrap();
    assert_eq!(repo.publish(input("image", "i", "I", json!({"source": name}))).unwrap_err().code(), "too_large");

    // Something that is not an image at all.
    let fake = home.path().join("fake.png");
    std::fs::write(&fake, b"#!/bin/sh\necho pwned\n").unwrap();
    let name = ingest_file(&home.cfg, &fake).unwrap();
    let mut repo = Repository::open(home.cfg.clone()).unwrap();
    assert_eq!(repo.publish(input("image", "f", "F", json!({"source": name}))).unwrap_err().code(), "invalid");
    assert_eq!(ingest_file(&home.cfg, home.path()).unwrap_err().code(), "invalid", "directories are not files");
}

#[test]
fn events_are_appended_and_tailed() {
    let home = TempHome::new();
    let mut repo = Repository::open(home.cfg.clone()).unwrap();
    repo.publish(message("m", "M", "b")).unwrap();
    let artifact = repo.load("m").unwrap();

    assert_eq!(read_events(&home.cfg, 0).unwrap(), (vec![], 0), "no file yet");
    repo.append_event(&build_event(&artifact, "action_invoked", None)).unwrap();
    let (first, offset) = read_events(&home.cfg, 0).unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].artifact_id, "m");
    assert_eq!(first[0].agent_id.as_deref(), Some("claude"));

    assert_eq!(read_events(&home.cfg, offset).unwrap().0.len(), 0, "nothing new");
    repo.append_event(&build_event(&artifact, "artifact_dismissed", None)).unwrap();
    let (second, _) = read_events(&home.cfg, offset).unwrap();
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].kind, "artifact_dismissed");
    assert_ne!(first[0].event_id, second[0].event_id);
}

#[test]
fn corrupt_files_do_not_break_the_store() {
    let home = TempHome::new();
    let mut repo = Repository::open(home.cfg.clone()).unwrap();
    repo.publish(message("good", "Good", "b")).unwrap();
    std::fs::write(home.cfg.meta_dir().join("junk.json"), b"{not json").unwrap();
    std::fs::write(home.cfg.meta_dir().join("notes.txt"), b"ignored").unwrap();
    let reopened = Repository::open(home.cfg.clone()).unwrap();
    assert_eq!(reopened.metas().len(), 1);
}
