//! Real socket / named-pipe round trips against a stand-in UI thread.

mod common;

use std::{sync::mpsc, thread};

use common::{TempHome, message};
use herdr_artifacts::{
    config::Config,
    protocol::{ArtifactPatch, Request, Response},
    storage::Repository,
    transport::{self, ipc},
};

/// Start the IPC server plus a thread that plays the UI: it owns the
/// repository and answers requests, exactly like the viewer's event loop.
fn start_viewer(cfg: &Config) {
    let (tx, rx) = mpsc::channel::<ipc::IpcRequest>();
    ipc::spawn_server(cfg, tx).expect("server binds");
    let cfg = cfg.clone();
    thread::spawn(move || {
        let mut repo = Repository::open(cfg).unwrap();
        for request in rx {
            let (response, _) = transport::handle(&mut repo, request.request);
            let _ = request.reply.send(response);
        }
    });
}

#[test]
fn publish_update_remove_over_ipc() {
    let home = TempHome::new();
    assert_eq!(ipc::send(&home.cfg, Request::Ping).unwrap(), None, "nobody listening yet");
    assert!(!ipc::is_running(&home.cfg));

    start_viewer(&home.cfg);
    assert!(ipc::is_running(&home.cfg));

    let response = ipc::send(&home.cfg, Request::Publish { artifact: message("m1", "Hello", "from an agent") }).unwrap();
    assert_eq!(response, Some(Response::Ok { id: Some("m1".into()), revision: Some(1) }));

    let patch = ArtifactPatch { status: Some("done".into()), ..Default::default() };
    let response = ipc::send(&home.cfg, Request::Update { id: "m1".into(), patch }).unwrap();
    assert_eq!(response, Some(Response::Ok { id: Some("m1".into()), revision: Some(2) }));

    // The viewer persisted it: a fresh repository sees the update.
    let stored = Repository::open(home.cfg.clone()).unwrap().load("m1").unwrap();
    assert_eq!(stored.status.as_deref(), Some("done"));

    let response = ipc::send(&home.cfg, Request::Remove { id: "m1".into() }).unwrap();
    assert!(matches!(response, Some(Response::Ok { .. })));
    let response = ipc::send(&home.cfg, Request::Remove { id: "m1".into() }).unwrap().unwrap();
    assert!(matches!(response, Response::Error { ref code, .. } if code == "not_found"), "{response:?}");
}

#[test]
fn invalid_artifacts_are_rejected_with_a_code() {
    let home = TempHome::new();
    start_viewer(&home.cfg);
    let mut bad = message("m", "T", "b");
    bad.kind = "html".into();
    let response = ipc::send(&home.cfg, Request::Publish { artifact: bad }).unwrap().unwrap();
    match response {
        Response::Error { code, message } => {
            assert_eq!(code, "invalid");
            assert!(message.contains("unknown artifact type 'html'"), "{message}");
        }
        other => panic!("expected an error, got {other:?}"),
    }
}

#[test]
fn wrong_token_is_unauthorized() {
    let home = TempHome::new();
    start_viewer(&home.cfg);
    let real = std::fs::read_to_string(home.cfg.token_path()).unwrap();
    assert!(real.len() >= 32, "token has enough entropy");

    // A client that cannot read the real token is refused.
    std::fs::write(home.cfg.token_path(), "0".repeat(64)).unwrap();
    let response = ipc::send(&home.cfg, Request::Publish { artifact: message("m", "T", "b") }).unwrap().unwrap();
    assert!(matches!(response, Response::Error { ref code, .. } if code == "unauthorized"), "{response:?}");

    std::fs::write(home.cfg.token_path(), &real).unwrap();
    assert!(ipc::is_running(&home.cfg));
    assert!(Repository::open(home.cfg.clone()).unwrap().metas().is_empty(), "nothing was stored");
}

#[test]
fn second_server_on_the_same_home_fails() {
    let home = TempHome::new();
    start_viewer(&home.cfg);
    let (tx, _rx) = mpsc::channel();
    assert!(ipc::spawn_server(&home.cfg, tx).is_err(), "the socket is already owned");
    assert!(ipc::is_running(&home.cfg), "the first viewer keeps working");
}

#[test]
fn separate_homes_are_isolated() {
    let (a, b) = (TempHome::new(), TempHome::new());
    start_viewer(&a.cfg);
    assert!(ipc::is_running(&a.cfg));
    assert!(!ipc::is_running(&b.cfg));
}

#[test]
fn many_sequential_clients() {
    let home = TempHome::new();
    start_viewer(&home.cfg);
    for i in 0..25 {
        let response = ipc::send(&home.cfg, Request::Publish { artifact: message(&format!("m{i}"), "T", "b") }).unwrap();
        assert!(matches!(response, Some(Response::Ok { .. })), "request {i}: {response:?}");
    }
    assert_eq!(Repository::open(home.cfg.clone()).unwrap().metas().len(), 25);
}
