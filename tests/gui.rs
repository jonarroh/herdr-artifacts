//! The native window's behaviour, without opening a window.

mod common;

use common::{TempHome, action, message};
use herdr_artifacts::{
    gui::session::{Effect, Pending, Session},
    protocol::{ArtifactInput, ArtifactPatch, Request, Response},
    storage::{Repository, read_events},
};
use serde_json::{Value, json};

fn deploy() -> ArtifactInput {
    ArtifactInput {
        actions: vec![
            action("approve", "Approve", "respond", Value::Null, false),
            action("reject", "Reject", "respond", Value::Null, true),
            action("logs", "Logs", "open_url", json!({"url": "http://localhost:8081/logs"}), false),
        ],
        ..message("deploy", "Deploy?", "ready")
    }
}

fn session(home: &TempHome, inputs: Vec<ArtifactInput>) -> Session {
    let mut repo = Repository::open(home.cfg.clone()).unwrap();
    for input in inputs {
        repo.publish(input).unwrap();
    }
    let mut session = Session::new(repo, home.cfg.clone());
    session.set_opener(|_| Ok(()));
    session
}

fn publish(session: &mut Session, input: ArtifactInput) -> Effect {
    let (response, effect) = session.handle(Request::Publish { artifact: input });
    assert!(matches!(response, Response::Ok { .. }), "{response:?}");
    effect
}

fn shown(session: &Session) -> &str {
    &session.current().unwrap().artifact.id
}

fn events(home: &TempHome) -> Vec<(String, Option<String>)> {
    read_events(&home.cfg, 0).unwrap().0.into_iter().map(|e| (e.kind, e.action_id)).collect()
}

#[test]
fn starts_on_the_newest_and_jumps_to_new_artifacts() {
    let home = TempHome::new();
    let mut s = session(&home, vec![message("a", "A", "1"), message("b", "B", "2")]);
    assert_eq!(shown(&s), "b");
    assert_eq!(publish(&mut s, message("c", "C", "3")), Effect::Attention);
    assert_eq!(shown(&s), "c");
    // An update to something else must not move the selection.
    s.select(0);
    assert_eq!(publish(&mut s, message("c", "C", "again")), Effect::None);
    assert_eq!(shown(&s), "a");
    s.move_by(-1);
    assert_eq!(shown(&s), "c", "navigation wraps");
}

#[test]
fn show_raises_the_window_and_selects_the_artifact() {
    let home = TempHome::new();
    let mut s = session(&home, vec![message("a", "A", "1"), message("b", "B", "2")]);
    let (response, effect) = s.handle(Request::Show { id: Some("a".into()) });
    assert!(matches!(response, Response::Ok { .. }));
    assert_eq!(effect, Effect::Raise);
    assert_eq!(shown(&s), "a");
    assert_eq!(s.handle(Request::Show { id: Some("missing".into()) }).1, Effect::Raise);
    assert_eq!(shown(&s), "a");
}

#[test]
fn plain_actions_run_and_sensitive_ones_ask_first() {
    let home = TempHome::new();
    let mut s = session(&home, vec![deploy()]);
    assert_eq!(s.request_action(0), None);
    assert_eq!(events(&home), [("action_invoked".to_string(), Some("approve".to_string()))]);
    assert_eq!(s.current().unwrap().artifact.response.as_deref(), Some("approve"));

    let ask = s.request_action(1).expect("confirm: true asks");
    assert_eq!(ask.pending, Pending::Invoke(1));
    assert!(s.is_asking());
    assert_eq!(s.request_action(0), None, "only one dialog at a time");
    s.resolve(&ask, false);
    assert!(!s.is_asking());
    assert_eq!(events(&home).len(), 1, "answering no does nothing");

    let ask = s.request_action(2).expect("open_url always asks");
    assert!(ask.question.contains("http://localhost:8081/logs"));
    s.resolve(&ask, true);
    assert_eq!(events(&home).last().unwrap().1.as_deref(), Some("logs"));
    assert_eq!(s.request_action(8), None, "no such action");
}

#[test]
fn a_yes_never_applies_to_an_artifact_that_changed_under_the_dialog() {
    let home = TempHome::new();
    let mut s = session(&home, vec![deploy()]);
    let ask = s.request_action(1).unwrap();
    let patch = ArtifactPatch { title: Some("Delete the database?".into()), ..Default::default() };
    s.handle(Request::Update { id: "deploy".into(), patch });
    s.resolve(&ask, true);
    assert!(events(&home).is_empty());
    assert!(s.status.as_ref().unwrap().1, "the user is told why");

    // Same when it was removed while the dialog was open.
    let ask = s.request_dismiss().unwrap();
    s.handle(Request::Remove { id: "deploy".into() });
    s.resolve(&ask, true);
    assert!(events(&home).is_empty());
    assert!(s.current().is_none());
}

#[test]
fn new_artifacts_do_not_move_the_selection_while_asking() {
    let home = TempHome::new();
    let mut s = session(&home, vec![deploy()]);
    let ask = s.request_action(1).unwrap();
    publish(&mut s, message("other", "Other", "x"));
    s.handle(Request::Show { id: Some("other".into()) });
    assert_eq!(shown(&s), "deploy");
    s.resolve(&ask, true);
    assert_eq!(events(&home), [("action_invoked".to_string(), Some("reject".to_string()))]);
}

#[test]
fn dismiss_is_confirmed_and_selects_a_neighbour() {
    let home = TempHome::new();
    let mut s = session(&home, vec![message("a", "A", "1"), message("b", "B", "2")]);
    let ask = s.request_dismiss().unwrap();
    assert_eq!(s.repo().metas().len(), 2);
    s.resolve(&ask, true);
    assert_eq!(shown(&s), "a");
    assert_eq!(events(&home), [("artifact_dismissed".to_string(), None)]);
    let ask = s.request_dismiss().unwrap();
    s.resolve(&ask, true);
    assert!(s.current().is_none());
    assert_eq!(s.request_dismiss(), None);
}
