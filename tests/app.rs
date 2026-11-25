//! The viewer driven through a test backend: layout, navigation, actions,
//! responses to agents, copy and live IPC updates.

mod common;

use std::sync::{Mutex, mpsc};

use common::{TempHome, action, input, message, write_png};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use herdr_artifacts::{
    app::{App, IpcStatus},
    interaction::{Mode, clipboard::Method},
    protocol::{ArtifactPatch, Request, Response},
    storage::{Repository, ingest_file, read_events},
    terminal::Capabilities,
    transport::ipc::IpcRequest,
};
use ratatui::{Terminal, backend::TestBackend};
use ratatui_image::picker::{Picker, ProtocolType};
use serde_json::json;

static OPENED: Mutex<Vec<String>> = Mutex::new(Vec::new());
static COPIED: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn fake_opener(url: &str) -> std::io::Result<()> {
    OPENED.lock().unwrap().push(url.to_string());
    Ok(())
}
fn fake_copier(text: &str) -> std::io::Result<Method> {
    COPIED.lock().unwrap().push(text.to_string());
    Ok(Method::Native)
}

fn app(home: &TempHome, setup: impl FnOnce(&mut Repository)) -> App {
    let mut repo = Repository::open(home.cfg.clone()).unwrap();
    setup(&mut repo);
    let caps = Capabilities {
        protocol: ProtocolType::Halfblocks,
        font_size: (10, 20),
        answered: false,
        forced: false,
        truecolor: true,
        host: None,
    };
    let mut app = App::new(repo, home.cfg.clone(), Picker::halfblocks(), caps, IpcStatus::Listening);
    app.set_opener(fake_opener);
    app.set_copier(fake_copier);
    app
}

fn screen(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let buffer = terminal.backend().buffer();
    let mut out = String::new();
    for y in 0..height {
        for x in 0..width {
            out.push_str(buffer[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

fn press(app: &mut App, code: KeyCode) -> bool {
    app.on_key(KeyEvent::new(code, KeyModifiers::NONE))
}
fn ch(app: &mut App, c: char) -> bool {
    press(app, KeyCode::Char(c))
}

fn at(id: &str, title: &str, body: &str, second: u32) -> herdr_artifacts::protocol::ArtifactInput {
    let mut a = message(id, title, body);
    // A fixed past date, so artifacts published "now" always sort after these.
    a.created_at = Some(format!("2026-01-01T10:00:{second:02}Z"));
    a
}

#[test]
fn empty_state_explains_how_to_publish() {
    let home = TempHome::new();
    let mut app = app(&home, |_| {});
    let text = screen(&mut app, 100, 24);
    assert!(text.contains("HERDR ARTIFACTS"));
    assert!(text.contains("No artifacts yet."));
    assert!(text.contains("herdr-artifacts publish qr"));
    assert!(text.contains("0 artifacts"));
}

#[test]
fn dashboard_layout_two_columns_history_footer() {
    let home = TempHome::new();
    let mut app = app(&home, |repo| {
        let mut qr = input("qr", "expo", "Expo QR", json!({"data": "exp://192.168.1.100:8081"}));
        qr.created_at = Some("2026-01-01T09:00:00Z".into());
        repo.publish(qr).unwrap();
        repo.publish(at("build", "Build Status", "Build completed\n12 files generated", 1)).unwrap();
    });
    let text = screen(&mut app, 120, 34);
    let line = |needle: &str| text.lines().position(|l| l.contains(needle)).unwrap_or_else(|| panic!("missing {needle:?} in\n{text}"));

    // Header: project, agent, connection, count, image mode.
    let header = text.lines().next().unwrap();
    for part in ["HERDR ARTIFACTS", "project demo", "agent claude", "● listening", "2 artifacts", "img halfblocks"] {
        assert!(header.contains(part), "header lacks {part:?}: {header}");
    }
    // Both cards share a row (two columns), QR is actually drawn.
    assert_eq!(line("Expo QR"), line("Build Status"));
    assert!(text.contains("█▀▀▀▀▀█") || text.contains("██████████████"), "QR finder pattern is drawn\n{text}");
    assert!(text.contains("Build completed") && text.contains("12 files generated"));
    // History below the canvas, chronological, then the footer.
    let history = line("Artifact History (2)");
    assert!(history > line("Build completed"));
    assert!(text.contains("  1. ▦ Expo QR") && text.contains("  2. ● Build Status"), "{text}");
    assert!(text.lines().last().unwrap().contains("[Tab] Navigate"));
}

#[test]
fn narrow_terminals_show_one_card_and_tiny_ones_hide_history() {
    let home = TempHome::new();
    let mut app = app(&home, |repo| {
        repo.publish(at("a", "Alpha", "first", 0)).unwrap();
        repo.publish(at("b", "Beta", "second", 1)).unwrap();
    });
    let narrow = screen(&mut app, 60, 24);
    assert!(narrow.contains("second") && !narrow.contains("first"), "only the selected (newest) card\n{narrow}");
    assert!(narrow.contains("Artifact History"));

    let tiny = screen(&mut app, 60, 10);
    assert!(tiny.contains("Beta") && !tiny.contains("Artifact History"), "{tiny}");

    // The header drops whole fields by priority instead of cutting words.
    let header = |app: &mut App, width| screen(app, width, 20).lines().next().unwrap().trim_end().to_string();
    assert_eq!(header(&mut app, 60), " HERDR ARTIFACTS   agent claude  ● listening  2 artifacts");
    assert_eq!(header(&mut app, 44), " HERDR ARTIFACTS   ● listening  2 artifacts");
    assert_eq!(header(&mut app, 20), " HERDR ARTIFACTS");
}

#[test]
fn navigation_and_expanded_view() {
    let home = TempHome::new();
    let mut app = app(&home, |repo| {
        for (i, name) in ["Alpha", "Beta", "Gamma"].iter().enumerate() {
            repo.publish(at(&format!("m{i}"), name, &format!("body of {name}"), i as u32)).unwrap();
        }
    });
    assert_eq!(app.ui.selected, 2, "starts on the newest artifact");
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.ui.selected, 0, "wraps around");
    press(&mut app, KeyCode::Down);
    assert_eq!(app.ui.selected, 1);

    press(&mut app, KeyCode::Enter);
    let text = screen(&mut app, 120, 30);
    assert!(text.contains("body of Beta"));
    assert!(!text.contains("body of Alpha") && !text.contains("Artifact History"), "expanded view shows one artifact only");
    assert!(text.contains("[Esc] Back"));

    press(&mut app, KeyCode::Esc);
    let text = screen(&mut app, 120, 30);
    assert!(text.contains("Artifact History (3)"));
    assert!(text.contains("▶  2. ● Beta"), "history highlights the selection\n{text}");
    assert!(ch(&mut app, 'q'), "q closes the viewer");
}

#[test]
fn approve_reject_sends_an_event_to_the_agent() {
    let home = TempHome::new();
    let mut app = app(&home, |repo| {
        let mut a = message("deploy", "Deploy to production?", "Version 1.4.2 is ready.");
        a.actions = vec![
            action("approve", "Aprobar", "respond", json!({"env": "prod"}), false),
            action("reject", "Rechazar", "respond", json!(null), true),
        ];
        repo.publish(a).unwrap();
    });
    let text = screen(&mut app, 100, 24);
    assert!(text.contains("1:Aprobar") && text.contains("2:Rechazar"));
    assert!(text.contains("awaiting reply"), "history shows the pending question");

    // "Rechazar" was published with confirm=true: it asks first.
    ch(&mut app, '2');
    assert_eq!(app.ui.mode, Mode::Confirm(herdr_artifacts::interaction::Pending::Invoke(1)));
    assert!(screen(&mut app, 100, 24).contains("Send 'Rechazar' to claude?"));
    ch(&mut app, 'n');
    assert!(read_events(&home.cfg, 0).unwrap().0.is_empty(), "cancelling sends nothing");

    // "Aprobar" through the action bar.
    ch(&mut app, 'a');
    assert_eq!(app.ui.mode, Mode::Actions);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.toast(), Some("Sent 'Aprobar' to claude"));

    let (events, _) = read_events(&home.cfg, 0).unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.kind, "action_invoked");
    assert_eq!(event.artifact_id, "deploy");
    assert_eq!(event.action_id.as_deref(), Some("approve"));
    assert_eq!(event.payload, json!({"env": "prod"}));
    assert_eq!(event.revision, 1);
    assert_eq!(event.agent_id.as_deref(), Some("claude"));

    // The choice is remembered and shown.
    assert_eq!(app.repo().load("deploy").unwrap().response.as_deref(), Some("approve"));
    let text = screen(&mut app, 100, 24);
    assert!(text.contains("1:✔ Aprobar") && text.contains("✔ approve"), "{text}");
}

#[test]
fn open_url_always_confirms_and_never_uses_unvalidated_urls() {
    let home = TempHome::new();
    let mut app = app(&home, |repo| {
        let mut a = message("srv", "Dev server", "Running");
        a.actions = vec![action("open", "Abrir", "open_url", json!({"url": "http://localhost:8081"}), false)];
        repo.publish(a).unwrap();
    });
    OPENED.lock().unwrap().clear();
    ch(&mut app, '1');
    let text = screen(&mut app, 100, 24);
    assert!(text.contains("Open this URL in your browser?") && text.contains("http://localhost:8081"));
    assert!(OPENED.lock().unwrap().is_empty(), "nothing opens before the user confirms");

    ch(&mut app, 'y');
    assert_eq!(*OPENED.lock().unwrap(), ["http://localhost:8081"]);
    assert_eq!(app.toast(), Some("Opened http://localhost:8081"));
    assert_eq!(read_events(&home.cfg, 0).unwrap().0[0].action_type.as_deref(), Some("open_url"));
}

#[test]
fn dismiss_removes_the_artifact_after_confirmation() {
    let home = TempHome::new();
    let mut app = app(&home, |repo| {
        repo.publish(at("a", "Alpha", "x", 0)).unwrap();
        repo.publish(at("b", "Beta", "y", 1)).unwrap();
    });
    ch(&mut app, 'd');
    assert!(screen(&mut app, 100, 24).contains("Delete 'Beta'?"));
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.repo().metas().len(), 2);

    ch(&mut app, 'd');
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.repo().metas().len(), 1);
    assert_eq!(app.ui.selected, 0);
    assert_eq!(app.toast(), Some("Dismissed 'Beta'"));
    let (events, _) = read_events(&home.cfg, 0).unwrap();
    assert_eq!((events[0].kind.as_str(), events[0].artifact_id.as_str()), ("artifact_dismissed", "b"));
    assert!(Repository::open(home.cfg.clone()).unwrap().load("b").is_err(), "gone from disk too");
}

#[test]
fn copy_button_on_text_artifacts_only() {
    let home = TempHome::new();
    let png_dir = home.path().to_path_buf();
    let mut app = app(&home, |repo| {
        let png = write_png(&png_dir, "pic.png", 32);
        let name = ingest_file(repo.config(), &png).unwrap();
        let mut img = input("image", "pic", "Picture", json!({"source": name}));
        img.created_at = Some("2026-11-24T10:00:00Z".into());
        repo.publish(img).unwrap();
        let mut code = input("code", "snippet", "Snippet", json!({"text": "fn main() {\n    println!(\"hola ñ\");\n}", "language": "rust"}));
        code.created_at = Some("2026-11-24T10:00:01Z".into());
        repo.publish(code).unwrap();
    });
    COPIED.lock().unwrap().clear();

    // Code artifact (selected): the button is visible and copies the raw text.
    press(&mut app, KeyCode::Enter);
    let source = "fn main() {\n    println!(\"hola ñ\");\n}";
    let text = screen(&mut app, 100, 24);
    assert!(text.lines().nth(1).unwrap().contains("[c] Copy"), "button sits on the card's top border\n{text}");
    assert!(text.lines().last().unwrap().contains("[c] Copy"), "footer advertises it too");
    ch(&mut app, 'c');
    assert_eq!(*COPIED.lock().unwrap(), [source], "the raw text, without line numbers or styling");
    let toast = format!("Copied {} characters to the clipboard", source.chars().count());
    assert_eq!(app.toast(), Some(toast.as_str()));
    assert!(screen(&mut app, 100, 24).contains(&toast), "toast is shown in the footer");

    // Image artifact: no button on the card, and the key reports why.
    press(&mut app, KeyCode::Tab);
    let text = screen(&mut app, 100, 24);
    let card: String = text.lines().take_while(|l| !l.contains("[Tab]")).collect();
    assert!(text.contains("Picture") && !card.contains("[c] Copy"), "{text}");
    ch(&mut app, 'c');
    assert_eq!(app.toast(), Some("This artifact has no text to copy"));
    assert_eq!(COPIED.lock().unwrap().len(), 1, "nothing else was copied");
}

#[test]
fn image_artifact_renders_with_unicode_fallback() {
    let home = TempHome::new();
    let png_dir = home.path().to_path_buf();
    let mut app = app(&home, |repo| {
        let png = write_png(&png_dir, "pic.png", 48);
        let name = ingest_file(repo.config(), &png).unwrap();
        repo.publish(input("image", "pic", "Picture", json!({"source": name}))).unwrap();
    });
    let text = screen(&mut app, 80, 24);
    assert!(text.contains('▀') || text.contains('▄'), "half-block image is drawn\n{text}");
    assert!(!text.contains("Cannot display image"));
}

#[test]
fn every_type_renders_without_errors() {
    let home = TempHome::new();
    let mut app = app(&home, |repo| {
        let items = [
            ("markdown", json!({"text": "# Report\n\n- item **one**\n- item two"}), "Report"),
            ("table", json!({"columns": ["File", "Size"], "rows": [["main.rs", 120], ["app.rs", 480]]}), "main.rs"),
            ("chart", json!({"style": "bar", "points": [{"label": "build", "value": 12}, {"label": "test", "value": 30}]}), "build"),
            ("chart", json!({"style": "line", "points": [{"label": "a", "value": 1}, {"label": "b", "value": 4}]}), "min 1"),
            ("code", json!({"text": "let x = 1;", "language": "rust"}), "1 │ let x = 1;"),
            ("qr", json!({"data": "https://herdr.dev"}), "https://herdr.dev"),
        ];
        for (i, (kind, content, _)) in items.iter().enumerate() {
            let mut a = input(kind, &format!("t{i}"), &format!("Item {i}"), content.clone());
            a.created_at = Some(format!("2026-11-24T10:00:0{i}Z"));
            repo.publish(a).unwrap();
        }
    });
    let expected = ["Report", "main.rs", "build", "min 1", "1 │ let x = 1;", "https://herdr.dev"];
    press(&mut app, KeyCode::Enter); // expanded: one artifact fills the canvas
    for (i, needle) in expected.iter().enumerate() {
        app.ui.select(i, expected.len());
        let text = screen(&mut app, 100, 40);
        assert!(text.contains(needle), "artifact {i} should show {needle:?}\n{text}");
        assert!(!text.contains("Invalid artifact") && !text.contains("could not be loaded"));
    }
}

#[test]
fn ipc_requests_update_the_view_live() {
    let home = TempHome::new();
    let mut app = app(&home, |repo| {
        repo.publish(at("old", "Old", "existing", 0)).unwrap();
    });
    let send = |app: &mut App, request: Request| -> Response {
        let (reply, rx) = mpsc::channel();
        app.on_ipc(IpcRequest { request, reply });
        rx.try_recv().expect("the viewer answers immediately")
    };

    // A new artifact arrives: it is stored, answered and selected.
    let response = send(&mut app, Request::Publish { artifact: message("new", "Fresh", "just published") });
    assert_eq!(response, Response::Ok { id: Some("new".into()), revision: Some(1) });
    assert_eq!(app.ui.selected, 1);
    assert!(screen(&mut app, 80, 24).contains("just published"));

    // Updating it changes what is on screen without moving the selection.
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.ui.selected, 0);
    let patch: ArtifactPatch = serde_json::from_value(json!({"title": "Fresher", "content": {"body": "updated text"}})).unwrap();
    let response = send(&mut app, Request::Update { id: "new".into(), patch });
    assert_eq!(response, Response::Ok { id: Some("new".into()), revision: Some(2) });
    assert_eq!(app.ui.selected, 0);
    press(&mut app, KeyCode::Tab);
    let text = screen(&mut app, 80, 24);
    assert!(text.contains("Fresher") && text.contains("updated text") && !text.contains("just published"), "{text}");

    // While the user is answering a prompt, new arrivals do not steal focus.
    ch(&mut app, 'd');
    send(&mut app, Request::Publish { artifact: message("third", "Third", "later") });
    assert_eq!(app.ui.selected, 1, "selection stays put during a confirmation");
    press(&mut app, KeyCode::Esc);

    // Invalid requests are answered with an error and change nothing.
    let mut bad = message("bad", "Bad", "x");
    bad.kind = "html".into();
    assert!(matches!(send(&mut app, Request::Publish { artifact: bad }), Response::Error { .. }));
    assert_eq!(app.repo().metas().len(), 3);

    send(&mut app, Request::Remove { id: "third".into() });
    send(&mut app, Request::Remove { id: "new".into() });
    assert_eq!(app.ui.selected, 0, "selection is clamped after removals");
    assert!(screen(&mut app, 80, 24).contains("existing"));
}

#[test]
fn only_visible_artifacts_are_drawn_from_disk() {
    let home = TempHome::new();
    let mut app = app(&home, |repo| {
        for i in 0..40 {
            repo.publish(at(&format!("m{i:02}"), &format!("Title {i:02}"), &format!("unique-body-{i:02}"), i)).unwrap();
        }
    });
    let text = screen(&mut app, 120, 30);
    let shown = (0..40).filter(|i| text.contains(&format!("unique-body-{i:02}"))).count();
    assert_eq!(shown, 2, "two cards on screen, the other 38 bodies are not loaded or drawn");
    assert!(text.contains("Artifact History (40)"));
}
