//! Navigation and selection as a pure state machine: keys go in, a
//! [`Command`] comes out. No I/O here, so every interaction is unit-testable.

pub mod actions;
pub mod clipboard;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Something that must be confirmed before it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pending {
    Invoke(usize),
    Dismiss,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Moving between artifacts.
    Browse,
    /// Choosing one of the selected artifact's actions.
    Actions,
    /// Waiting for a yes/no answer.
    Confirm(Pending),
    Help,
}

/// What the app must do in response to a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Nothing changed.
    None,
    /// Only the view changed.
    Redraw,
    /// A different artifact is selected.
    SelectionChanged,
    Invoke(usize),
    Dismiss,
    /// Copy the selected artifact's text to the clipboard.
    Copy,
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiState {
    pub selected: usize,
    /// The selected artifact fills the canvas.
    pub expanded: bool,
    pub mode: Mode,
    pub action_idx: usize,
    pub scroll: u16,
}

impl Default for UiState {
    fn default() -> Self {
        Self { selected: 0, expanded: false, mode: Mode::Browse, action_idx: 0, scroll: 0 }
    }
}

const SCROLL_STEP: u16 = 5;

impl UiState {
    /// `count` is the number of artifacts; `confirm` has one flag per action
    /// of the selected artifact, telling whether it needs confirmation.
    pub fn handle_key(&mut self, key: KeyEvent, count: usize, confirm: &[bool]) -> Command {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Command::Quit;
        }
        match self.mode {
            Mode::Help => {
                self.mode = Mode::Browse;
                Command::Redraw
            }
            Mode::Confirm(pending) => match key.code {
                KeyCode::Char('y' | 'Y') | KeyCode::Enter => {
                    self.mode = Mode::Browse;
                    match pending {
                        Pending::Invoke(i) => Command::Invoke(i),
                        Pending::Dismiss => Command::Dismiss,
                    }
                }
                KeyCode::Char('n' | 'N' | 'q') | KeyCode::Esc => {
                    self.mode = Mode::Browse;
                    Command::Redraw
                }
                _ => Command::None,
            },
            Mode::Actions => self.actions_key(key, confirm),
            Mode::Browse => self.browse_key(key, count, confirm),
        }
    }

    fn browse_key(&mut self, key: KeyEvent, count: usize, confirm: &[bool]) -> Command {
        match key.code {
            KeyCode::Char('q') => Command::Quit,
            KeyCode::Esc if self.expanded => {
                self.expanded = false;
                Command::Redraw
            }
            KeyCode::Tab | KeyCode::Down | KeyCode::Char('j') => self.move_by(1, count),
            KeyCode::BackTab | KeyCode::Up | KeyCode::Char('k') => self.move_by(-1, count),
            KeyCode::Home | KeyCode::Char('g') => self.select(0, count),
            KeyCode::End | KeyCode::Char('G') => self.select(count.saturating_sub(1), count),
            KeyCode::Enter if count > 0 => {
                self.expanded = !self.expanded;
                self.scroll = 0;
                Command::Redraw
            }
            KeyCode::Char('a') | KeyCode::Right if !confirm.is_empty() => {
                self.mode = Mode::Actions;
                self.action_idx = 0;
                Command::Redraw
            }
            KeyCode::Char(c @ '1'..='9') => self.trigger(c as usize - '1' as usize, confirm),
            KeyCode::PageDown | KeyCode::Char('J') => {
                self.scroll = self.scroll.saturating_add(SCROLL_STEP);
                Command::Redraw
            }
            KeyCode::PageUp | KeyCode::Char('K') => {
                self.scroll = self.scroll.saturating_sub(SCROLL_STEP);
                Command::Redraw
            }
            KeyCode::Char('d') | KeyCode::Delete if count > 0 => {
                self.mode = Mode::Confirm(Pending::Dismiss);
                Command::Redraw
            }
            KeyCode::Char('c' | 'y') if count > 0 => Command::Copy,
            KeyCode::Char('?') => {
                self.mode = Mode::Help;
                Command::Redraw
            }
            _ => Command::None,
        }
    }

    fn actions_key(&mut self, key: KeyEvent, confirm: &[bool]) -> Command {
        let n = confirm.len();
        if n == 0 {
            self.mode = Mode::Browse;
            return Command::Redraw;
        }
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.mode = Mode::Browse;
                Command::Redraw
            }
            KeyCode::Right | KeyCode::Tab | KeyCode::Char('l') => {
                self.action_idx = (self.action_idx + 1) % n;
                Command::Redraw
            }
            KeyCode::Left | KeyCode::BackTab | KeyCode::Char('h') => {
                self.action_idx = (self.action_idx + n - 1) % n;
                Command::Redraw
            }
            KeyCode::Enter => self.trigger(self.action_idx.min(n - 1), confirm),
            KeyCode::Char(c @ '1'..='9') => self.trigger(c as usize - '1' as usize, confirm),
            _ => Command::None,
        }
    }

    fn trigger(&mut self, idx: usize, confirm: &[bool]) -> Command {
        match confirm.get(idx) {
            None => Command::None,
            Some(true) => {
                self.action_idx = idx;
                self.mode = Mode::Confirm(Pending::Invoke(idx));
                Command::Redraw
            }
            Some(false) => {
                self.action_idx = idx;
                self.mode = Mode::Browse;
                Command::Invoke(idx)
            }
        }
    }

    fn move_by(&mut self, delta: isize, count: usize) -> Command {
        if count == 0 {
            return Command::None;
        }
        let next = (self.selected as isize + delta).rem_euclid(count as isize) as usize;
        self.select(next, count)
    }

    /// Select an artifact, resetting per-artifact view state.
    pub fn select(&mut self, index: usize, count: usize) -> Command {
        if count == 0 {
            self.selected = 0;
            return Command::None;
        }
        let index = index.min(count - 1);
        if index == self.selected {
            return Command::None;
        }
        self.selected = index;
        self.scroll = 0;
        self.action_idx = 0;
        Command::SelectionChanged
    }

    /// Keep the selection valid after the artifact list shrank.
    pub fn clamp(&mut self, count: usize) {
        if self.selected >= count {
            self.selected = count.saturating_sub(1);
            self.scroll = 0;
            self.action_idx = 0;
        }
        if count == 0 {
            self.expanded = false;
            self.mode = Mode::Browse;
        }
    }

    /// The user is in the middle of answering something; do not move the
    /// selection under them when new artifacts arrive.
    pub fn is_busy(&self) -> bool {
        !matches!(self.mode, Mode::Browse)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    fn ch(c: char) -> KeyEvent {
        key(KeyCode::Char(c))
    }

    #[test]
    fn navigation_wraps() {
        let mut ui = UiState::default();
        assert_eq!(ui.handle_key(key(KeyCode::Tab), 3, &[]), Command::SelectionChanged);
        assert_eq!(ui.selected, 1);
        ui.handle_key(key(KeyCode::Down), 3, &[]);
        assert_eq!(ui.handle_key(ch('j'), 3, &[]), Command::SelectionChanged);
        assert_eq!(ui.selected, 0);
        ui.handle_key(key(KeyCode::BackTab), 3, &[]);
        assert_eq!(ui.selected, 2);
        ui.handle_key(ch('g'), 3, &[]);
        assert_eq!(ui.selected, 0);
        ui.handle_key(ch('G'), 3, &[]);
        assert_eq!(ui.selected, 2);
        assert_eq!(ui.handle_key(key(KeyCode::Tab), 0, &[]), Command::None);
        assert_eq!(ui.handle_key(key(KeyCode::Tab), 1, &[]), Command::SelectionChanged);
    }

    #[test]
    fn expand_and_scroll_reset_on_selection() {
        let mut ui = UiState::default();
        assert_eq!(ui.handle_key(key(KeyCode::Enter), 2, &[]), Command::Redraw);
        assert!(ui.expanded);
        ui.handle_key(key(KeyCode::PageDown), 2, &[]);
        assert_eq!(ui.scroll, SCROLL_STEP);
        ui.handle_key(key(KeyCode::Tab), 2, &[]);
        assert_eq!(ui.scroll, 0);
        assert!(ui.expanded, "expanded view follows the selection");
        ui.handle_key(key(KeyCode::Esc), 2, &[]);
        assert!(!ui.expanded);
        assert_eq!(ui.handle_key(key(KeyCode::Esc), 2, &[]), Command::None, "esc never quits");
        assert_eq!(ui.handle_key(ch('q'), 2, &[]), Command::Quit);
    }

    #[test]
    fn direct_action_without_confirmation() {
        let mut ui = UiState::default();
        assert_eq!(ui.handle_key(ch('2'), 1, &[false, false]), Command::Invoke(1));
        assert_eq!(ui.mode, Mode::Browse);
        assert_eq!(ui.handle_key(ch('3'), 1, &[false, false]), Command::None);
    }

    #[test]
    fn sensitive_action_requires_confirmation() {
        let mut ui = UiState::default();
        assert_eq!(ui.handle_key(ch('1'), 1, &[true]), Command::Redraw);
        assert_eq!(ui.mode, Mode::Confirm(Pending::Invoke(0)));
        assert_eq!(ui.handle_key(ch('n'), 1, &[true]), Command::Redraw);
        assert_eq!(ui.mode, Mode::Browse);
        ui.handle_key(ch('1'), 1, &[true]);
        assert_eq!(ui.handle_key(ch('x'), 1, &[true]), Command::None);
        assert_eq!(ui.handle_key(ch('y'), 1, &[true]), Command::Invoke(0));
        assert_eq!(ui.mode, Mode::Browse);
    }

    #[test]
    fn action_bar_selection() {
        let mut ui = UiState::default();
        assert_eq!(ui.handle_key(ch('a'), 1, &[]), Command::None, "no actions, no action mode");
        ui.handle_key(ch('a'), 1, &[false, true, false]);
        assert_eq!(ui.mode, Mode::Actions);
        ui.handle_key(key(KeyCode::Right), 1, &[false, true, false]);
        assert_eq!(ui.action_idx, 1);
        assert_eq!(ui.handle_key(key(KeyCode::Enter), 1, &[false, true, false]), Command::Redraw);
        assert_eq!(ui.mode, Mode::Confirm(Pending::Invoke(1)));
        ui.handle_key(key(KeyCode::Esc), 1, &[false, true, false]);
        ui.handle_key(ch('a'), 1, &[false, true, false]);
        assert_eq!(ui.action_idx, 0, "the action bar always opens on the first action");
        ui.handle_key(key(KeyCode::Left), 1, &[false, true, false]);
        assert_eq!(ui.action_idx, 2, "and wraps around");
        assert_eq!(ui.handle_key(key(KeyCode::Enter), 1, &[false, true, false]), Command::Invoke(2));
    }

    #[test]
    fn dismiss_is_confirmed() {
        let mut ui = UiState::default();
        assert_eq!(ui.handle_key(ch('d'), 0, &[]), Command::None);
        ui.handle_key(ch('d'), 2, &[]);
        assert_eq!(ui.mode, Mode::Confirm(Pending::Dismiss));
        assert!(ui.is_busy());
        assert_eq!(ui.handle_key(key(KeyCode::Enter), 2, &[]), Command::Dismiss);
    }

    #[test]
    fn copy_key() {
        let mut ui = UiState::default();
        assert_eq!(ui.handle_key(ch('c'), 0, &[]), Command::None, "nothing to copy");
        assert_eq!(ui.handle_key(ch('c'), 2, &[]), Command::Copy);
        ui.expanded = true;
        assert_eq!(ui.handle_key(ch('y'), 2, &[]), Command::Copy);
        // While confirming, `y` means yes, not copy.
        ui.mode = Mode::Confirm(Pending::Dismiss);
        assert_eq!(ui.handle_key(ch('y'), 2, &[]), Command::Dismiss);
    }

    #[test]
    fn clamp_after_removal() {
        let mut ui = UiState { selected: 4, expanded: true, ..Default::default() };
        ui.clamp(2);
        assert_eq!(ui.selected, 1);
        ui.clamp(0);
        assert_eq!(ui.selected, 0);
        assert!(!ui.expanded);
    }

    #[test]
    fn ctrl_c_quits_from_any_mode() {
        let mut ui = UiState { mode: Mode::Confirm(Pending::Dismiss), ..Default::default() };
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(ui.handle_key(ctrl_c, 1, &[]), Command::Quit);
    }
}
