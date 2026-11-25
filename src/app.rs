//! The TUI: layout, drawing and the event loop.
//!
//! The loop is single-threaded and draws only when something changed (a key,
//! a resize, or an IPC request). While idle it just blocks in `poll`.

use std::{
    sync::mpsc::{self, Receiver},
    time::{Duration, SystemTime},
};

use crossterm::event::{self, Event, KeyEvent, KeyEventKind};
use ratatui::{
    Frame,
    backend::Backend,
    Terminal,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

use crate::{
    config::Config,
    error::Result,
    interaction::{
        Command, Mode, Pending, UiState,
        actions::{self, Outcome},
        clipboard::{self, Method},
    },
    protocol::{Artifact, ArtifactMeta, Content},
    renderer::{self, ImageCache, type_icon},
    storage::Repository,
    terminal::{self, Capabilities, protocol_name},
    time,
    transport::{self, Change, ipc::{self, IpcRequest}},
};

/// Two cards side by side need at least this many columns.
const TWO_COLUMN_MIN_WIDTH: u16 = 80;
/// The history panel is hidden on very short terminals.
const HISTORY_MIN_HEIGHT: u16 = 16;
const POLL_INTERVAL: Duration = Duration::from_millis(100);
/// How often a secondary viewer checks the store for outside changes.
const RESCAN_EVERY_TICKS: u32 = 10;

#[derive(Debug, Clone, PartialEq)]
pub enum IpcStatus {
    Listening,
    /// Another viewer owns the socket; this one follows the store on disk.
    Secondary,
    Failed(String),
}

/// A full artifact currently on screen.
struct Loaded {
    artifact: Artifact,
    content: std::result::Result<Content, String>,
}

pub struct App {
    repo: Repository,
    cfg: Config,
    caps: Capabilities,
    images: ImageCache,
    pub ui: UiState,
    history: ListState,
    /// Only the artifacts being displayed are kept loaded (at most two).
    loaded: Vec<Loaded>,
    toast: Option<(String, bool)>,
    ipc: IpcStatus,
    opener: fn(&str) -> std::io::Result<()>,
    copier: fn(&str) -> std::io::Result<Method>,
    dirty: bool,
    /// Real image protocols paint outside the cell buffer; wipe the screen
    /// before the next frame when what is shown changed.
    needs_clear: bool,
}

/// Run the viewer until the user quits.
pub fn run(cfg: Config) -> Result<()> {
    let repo = Repository::open(cfg.clone())?;
    let (tx, rx) = mpsc::channel();
    let ipc = match ipc::spawn_server(&cfg, tx) {
        Ok(()) => IpcStatus::Listening,
        Err(_) if ipc::is_running(&cfg) => IpcStatus::Secondary,
        Err(e) => IpcStatus::Failed(e.to_string()),
    };

    let mut terminal = ratatui::init();
    // Must happen after entering the alternate screen, before reading events.
    let (picker, caps) = terminal::detect();
    let mut app = App::new(repo, cfg, picker, caps, ipc);
    let result = app.event_loop(&mut terminal, &rx);
    ratatui::restore();
    result
}

impl App {
    pub fn new(repo: Repository, cfg: Config, picker: ratatui_image::picker::Picker, caps: Capabilities, ipc: IpcStatus) -> Self {
        // Start on the newest artifact.
        let ui = UiState { selected: repo.metas().len().saturating_sub(1), ..UiState::default() };
        Self {
            images: ImageCache::new(picker, cfg.limits.image_cache_entries),
            repo,
            cfg,
            caps,
            ui,
            history: ListState::default(),
            loaded: Vec::new(),
            toast: None,
            ipc,
            opener: actions::open_url,
            copier: clipboard::copy,
            dirty: true,
            needs_clear: false,
        }
    }

    /// Replace the URL opener (tests must not launch a browser).
    pub fn set_opener(&mut self, opener: fn(&str) -> std::io::Result<()>) {
        self.opener = opener;
    }

    /// Replace the clipboard writer (tests must not touch the real clipboard).
    pub fn set_copier(&mut self, copier: fn(&str) -> std::io::Result<Method>) {
        self.copier = copier;
    }

    /// Copy the selected artifact's text, if its type has any.
    fn copy_selected(&mut self) {
        let id = self.repo.metas().get(self.ui.selected).map(|m| m.id.clone());
        let text = id
            .and_then(|id| self.loaded.iter().find(|l| l.artifact.id == id))
            .and_then(|l| l.content.as_ref().ok())
            .and_then(Content::copy_text);
        self.toast = Some(match text {
            None => ("This artifact has no text to copy".to_string(), true),
            Some(text) => match (self.copier)(&text) {
                Ok(method) => {
                    let via = if method == Method::Osc52 { " (via terminal)" } else { "" };
                    (format!("Copied {} characters to the clipboard{via}", text.chars().count()), false)
                }
                Err(e) => (format!("Copy failed: {e}"), true),
            },
        });
    }

    pub fn toast(&self) -> Option<&str> {
        self.toast.as_ref().map(|(t, _)| t.as_str())
    }

    pub fn repo(&self) -> &Repository {
        &self.repo
    }

    fn event_loop<B: Backend>(&mut self, terminal: &mut Terminal<B>, rx: &Receiver<IpcRequest>) -> Result<()>
    where
        B::Error: std::fmt::Display,
    {
        let io_err = |e: B::Error| crate::error::Error::Io(e.to_string());
        let mut tick = 0u32;
        let mut store_stamp = self.store_stamp();
        loop {
            if self.dirty {
                if std::mem::take(&mut self.needs_clear) {
                    terminal.clear().map_err(io_err)?;
                }
                terminal.draw(|frame| self.draw(frame)).map_err(io_err)?;
                self.dirty = false;
            }
            if event::poll(POLL_INTERVAL)? {
                match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        if self.on_key(key) {
                            return Ok(());
                        }
                    }
                    Event::Resize(..) => {
                        self.images.clear();
                        self.invalidate();
                    }
                    _ => {}
                }
            }
            while let Ok(request) = rx.try_recv() {
                self.on_ipc(request);
            }
            tick = tick.wrapping_add(1);
            if self.ipc != IpcStatus::Listening && tick.is_multiple_of(RESCAN_EVERY_TICKS) {
                let stamp = self.store_stamp();
                if stamp != store_stamp {
                    store_stamp = stamp;
                    if self.repo.reload().is_ok() {
                        self.ui.clamp(self.repo.metas().len());
                        self.invalidate();
                    }
                }
            }
        }
    }

    fn store_stamp(&self) -> Option<SystemTime> {
        std::fs::metadata(self.cfg.meta_dir()).and_then(|m| m.modified()).ok()
    }

    fn invalidate(&mut self) {
        self.loaded.clear();
        self.dirty = true;
        self.needs_clear = self.caps.real_images();
    }

    /// Apply a request that arrived over IPC and answer it.
    pub fn on_ipc(&mut self, request: IpcRequest) {
        let before = self.repo.metas().len();
        let (response, change) = transport::handle(&mut self.repo, request.request);
        let _ = request.reply.send(response);
        match change {
            Change::None => return,
            Change::Upserted(id) => {
                let count = self.repo.metas().len();
                let position = self.repo.position(&id);
                // Jump to brand-new artifacts unless the user is mid-answer.
                if count > before && !self.ui.is_busy()
                    && let Some(position) = position {
                        self.ui.select(position, count);
                    }
                self.ui.clamp(count);
            }
            Change::Removed(_) => self.ui.clamp(self.repo.metas().len()),
        }
        self.invalidate();
    }

    /// Handle a key press. Returns `true` when the app should exit.
    pub fn on_key(&mut self, key: KeyEvent) -> bool {
        self.toast = None;
        self.dirty = true;
        let count = self.repo.metas().len();
        let selected = self.selected_artifact();
        let confirm: Vec<bool> =
            selected.as_ref().map(|a| a.actions.iter().map(|x| x.needs_confirmation()).collect()).unwrap_or_default();

        match self.ui.handle_key(key, count, &confirm) {
            Command::Quit => return true,
            Command::None => self.dirty = false,
            Command::Redraw => {}
            Command::SelectionChanged => self.needs_clear = self.caps.real_images(),
            Command::Copy => self.copy_selected(),
            Command::Invoke(index) => {
                if let Some(artifact) = selected {
                    let result = actions::execute(&mut self.repo, &artifact, index, &self.opener);
                    self.finish_action(&artifact, result);
                }
            }
            Command::Dismiss => {
                if let Some(artifact) = selected {
                    let result = actions::dismiss(&mut self.repo, &artifact);
                    self.finish_action(&artifact, result);
                }
            }
        }
        false
    }

    fn finish_action(&mut self, artifact: &Artifact, result: Result<Outcome>) {
        self.toast = Some(match result {
            Ok(outcome) => (outcome.describe(artifact), false),
            Err(e) => (e.to_string(), true),
        });
        self.ui.clamp(self.repo.metas().len());
        self.invalidate();
    }

    fn selected_artifact(&mut self) -> Option<Artifact> {
        let id = self.repo.metas().get(self.ui.selected)?.id.clone();
        self.ensure_loaded(&[self.ui.selected]);
        self.loaded.iter().find(|l| l.artifact.id == id).map(|l| l.artifact.clone())
    }

    /// Load the artifacts at `indices` from disk, keeping already loaded ones
    /// that are still current.
    fn ensure_loaded(&mut self, indices: &[usize]) {
        for &index in indices {
            let Some(meta) = self.repo.metas().get(index) else { continue };
            let current = self.loaded.iter().any(|l| l.artifact.id == meta.id && l.artifact.revision == meta.revision);
            if current {
                continue;
            }
            let id = meta.id.clone();
            self.loaded.retain(|l| l.artifact.id != id);
            let limits = &self.cfg.limits;
            let loaded = match self.repo.load(&id) {
                Ok(artifact) => {
                    let content = Content::parse(&artifact.kind, &artifact.content, limits).map_err(|e| e.to_string());
                    Loaded { artifact, content }
                }
                Err(_) => continue,
            };
            self.loaded.push(loaded);
        }
    }

    /// Indices of the artifacts shown on the canvas.
    fn visible(&self, canvas_width: u16) -> Vec<usize> {
        let count = self.repo.metas().len();
        if count == 0 {
            return Vec::new();
        }
        let selected = self.ui.selected.min(count - 1);
        if self.ui.expanded || count < 2 || canvas_width < TWO_COLUMN_MIN_WIDTH {
            return vec![selected];
        }
        let first = selected - selected % 2;
        if first + 1 < count { vec![first, first + 1] } else { vec![first] }
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let count = self.repo.metas().len();
        let show_history = !self.ui.expanded && count > 0 && area.height >= HISTORY_MIN_HEIGHT;
        let history_height = if show_history { (count as u16 + 2).min((area.height / 5).clamp(3, 9)) } else { 0 };
        let [header, canvas, history, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(history_height),
            Constraint::Length(1),
        ])
        .areas(area);

        self.draw_header(frame, header);
        if count == 0 {
            self.draw_empty(frame, canvas);
        } else {
            let visible = self.visible(canvas.width);
            self.ensure_loaded(&visible);
            // Free artifacts that left the screen.
            let ids: Vec<&str> = visible.iter().filter_map(|&i| self.repo.metas().get(i)).map(|m| m.id.as_str()).collect();
            self.loaded.retain(|l| ids.contains(&l.artifact.id.as_str()));

            let columns = Layout::horizontal(vec![Constraint::Fill(1); visible.len()]).split(canvas);
            for (slot, &index) in visible.iter().enumerate() {
                self.draw_card(frame, columns[slot], index);
            }
        }
        if show_history {
            self.draw_history(frame, history);
        }
        self.draw_footer(frame, footer);
        match self.ui.mode {
            Mode::Confirm(pending) => self.draw_confirm(frame, area, pending),
            Mode::Help => draw_help(frame, area),
            _ => {}
        }
    }

    fn draw_header(&self, frame: &mut Frame, area: Rect) {
        let meta = self.repo.metas().get(self.ui.selected);
        let dim = Style::new().fg(Color::DarkGray);
        let field = |meta: Option<&ArtifactMeta>, f: fn(&ArtifactMeta) -> Option<&String>| {
            meta.and_then(f).cloned().unwrap_or_else(|| "—".to_string())
        };
        let (dot, label, color) = match &self.ipc {
            IpcStatus::Listening => ("●", "listening".to_string(), Color::Green),
            IpcStatus::Secondary => ("◐", "secondary viewer".to_string(), Color::Yellow),
            IpcStatus::Failed(e) => ("○", format!("ipc off: {e}"), Color::Red),
        };
        let count = self.repo.metas().len();
        // Segments in display order, each with a priority (lower is kept
        // longer). On narrow panes the least important ones are dropped
        // whole instead of cutting the line mid-word.
        let mut segments: Vec<(u8, Vec<Span>)> = vec![
            (0, vec![Span::styled(" HERDR ARTIFACTS ", Style::new().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD))]),
            (4, vec![Span::styled("  project ", dim), Span::raw(field(meta, |m| m.project_id.as_ref()))]),
            (3, vec![Span::styled("  agent ", dim), Span::raw(field(meta, |m| m.agent_id.as_ref()))]),
            (1, vec![Span::styled(format!("  {dot} "), Style::new().fg(color)), Span::raw(label)]),
            (2, vec![Span::raw(format!("  {count} ")), Span::styled(if count == 1 { "artifact" } else { "artifacts" }, dim)]),
            (5, vec![Span::styled("  img ", dim), Span::raw(protocol_name(self.images.protocol_type()))]),
        ];
        let width = |segments: &[(u8, Vec<Span>)]| -> usize { segments.iter().flat_map(|(_, spans)| spans).map(Span::width).sum() };
        while width(&segments) > usize::from(area.width) && segments.len() > 1 {
            let drop = segments.iter().enumerate().max_by_key(|(_, (priority, _))| *priority).map(|(i, _)| i).unwrap_or(0);
            segments.remove(drop);
        }
        let line = Line::from(segments.into_iter().flat_map(|(_, spans)| spans).collect::<Vec<_>>());
        frame.render_widget(line, area);
    }

    fn draw_empty(&self, frame: &mut Frame, area: Rect) {
        let text = vec![
            Line::from("No artifacts yet."),
            Line::default(),
            Line::from("Agents publish with the CLI, for example:"),
            Line::default(),
            Line::from(Span::styled(
                "herdr-artifacts publish message --title \"Build\" --body \"Done\"",
                Style::new().fg(Color::Cyan),
            )),
            Line::from(Span::styled(
                "herdr-artifacts publish qr --data \"exp://192.168.1.100:8081\"",
                Style::new().fg(Color::Cyan),
            )),
        ];
        let block = Block::bordered().border_style(Style::new().fg(Color::DarkGray));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let top = inner.height.saturating_sub(text.len() as u16) / 2;
        let inner = Rect { y: inner.y + top, height: inner.height - top, ..inner };
        frame.render_widget(Paragraph::new(text).centered().wrap(Wrap { trim: true }), inner);
    }

    fn draw_card(&mut self, frame: &mut Frame, area: Rect, index: usize) {
        let Some(meta) = self.repo.metas().get(index).cloned() else { return };
        let selected = index == self.ui.selected;
        let accent = if selected { Color::Cyan } else { Color::DarkGray };
        let age = time::parse_rfc3339(&meta.created_at).map(|t| time::age(time::now_unix(), t)).unwrap_or_default();
        let info = [Some(meta.kind.clone()), meta.agent_id.clone(), meta.status.clone(), Some(age)]
            .into_iter()
            .flatten()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        let block = Block::bordered()
            .border_type(if selected { BorderType::Thick } else { BorderType::Plain })
            .border_style(Style::new().fg(accent))
            .title(Line::from(vec![
                Span::styled(format!(" {} ", type_icon(&meta.kind)), Style::new().fg(accent)),
                Span::styled(format!("{} ", meta.title), Style::new().add_modifier(Modifier::BOLD)),
            ]))
            .title_bottom(Line::from(Span::styled(format!(" {info} "), Style::new().fg(Color::DarkGray))).right_aligned());
        // Textual artifacts get a Copy button on the top border.
        let slot = self.loaded.iter().position(|l| l.artifact.id == meta.id);
        let copyable = slot
            .and_then(|s| self.loaded[s].content.as_ref().ok())
            .is_some_and(|c| c.copy_text().is_some());
        let block = if copyable {
            let style = if selected {
                Style::new().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(Color::DarkGray)
            };
            block.title_top(Line::from(vec![Span::styled(" [c] Copy ", style), Span::raw(" ")]).right_aligned())
        } else {
            block
        };
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let Some(slot) = slot else {
            frame.render_widget(Paragraph::new("Artifact could not be loaded.").style(Style::new().fg(Color::Red)), inner);
            return;
        };
        let has_description = self.loaded[slot].artifact.description.is_some() && inner.height > 4;
        let has_actions = !self.loaded[slot].artifact.actions.is_empty() && inner.height > 2;
        let [description, body, actions] = Layout::vertical([
            Constraint::Length(u16::from(has_description) * 2),
            Constraint::Min(1),
            Constraint::Length(u16::from(has_actions) * 2),
        ])
        .areas(inner);

        let loaded = &self.loaded[slot];
        if has_description {
            let text = loaded.artifact.description.clone().unwrap_or_default();
            frame.render_widget(Paragraph::new(text).style(Style::new().fg(Color::Gray).add_modifier(Modifier::ITALIC)), description);
        }
        let scroll = if selected { self.ui.scroll } else { 0 };
        match &loaded.content {
            Ok(content) => renderer::render(content, body, frame.buffer_mut(), scroll, &mut self.images, &self.cfg),
            Err(e) => frame.render_widget(
                Paragraph::new(format!("Invalid artifact: {e}")).style(Style::new().fg(Color::Red)).wrap(Wrap { trim: true }),
                body,
            ),
        }
        if has_actions {
            let focus = selected && matches!(self.ui.mode, Mode::Actions | Mode::Confirm(Pending::Invoke(_)));
            let mut spans = Vec::new();
            for (i, action) in loaded.artifact.actions.iter().enumerate() {
                let chosen = loaded.artifact.response.as_deref() == Some(action.id.as_str());
                let mut style = Style::new().fg(if chosen { Color::Green } else { Color::White });
                if focus && i == self.ui.action_idx {
                    style = style.add_modifier(Modifier::REVERSED | Modifier::BOLD);
                }
                let mark = if chosen { "✔ " } else { "" };
                spans.push(Span::styled(format!(" {}:{mark}{} ", i + 1, action.label), style));
                spans.push(Span::raw(" "));
            }
            let row = Rect { y: actions.y + 1, height: 1, ..actions };
            frame.render_widget(Line::from(spans), row);
        }
    }

    fn draw_history(&mut self, frame: &mut Frame, area: Rect) {
        let now = time::now_unix();
        let dim = Style::new().fg(Color::DarkGray);
        let items: Vec<ListItem> = self
            .repo
            .metas()
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let age = time::parse_rfc3339(&m.created_at).map(|t| time::age(now, t)).unwrap_or_default();
                let state = match (&m.response, &m.status, m.actions) {
                    (Some(r), _, _) => Span::styled(format!("✔ {r}"), Style::new().fg(Color::Green)),
                    (None, _, n) if n > 0 => Span::styled("awaiting reply", Style::new().fg(Color::Yellow)),
                    (None, Some(s), _) => Span::raw(s.clone()),
                    _ => Span::raw(""),
                };
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{:>3}. ", i + 1), dim),
                    Span::raw(format!("{} ", type_icon(&m.kind))),
                    Span::raw(format!("{:<28.28} ", m.title)),
                    Span::styled(format!("{:<9.9}", m.kind), dim),
                    Span::styled(format!("{:<12.12}", m.agent_id.as_deref().unwrap_or("—")), Style::new().fg(Color::Magenta)),
                    Span::styled(format!("{age:>4}  "), dim),
                    state,
                ]))
            })
            .collect();
        let title = format!(" Artifact History ({}) ", items.len());
        let list = List::new(items)
            .block(Block::bordered().border_style(dim).title(title))
            .highlight_style(Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD))
            .highlight_symbol("▶");
        self.history.select(Some(self.ui.selected));
        frame.render_stateful_widget(list, area, &mut self.history);
    }

    fn draw_footer(&self, frame: &mut Frame, area: Rect) {
        if let Some((text, is_error)) = &self.toast {
            let color = if *is_error { Color::Red } else { Color::Green };
            frame.render_widget(Line::from(Span::styled(format!(" {text}"), Style::new().fg(color))), area);
            return;
        }
        let keys: &[(&str, &str)] = match self.ui.mode {
            Mode::Browse if self.ui.expanded => {
                &[("Tab", "Next"), ("PgUp/PgDn", "Scroll"), ("c", "Copy"), ("a", "Actions"), ("Esc", "Back"), ("q", "Close")]            }
            Mode::Browse => {
                &[("Tab", "Navigate"), ("Enter", "Expand"), ("c", "Copy"), ("a/1-9", "Actions"), ("d", "Dismiss"), ("?", "Help"), ("q/Esc", "Close")]
            }
            Mode::Actions => &[("←/→", "Choose"), ("Enter", "Run"), ("Esc", "Back")],
            Mode::Confirm(_) => &[("y", "Confirm"), ("n", "Cancel")],
            Mode::Help => &[("any key", "Close help")],
        };
        let mut spans = vec![Span::raw(" ")];
        for (key, label) in keys {
            spans.push(Span::styled(format!("[{key}]"), Style::new().fg(Color::Cyan)));
            spans.push(Span::styled(format!(" {label}  "), Style::new().fg(Color::Gray)));
        }
        frame.render_widget(Line::from(spans), area);
    }

    fn draw_confirm(&self, frame: &mut Frame, area: Rect, pending: Pending) {
        let artifact = self.repo.metas().get(self.ui.selected).and_then(|m| self.loaded.iter().find(|l| l.artifact.id == m.id));
        let Some(loaded) = artifact else { return };
        let question = match pending {
            Pending::Dismiss => format!("Delete '{}'?\nThis removes it from the history.", loaded.artifact.title),
            Pending::Invoke(i) => match loaded.artifact.actions.get(i) {
                Some(a) if a.kind == "open_url" => format!("Open this URL in your browser?\n{}", a.url().unwrap_or_default()),
                Some(a) if a.kind == "dismiss" => format!("'{}' will delete this artifact. Continue?", a.label),
                Some(a) => format!("Send '{}' to {}?", a.label, loaded.artifact.agent_id.as_deref().unwrap_or("the agent")),
                None => return,
            },
        };
        let mut lines: Vec<Line> = question.lines().map(|l| Line::from(l.to_string())).collect();
        lines.push(Line::default());
        lines.push(Line::from(vec![
            Span::styled("[y]", Style::new().fg(Color::Green)),
            Span::raw(" Confirm   "),
            Span::styled("[n]", Style::new().fg(Color::Red)),
            Span::raw(" Cancel"),
        ]));
        let rect = centered(area, 60, lines.len() as u16 + 2);
        frame.render_widget(Clear, rect);
        let block = Block::bordered().border_type(BorderType::Double).border_style(Style::new().fg(Color::Yellow)).title(" Confirm ");
        frame.render_widget(Paragraph::new(lines).centered().wrap(Wrap { trim: true }).block(block), rect);
    }
}

fn draw_help(frame: &mut Frame, area: Rect) {
    let rows = [
        ("Tab / ↓ / j", "next artifact"),
        ("Shift+Tab / ↑ / k", "previous artifact"),
        ("g / G", "first / last artifact"),
        ("Enter", "expand or collapse the selected artifact"),
        ("PgUp / PgDn", "scroll long content"),
        ("c / y", "copy the artifact's text to the clipboard"),
        ("a, then ←/→ Enter", "choose and run an action"),
        ("1-9", "run an action directly"),
        ("d", "dismiss (delete) the artifact"),
        ("Esc", "go back; at the top level, close"),
        ("q / Ctrl+C", "close"),
    ];
    let lines: Vec<Line> = rows
        .iter()
        .map(|(k, v)| Line::from(vec![Span::styled(format!("{k:>20}  "), Style::new().fg(Color::Cyan)), Span::raw(*v)]))
        .collect();
    let rect = centered(area, 70, lines.len() as u16 + 2);
    frame.render_widget(Clear, rect);
    frame.render_widget(Paragraph::new(lines).block(Block::bordered().title(" Keys ")), rect);
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect { x: area.x + (area.width - width) / 2, y: area.y + (area.height - height) / 2, width, height }
}
