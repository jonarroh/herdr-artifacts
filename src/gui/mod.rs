//! The native window: a real OS window that floats above the terminal, with
//! the operating system's own dialogs for confirmations.
//!
//! The window owns the IPC socket exactly like the terminal viewer does, so
//! agents publish to it without knowing which kind of viewer is running.

mod chrome;
pub mod launch;
pub mod session;
pub mod size;
mod theme;
pub mod view;

use std::{
    sync::mpsc::{self, Receiver},
    thread,
};

use eframe::egui::{self, Key, Margin, Modifiers, RichText, ViewportCommand, pos2, vec2};
use rfd::{AsyncMessageDialog, MessageButtons, MessageDialogResult, MessageLevel};

use crate::{
    config::Config,
    error::{Error, Result},
    protocol::Content,
    storage::Repository,
    transport::ipc::{self, IpcRequest},
};
use session::{Ask, Effect, Session};

const TITLE: &str = "Herdr Artifacts";

/// Show the native window until the user closes it. Returns immediately when
/// another window is already open (it is brought to the front instead).
pub fn run(cfg: Config) -> Result<()> {
    let (tx, rx) = mpsc::channel();
    if let Err(e) = ipc::spawn_server(&cfg, tx) {
        return match launch::raise(&cfg, None)? {
            launch::Raised::Window => Ok(()),
            launch::Raised::Terminal => Err(Error::invalid(launch::TERMINAL_VIEWER_OPEN)),
            launch::Raised::Nothing => Err(e),
        };
    }
    let repo = Repository::open(cfg.clone())?;
    let session = Session::new(repo, cfg);
    let initial = wanted_size(&session, false);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(TITLE)
            .with_app_id("herdr-artifacts")
            .with_inner_size(initial)
            .with_min_inner_size(size::MIN)
            // A modal has to be visible: it floats above the terminal.
            .with_always_on_top()
            .with_active(true),
        centered: true,
        ..Default::default()
    };
    eframe::run_native(
        TITLE,
        options,
        Box::new(move |cc| {
            system_fonts(&cc.egui_ctx);
            theme::apply(&cc.egui_ctx);
            let requests = wake_on_request(rx, cc.egui_ctx.clone());
            cc.egui_ctx.send_viewport_cmd(ViewportCommand::Focus);
            let fitted = Fitted { id: shown_id(&session), history: false, size: initial, checked_monitor: false };
            Ok(Box::new(Window {
                session,
                requests,
                dialog: None,
                images: Default::default(),
                title: String::new(),
                history_open: false,
                fitted,
            }))
        }),
    )
    .map_err(|e| Error::Io(format!("could not open the native window: {e}")))
}

fn shown_id(session: &Session) -> Option<String> {
    session.current().map(|c| c.artifact.id.clone())
}

/// The window size that suits what the session is showing.
fn wanted_size(session: &Session, history: bool) -> [f32; 2] {
    let Some(current) = session.current() else { return [520.0, 320.0] };
    let content = current.content.as_ref().ok();
    // Only the file header is read, to learn the image's shape.
    let image = match content {
        Some(Content::Image { source }) => image::image_dimensions(session.cfg().files_dir().join(source)).ok(),
        _ => None,
    };
    size::preferred(&current.artifact, content, image, history)
}

/// What the window was last sized for.
struct Fitted {
    id: Option<String>,
    history: bool,
    size: [f32; 2],
    /// The monitor is only known once the window exists.
    checked_monitor: bool,
}

/// The window only repaints when asked, so each request has to wake it.
fn wake_on_request(rx: Receiver<IpcRequest>, ctx: egui::Context) -> Receiver<IpcRequest> {
    let (tx, forwarded) = mpsc::channel();
    thread::spawn(move || {
        for request in rx {
            if tx.send(request).is_err() {
                return;
            }
            ctx.request_repaint();
        }
    });
    forwarded
}

/// Use the platform's UI fonts when they are where Windows keeps them.
fn system_fonts(ctx: &egui::Context) {
    let Some(windows) = std::env::var_os("WINDIR").map(std::path::PathBuf::from) else { return };
    let mut fonts = egui::FontDefinitions::default();
    for (file, family) in [("segoeui.ttf", egui::FontFamily::Proportional), ("consola.ttf", egui::FontFamily::Monospace)]
    {
        if let Ok(bytes) = std::fs::read(windows.join("Fonts").join(file)) {
            fonts.font_data.insert(file.to_string(), egui::FontData::from_owned(bytes).into());
            // First choice; the built-in fonts still provide missing symbols.
            fonts.families.entry(family).or_default().insert(0, file.to_string());
        }
    }
    // Last resort for the marks and icons the other fonts lack.
    if let Ok(bytes) = std::fs::read(windows.join("Fonts").join("seguisym.ttf")) {
        fonts.font_data.insert("seguisym.ttf".to_string(), egui::FontData::from_owned(bytes).into());
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts.families.entry(family).or_default().push("seguisym.ttf".to_string());
        }
    }
    ctx.set_fonts(fonts);
}

/// A native confirmation dialog that is currently open.
struct Dialog {
    ask: Ask,
    answer: Receiver<bool>,
}

/// Open the operating system's yes/no dialog, owned by the window. It runs off
/// the UI thread so agents can keep publishing while it is open.
fn open_dialog(ask: Ask, frame: &eframe::Frame, ctx: egui::Context) -> Dialog {
    let future = AsyncMessageDialog::new()
        .set_parent(frame)
        .set_level(MessageLevel::Warning)
        .set_title(TITLE)
        .set_description(&ask.question)
        .set_buttons(MessageButtons::YesNo)
        .show();
    let (tx, answer) = mpsc::channel();
    thread::spawn(move || {
        let yes = pollster::block_on(future) == MessageDialogResult::Yes;
        let _ = tx.send(yes);
        ctx.request_repaint();
    });
    Dialog { ask, answer }
}

/// What the user asked for during one frame.
enum Intent {
    Select(usize),
    Move(isize),
    Action(usize),
    Dismiss,
    Copy(String),
    ToggleHistory,
    Close,
}

struct Window {
    session: Session,
    requests: Receiver<IpcRequest>,
    dialog: Option<Dialog>,
    images: view::ImageSlot,
    title: String,
    history_open: bool,
    fitted: Fitted,
}

impl Window {
    /// Resize the window when it starts showing something of another shape,
    /// keeping its centre in place.
    fn fit(&mut self, ctx: &egui::Context) {
        let (monitor, outer, inner, held) = ctx.input(|input| {
            let v = input.viewport();
            let held = v.maximized.unwrap_or(false) || v.fullscreen.unwrap_or(false);
            (v.monitor_size, v.outer_rect, v.inner_rect, held)
        });
        // Do not move things under an open dialog, or undo maximizing.
        if self.dialog.is_some() || held {
            return;
        }
        let id = shown_id(&self.session);
        let wanted = wanted_size(&self.session, self.history_open);
        let first_look = !self.fitted.checked_monitor && monitor.is_some();
        let changed = id != self.fitted.id
            || self.history_open != self.fitted.history
            || size::differs(wanted, self.fitted.size);
        if !changed && !first_look {
            return;
        }
        let checked_monitor = self.fitted.checked_monitor || monitor.is_some();
        self.fitted = Fitted { id, history: self.history_open, size: wanted, checked_monitor };
        let [width, height] = size::fit_monitor(wanted, monitor.map(|m| [m.x, m.y]));
        ctx.send_viewport_cmd(ViewportCommand::InnerSize(vec2(width, height)));
        if let (Some(outer), Some(inner)) = (outer, inner) {
            let frame = outer.size() - inner.size();
            let new_outer = vec2(width, height) + frame;
            let mut position = outer.center() - new_outer / 2.0;
            // Keep it on screen when it is on the primary monitor.
            if let Some(m) = monitor
                && egui::Rect::from_min_size(pos2(0.0, 0.0), m).contains(outer.center())
            {
                position.x = position.x.clamp(0.0, (m.x - new_outer.x).max(0.0));
                position.y = position.y.clamp(0.0, (m.y - new_outer.y).max(0.0));
            }
            ctx.send_viewport_cmd(ViewportCommand::OuterPosition(pos2(position.x, position.y)));
        }
    }
}

impl eframe::App for Window {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(IpcRequest { request, reply }) = self.requests.try_recv() {
            let (response, effect) = self.session.handle(request);
            let _ = reply.send(response);
            match effect {
                Effect::None => {}
                Effect::Attention => ctx.send_viewport_cmd(ViewportCommand::RequestUserAttention(
                    egui::UserAttentionType::Informational,
                )),
                Effect::Raise => {
                    ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(ViewportCommand::Focus);
                }
            }
        }

        if let Some(dialog) = &self.dialog {
            let answer = match dialog.answer.try_recv() {
                Ok(yes) => Some(yes),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => Some(false),
            };
            if let Some(yes) = answer {
                let dialog = self.dialog.take().expect("checked above");
                self.session.resolve(&dialog.ask, yes);
            }
        }

        let title = match self.session.current() {
            Some(current) => format!("{} — {TITLE}", current.artifact.title),
            None => TITLE.to_string(),
        };
        if title != self.title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.title = title;
        }
        if self.session.repo().metas().len() < 2 {
            self.history_open = false;
        }
        self.fit(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let mut intents = Vec::new();
        if self.dialog.is_none() {
            keys(ui, &self.session, &mut intents);
        }
        let enabled = self.dialog.is_none();
        let background = ui.visuals().panel_fill;
        let bar = |top: i8, bottom: i8| {
            egui::Frame::new()
                .fill(background)
                .inner_margin(Margin { left: theme::GUTTER, right: theme::GUTTER, top, bottom })
        };
        if self.history_open {
            egui::Panel::left("history")
                .resizable(false)
                .exact_size(size::HISTORY_WIDTH)
                .frame(bar(16, 12))
                .show(ui, |ui| {
                    ui.add_enabled_ui(enabled, |ui| chrome::history(ui, &self.session, &mut intents));
                });
        }
        egui::Panel::top("header").show_separator_line(false).frame(bar(14, 12)).show(ui, |ui| {
            ui.add_enabled_ui(enabled, |ui| chrome::header(ui, &self.session, self.history_open, &mut intents));
        });
        egui::Panel::bottom("actions").show_separator_line(false).frame(bar(14, 16)).show(ui, |ui| {
            ui.add_enabled_ui(enabled, |ui| chrome::footer(ui, &self.session, &mut intents));
        });
        egui::CentralPanel::default().frame(bar(0, 0)).show(ui, |ui| {
            ui.add_enabled_ui(enabled, |ui| artifact(ui, &self.session, &mut self.images));
        });

        for intent in intents {
            let ask = match intent {
                Intent::Select(index) => {
                    self.session.select(index);
                    None
                }
                Intent::Move(delta) => {
                    self.session.move_by(delta);
                    None
                }
                Intent::Action(index) => self.session.request_action(index),
                Intent::Dismiss => self.session.request_dismiss(),
                Intent::Copy(text) => {
                    self.session.status = Some((format!("Copied {} characters", text.chars().count()), false));
                    ui.ctx().copy_text(text);
                    None
                }
                Intent::ToggleHistory => {
                    self.history_open = !self.history_open;
                    None
                }
                Intent::Close => {
                    ui.ctx().send_viewport_cmd(ViewportCommand::Close);
                    None
                }
            };
            if let Some(ask) = ask {
                self.dialog = Some(open_dialog(ask, frame, ui.ctx().clone()));
            }
            ui.ctx().request_repaint();
        }
    }
}

fn keys(ui: &mut egui::Ui, session: &Session, intents: &mut Vec<Intent>) {
    const DIGITS: [Key; 9] =
        [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9];
    ui.input_mut(|input| {
        let mut pressed = |key| input.consume_key(Modifiers::NONE, key);
        if pressed(Key::Escape) || pressed(Key::Q) {
            intents.push(Intent::Close);
        }
        if pressed(Key::J) || pressed(Key::ArrowDown) {
            intents.push(Intent::Move(1));
        }
        if pressed(Key::K) || pressed(Key::ArrowUp) {
            intents.push(Intent::Move(-1));
        }
        if pressed(Key::H) {
            intents.push(Intent::ToggleHistory);
        }
        if pressed(Key::D) || pressed(Key::Delete) {
            intents.push(Intent::Dismiss);
        }
        for (index, key) in DIGITS.into_iter().enumerate() {
            if pressed(key) {
                intents.push(Intent::Action(index));
            }
        }
    });
    // Ctrl+C with selected text belongs to egui; a bare `c` copies everything.
    if ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::C))
        && let Some(text) = chrome::copy_text(session)
    {
        intents.push(Intent::Copy(text));
    }
}

/// The artifact's content on its card.
fn artifact(ui: &mut egui::Ui, session: &Session, images: &mut view::ImageSlot) {
    theme::card(ui.visuals()).show(ui, |ui| {
        ui.set_min_size(ui.available_size());
        let Some(current) = session.current() else {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("No artifacts yet.\n\nherdr-artifacts publish message --body \"Done\"").weak());
            });
            return;
        };
        // A separate id per artifact keeps scroll positions from leaking over.
        ui.push_id(&current.artifact.id, |ui| match &current.content {
            Ok(content) => view::content(ui, content, images, session.cfg()),
            Err(e) => view::error(ui, &format!("Invalid artifact: {e}")),
        });
    });
}
