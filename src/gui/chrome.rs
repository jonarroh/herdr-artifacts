//! Everything around the artifact: header, buttons and history sidebar.

use eframe::egui::{self, Align, Button, Color32, Label, Layout, RichText, Ui, vec2};

use super::{
    Intent,
    session::{Current, Session},
    theme,
};
use crate::{renderer::type_icon, time};

pub fn copy_text(session: &Session) -> Option<String> {
    session.current()?.content.as_ref().ok()?.copy_text()
}

pub fn header(ui: &mut Ui, session: &Session, history_open: bool, intents: &mut Vec<Intent>) {
    let metas = session.repo().metas();
    ui.horizontal(|ui| {
        if let Some(Current { artifact, .. }) = session.current() {
            let kind = format!("{}  {}", type_icon(&artifact.kind), artifact.kind.to_uppercase());
            theme::chip(ui, &kind, theme::kind_color(&artifact.kind));
            let age = time::parse_rfc3339(&artifact.created_at).map(|t| time::age(time::now_unix(), t));
            let info = [artifact.project_id.clone(), artifact.agent_id.clone(), artifact.status.clone(), age]
                .into_iter()
                .flatten()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("  ·  ");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if metas.len() > 1 {
                    history_toggle(ui, session, history_open, intents);
                }
                ui.add(Label::new(RichText::new(info).small().weak()).truncate());
            });
        }
    });
    if let Some(Current { artifact, .. }) = session.current() {
        ui.add_space(2.0);
        let title = RichText::new(&artifact.title).heading().strong().color(ui.visuals().widgets.active.fg_stroke.color);
        ui.add(Label::new(title).truncate());
        if let Some(description) = &artifact.description {
            ui.add(Label::new(RichText::new(description).weak()).truncate());
        }
    }
}

fn history_toggle(ui: &mut Ui, session: &Session, open: bool, intents: &mut Vec<Intent>) {
    let metas = session.repo().metas();
    // Draw attention to unanswered artifacts that are not on screen.
    let elsewhere = metas
        .iter()
        .enumerate()
        .any(|(i, m)| Some(i) != session.selected_index() && m.actions > 0 && m.response.is_none());
    let mut label = RichText::new(format!("☰  {}", metas.len())).small();
    if elsewhere {
        label = label.color(ui.visuals().warn_fg_color);
    }
    let hint = if elsewhere { "History (H) · some are waiting for a reply" } else { "History (H)" };
    if ui.add(Button::selectable(open, label)).on_hover_text(hint).clicked() {
        intents.push(Intent::ToggleHistory);
    }
}

pub fn footer(ui: &mut Ui, session: &Session, intents: &mut Vec<Intent>) {
    if let Some((text, is_error)) = &session.status {
        let color = if *is_error { ui.visuals().error_fg_color } else { ui.visuals().weak_text_color() };
        ui.add(Label::new(RichText::new(text).small().color(color)).truncate());
    }
    ui.horizontal_wrapped(|ui| {
        if let Some(current) = session.current() {
            let artifact = &current.artifact;
            // The first plain reply is the main thing to do.
            let primary = artifact.actions.iter().position(|a| a.kind == "respond");
            for (index, action) in artifact.actions.iter().enumerate() {
                let chosen = artifact.response.as_deref() == Some(action.id.as_str());
                let mark = if chosen { "✔  " } else { "" };
                let mut text = RichText::new(format!("{mark}{}", action.label)).strong();
                let mut button_fill = None;
                if primary == Some(index) {
                    text = text.color(Color32::WHITE);
                    button_fill = Some(ui.visuals().selection.bg_fill);
                }
                let mut button = Button::new(text).min_size(vec2(104.0, 34.0));
                if let Some(fill) = button_fill {
                    button = button.fill(fill);
                }
                if ui.add(button).on_hover_text(format!("Shortcut: {}", index + 1)).clicked() {
                    intents.push(Intent::Action(index));
                }
            }
        }
        // Secondary tools stay quiet until hovered.
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if session.current().is_none() {
                return;
            }
            let quiet = |label: &str| {
                Button::new(RichText::new(label).weak())
                    .frame_when_inactive(false)
                    .min_size(vec2(0.0, 34.0))
            };
            if ui.add(quiet("Delete")).on_hover_text("D").clicked() {
                intents.push(Intent::Dismiss);
            }
            if let Some(text) = copy_text(session)
                && ui.add(quiet("Copy")).on_hover_text("C").clicked()
            {
                intents.push(Intent::Copy(text));
            }
        });
    });
}

pub fn history(ui: &mut Ui, session: &Session, intents: &mut Vec<Intent>) {
    let metas = session.repo().metas();
    ui.label(RichText::new(format!("HISTORY  ·  {}", metas.len())).small().strong().weak());
    ui.add_space(4.0);
    let selected = session.selected_index();
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.with_layout(Layout::top_down_justified(Align::Min), |ui| {
            // Newest first.
            for (index, meta) in metas.iter().enumerate().rev() {
                let is_selected = selected == Some(index);
                let mut label = RichText::new(format!("{}  {}", type_icon(&meta.kind), meta.title));
                if meta.actions > 0 && meta.response.is_none() && !is_selected {
                    label = label.color(ui.visuals().warn_fg_color);
                }
                let button = Button::selectable(is_selected, label).truncate().min_size(vec2(0.0, 30.0));
                if ui.add(button).clicked() {
                    intents.push(Intent::Select(index));
                }
            }
        });
    });
}
