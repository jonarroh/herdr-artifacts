//! The real binary, end to end: publish from the CLI (as an agent would),
//! with and without a running viewer, then read results back.

mod common;

use std::{
    path::Path,
    process::{Command, Output},
    sync::mpsc,
    thread,
};

use common::{TempHome, fixture, write_png};
use herdr_artifacts::{
    interaction::actions::{self, build_event},
    storage::Repository,
    transport::{self, ipc},
};
use serde_json::{Value, json};

fn run(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_herdr-artifacts"))
        .args(args)
        .env("HERDR_ARTIFACTS_HOME", home)
        .env("HERDR_ARTIFACTS_PROJECT", "cli-test")
        .env("HERDR_ARTIFACTS_AGENT", "test-agent")
        .output()
        .expect("binary runs")
}

fn ok(home: &Path, args: &[&str]) -> Value {
    let out = run(home, args);
    assert!(out.status.success(), "{args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).unwrap();
    serde_json::from_str(stdout.trim()).unwrap_or_else(|e| panic!("{args:?} printed non-JSON ({e}): {stdout}"))
}

fn fails(home: &Path, args: &[&str], code: &str) {
    let out = run(home, args);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{args:?} should fail");
    assert!(stderr.contains(&format!("error[{code}]")), "{args:?}: expected {code}, got: {stderr}");
}

fn show(home: &Path, id: &str) -> Value {
    ok(home, &["show", id])
}

#[test]
fn spec_examples_publish_every_type_offline() {
    let home = TempHome::new();
    let h = home.path();
    let png = write_png(h, "qr.png", 64);
    let f = |name: &str| fixture(name).to_string_lossy().into_owned();

    let r = ok(h, &["publish", "image", png.to_str().unwrap(), "--title", "Expo QR", "--id", "img"]);
    assert_eq!(r, json!({"ok": true, "id": "img", "revision": 1, "delivered": "store"}));
    ok(h, &["publish", "message", "--title", "Build completed", "--body", "La compilación terminó correctamente", "--id", "msg", "--level", "success"]);
    ok(h, &["publish", "markdown", &f("report.md"), "--title", "Architecture Report", "--id", "md"]);
    ok(h, &["publish", "qr", "--data", "exp://192.168.1.100:8081", "--id", "qr"]);
    ok(h, &["publish", "table", &f("build.csv"), "--id", "tbl"]);
    ok(h, &["publish", "chart", &f("timings.json"), "--id", "chart-file"]);
    ok(h, &["publish", "chart", "--point", "a=1", "--point", "b=2.5", "--style", "line", "--id", "chart-args"]);
    ok(h, &["publish", "code", &f("snippet.rs"), "--id", "code"]);
    ok(h, &["publish", "json", &f("approval.json")]);

    let list = ok(h, &["list", "--json"]);
    assert_eq!(list.as_array().unwrap().len(), 9);

    let img = show(h, "img");
    let stored = img["content"]["source"].as_str().unwrap();
    assert!(stored.starts_with("img-") && stored.ends_with(".png"), "image was copied into the store as {stored}");
    assert!(home.cfg.files_dir().join(stored).is_file());
    assert_eq!(img["project_id"], "cli-test");
    assert_eq!(img["agent_id"], "test-agent");

    assert_eq!(show(h, "msg")["content"], json!({"body": "La compilación terminó correctamente", "level": "success"}));
    assert_eq!(show(h, "md")["title"], "Architecture Report");
    assert!(show(h, "md")["content"]["text"].as_str().unwrap().starts_with("# Architecture Report"));
    assert_eq!(show(h, "qr")["title"], "qr", "title defaults to the type");
    assert_eq!(show(h, "tbl")["content"]["rows"][0], json!(["install, cached", "2.1s", "ok"]));
    assert_eq!(show(h, "tbl")["title"], "build.csv", "title defaults to the file name");
    assert_eq!(show(h, "chart-args")["content"], json!({"points": [{"label": "a", "value": 1.0}, {"label": "b", "value": 2.5}], "style": "line"}));
    assert_eq!(show(h, "code")["content"]["language"], "rs");

    let approval = show(h, "deploy-approval");
    assert_eq!(approval["agent_id"], "claude", "values in the JSON contract win over environment defaults");
    assert_eq!(approval["actions"].as_array().unwrap().len(), 3);
    assert_eq!(approval["actions"][1]["confirm"], true);
}

#[test]
fn update_and_remove_by_id() {
    let home = TempHome::new();
    let h = home.path();
    ok(h, &["publish", "message", "--id", "build", "--title", "Build", "--body", "running", "--status", "running"]);
    let r = ok(h, &["update", "build", "--body", "done in 38s", "--level", "success", "--status", "done", "--action", "ok:Entendido"]);
    assert_eq!(r["revision"], 2);
    let a = show(h, "build");
    assert_eq!(a["content"], json!({"body": "done in 38s", "level": "success"}));
    assert_eq!(a["status"], "done");
    assert_eq!(a["actions"][0]["label"], "Entendido");
    assert_eq!(a["title"], "Build", "untouched fields are kept");

    ok(h, &["update", "build", "--clear-actions"]);
    assert!(show(h, "build").get("actions").is_none());

    ok(h, &["remove", "build"]);
    fails(h, &["show", "build"], "not_found");
    fails(h, &["update", "build", "--title", "x"], "not_found");
    fails(h, &["remove", "build"], "not_found");
}

#[test]
fn bad_input_is_rejected_with_clear_errors() {
    let home = TempHome::new();
    let h = home.path();
    fails(h, &["publish", "html", "--body", "<b>x</b>"], "invalid");
    fails(h, &["publish", "qr"], "invalid");
    fails(h, &["publish", "image", "does-not-exist.png"], "invalid");
    fails(h, &["publish", "message", "--body", "x", "--id", "../../etc/passwd"], "invalid");
    fails(h, &["publish", "message", "--body", "x", "--action", "run:Run:exec"], "invalid");
    fails(h, &["publish", "message", "--body", "x", "--action", "open:Open:open_url:file:///etc/passwd"], "invalid");
    fails(h, &["publish", "message", "--body", "x", "--action", "open:Open:open_url:javascript:alert(1)"], "invalid");

    // A text file posing as an image is refused and leaves nothing behind.
    let fake = h.join("fake.png");
    std::fs::write(&fake, "not really a png").unwrap();
    fails(h, &["publish", "image", fake.to_str().unwrap()], "invalid");
    assert_eq!(std::fs::read_dir(home.cfg.files_dir()).unwrap().count(), 0, "rejected image is not kept in the store");
    assert_eq!(ok(h, &["list", "--json"]), json!([]));
}

#[test]
fn file_size_limit_is_configurable() {
    let home = TempHome::new();
    let h = home.path();
    std::fs::write(h.join("config.json"), r#"{"max_file_bytes": 100, "max_payload_bytes": 64}"#).unwrap();
    let png = write_png(h, "pic.png", 64);
    fails(h, &["publish", "image", png.to_str().unwrap()], "too_large");
    fails(h, &["publish", "markdown", &fixture("report.md").to_string_lossy()], "too_large");
    ok(h, &["publish", "message", "--body", "small"]);
}

#[test]
fn publishing_reaches_a_running_viewer_over_ipc() {
    let home = TempHome::new();
    let h = home.path();
    // Stand-in for the viewer's event loop.
    let (tx, rx) = mpsc::channel::<ipc::IpcRequest>();
    ipc::spawn_server(&home.cfg, tx).unwrap();
    let cfg = home.cfg.clone();
    thread::spawn(move || {
        let mut repo = Repository::open(cfg).unwrap();
        for request in rx {
            let (response, _) = transport::handle(&mut repo, request.request);
            let _ = request.reply.send(response);
        }
    });

    let r = ok(h, &["publish", "qr", "--data", "exp://192.168.1.100:8081", "--title", "Expo QR", "--id", "expo"]);
    assert_eq!(r["delivered"], "viewer");
    let r = ok(h, &["update", "expo", "--data", "exp://10.0.0.2:8081"]);
    assert_eq!((r["delivered"].as_str(), r["revision"].as_u64()), (Some("viewer"), Some(2)));
    assert_eq!(show(h, "expo")["content"]["data"], "exp://10.0.0.2:8081");
    fails(h, &["publish", "qr", "--data", ""], "invalid");

    let doctor = run(h, &["doctor"]);
    assert!(String::from_utf8_lossy(&doctor.stdout).contains("viewer (ipc)    running"));
}

#[test]
fn wait_returns_the_users_response() {
    let home = TempHome::new();
    let h = home.path();
    ok(h, &["publish", "message", "--id", "ask", "--title", "Continue?", "--body", "?", "--action", "yes:Sí", "--action", "no:No"]);

    // Nobody answered yet.
    let out = run(h, &["wait", "ask", "--timeout", "1"]);
    assert_eq!(out.status.code(), Some(2), "timeout has its own exit code");

    // The user picks "Sí" in the viewer (same code path the TUI uses).
    let mut repo = Repository::open(home.cfg.clone()).unwrap();
    let artifact = repo.load("ask").unwrap();
    actions::execute(&mut repo, &artifact, 0, &|_| Ok(())).unwrap();

    let event = ok(h, &["wait", "ask", "--timeout", "5"]);
    assert_eq!(event["type"], "action_invoked");
    assert_eq!(event["artifact_id"], "ask");
    assert_eq!(event["action_id"], "yes");
    assert_eq!(event["agent_id"], "test-agent");

    // After the agent updates the artifact (a new question), old answers no longer count.
    ok(h, &["update", "ask", "--body", "And now?"]);
    assert_eq!(run(h, &["wait", "ask", "--timeout", "1"]).status.code(), Some(2));
    fails(h, &["wait", "missing", "--timeout", "1"], "not_found");

    // `events` lists everything, optionally filtered.
    repo.append_event(&build_event(&artifact, "artifact_dismissed", None)).unwrap();
    let out = run(h, &["events", "--artifact", "ask"]);
    assert_eq!(String::from_utf8_lossy(&out.stdout).lines().count(), 2);
    let out = run(h, &["events", "--artifact", "other"]);
    assert!(out.stdout.is_empty());
}
