//! Recovery is a History filter, using its list/detail and async action patterns.
use super::*;
use crate::control::{
    ControlReply,
    recovery::{RecoveryCommand, RecoverySummary},
};
use crate::desktop::file_dialog::{Dialog, Pending as FileChoice, Target};
use crate::rune_ui::controls::{DESCRIPTION, button_disabled};
use std::sync::mpsc::{self, Receiver};

#[derive(Default)]
pub(super) struct RecoveryPane {
    pub(super) showing: bool,
    entries: Vec<RecoverySummary>,
    selected: Option<String>,
    revision: Option<u64>,
    pending: Option<Receiver<(Kind, ControlReply)>>,
    content: Option<String>,
    raw: bool,
    message: Option<String>,
    armed: bool,
    dialog: Option<FileChoice>,
    export: Option<(String, bool, bool)>,
    queued_text: Option<(String, bool)>,
    queued_action: Option<RecoveryCommand>,
    playback: Option<Playback>,
}
#[derive(Clone)]
enum Kind {
    List(u64),
    Text(String, bool),
    Action,
}
impl RecoveryPane {
    pub(super) fn fixture() -> Self {
        let id = "0123456789abcdef0123456789abcdef".to_string();
        Self {
            showing: true,
            selected: Some(id.clone()),
            entries: vec![RecoverySummary {
                id,
                created_ms: Some(crate::history::now_ms() - 90_000),
                saved_audio_ms: 12_400,
                bytes: 1_048_576,
                capture: "sealed".into(),
                delivery: "unknown".into(),
                error: None,
                model: Some("Parakeet v3".into()),
                language: Some("en".into()),
                has_raw_text: true,
                has_final_text: true,
                audio_available: true,
                kept: false,
                retrying: false,
                deleting: false,
            }],
            content: Some(
                "The saved transcript remains available after an interrupted delivery.".into(),
            ),
            ..Self::default()
        }
    }
    pub(super) fn poll(&mut self, visible: bool) -> bool {
        let mut changed = false;
        if let Some(playback) = &mut self.playback {
            changed |= playback.poll();
            if !visible || !self.showing {
                playback.stop();
            }
        }
        if let Some(answer) = self.dialog.as_ref().and_then(FileChoice::poll) {
            self.dialog = None;
            if let Ok(Some(path)) = answer
                && let Some((id, audio, raw)) = self.export.take()
            {
                let command = if audio {
                    RecoveryCommand::ExportAudio {
                        id: id.clone(),
                        path,
                    }
                } else {
                    RecoveryCommand::ExportText {
                        id: id.clone(),
                        path,
                        raw,
                    }
                };
                self.request(command, Kind::Action, None);
            }
            changed = true;
        }
        let answer = self
            .pending
            .as_ref()
            .and_then(|receiver| match receiver.try_recv() {
                Ok(answer) => Some(answer),
                Err(mpsc::TryRecvError::Disconnected) => Some((
                    Kind::Action,
                    ControlReply::error(
                        "worker-stopped",
                        "The recovery operation stopped. Saved work remains available.",
                    ),
                )),
                Err(mpsc::TryRecvError::Empty) => None,
            });
        if let Some((kind, answer)) = answer {
            self.pending = None;
            match (kind, answer) {
                (Kind::List(revision), ControlReply::Recovery { entries }) => {
                    self.entries = entries;
                    self.entries
                        .sort_by_key(|entry| std::cmp::Reverse(entry.created_ms));
                    self.revision = Some(revision);
                    if self
                        .selected
                        .as_ref()
                        .is_some_and(|id| !self.entries.iter().any(|entry| &entry.id == id))
                    {
                        self.selected = None;
                        self.content = None;
                        self.armed = false;
                    }
                }
                (Kind::Text(id, raw), ControlReply::Transcript { text })
                    if self.selected.as_ref() == Some(&id) && self.raw == raw =>
                {
                    self.content = Some(text)
                }
                (Kind::List(revision), ControlReply::Error { message, .. }) => {
                    self.message = Some(message);
                    self.revision = Some(revision);
                }
                (_, ControlReply::Error { message, .. })
                | (_, ControlReply::Done { message, .. }) => {
                    self.message = Some(message);
                    self.revision = None;
                }
                _ => {}
            }
            changed = true;
        }
        if self.pending.is_none()
            && let Some(command) = self.queued_action.take()
        {
            self.request(command, Kind::Action, None);
        }
        if visible
            && self.showing
            && self.pending.is_none()
            && let Some((id, raw)) = self.queued_text.take()
        {
            self.request(
                RecoveryCommand::Text {
                    id: id.clone(),
                    raw,
                },
                Kind::Text(id, raw),
                None,
            );
        }
        #[cfg(target_os = "macos")]
        if visible
            && self.pending.is_none()
            && let Some(runtime) = crate::macos_rune::runtime()
        {
            let revision = runtime.recovery().revision();
            if self.revision != Some(revision) {
                self.request(RecoveryCommand::List, Kind::List(revision), None);
            }
        }
        changed
    }
    fn request(&mut self, command: RecoveryCommand, kind: Kind, wake: Option<rune::Waker>) {
        if self.pending.is_some() {
            if matches!(kind, Kind::Action) {
                self.queued_action = Some(command);
            }
            return;
        }
        #[cfg(target_os = "macos")]
        if let Some(runtime) = crate::macos_rune::runtime() {
            let client = runtime.recovery();
            let (send, receive) = mpsc::sync_channel(1);
            match std::thread::Builder::new()
                .name("hex-recovery-action".into())
                .spawn(move || {
                    let reply = crate::recovery::execute(&client, command);
                    let _ = send.send((kind, reply));
                    if let Some(wake) = wake {
                        wake();
                    }
                }) {
                Ok(_) => self.pending = Some(receive),
                Err(_) => {
                    self.message = Some(
                        tr("The recovery operation stopped. Saved work remains available.").into(),
                    )
                }
            }
            return;
        }
        let _ = (command, kind, wake);
    }
}

pub(super) fn header(ui: &Ui, p: Paint, actions: &mut Vec<El>) {
    if ui.history.recovery.entries.is_empty() && !crate::labs::recording_recovery() {
        return;
    }
    actions.push(button(
        p,
        "history-recovery-filter",
        if ui.history.recovery.showing {
            tr("History")
        } else {
            tr("Recovery")
        },
        ButtonKind::Secondary,
        |ui: &mut Ui, _| {
            ui.history.recovery.showing = !ui.history.recovery.showing;
            ui.history.recovery.armed = false;
        },
    ));
}
pub(super) fn pane(ui: &Ui, p: Paint) -> El {
    let state = &ui.history.recovery;
    let mut list = col().key("recovery-list").label(tr("Recovery"));
    for (index, entry) in state.entries.iter().enumerate() {
        let id = entry.id.clone();
        list = list.child(
            ld::item(
                p,
                Item {
                    key: format!("recovery-{id}"),
                    title: status(entry).into(),
                    meta: format!(
                        "{} s · {} MiB",
                        entry.saved_audio_ms / 1000,
                        entry.bytes / (1024 * 1024)
                    ),
                    meta_ink: None,
                    time: entry.created_ms.map(event_age).unwrap_or_default(),
                    hover: vec![],
                    selected: state.selected.as_ref() == Some(&id),
                },
                move |ui: &mut Ui| select(ui, id.clone()),
            )
            .position_in_set(index + 1, state.entries.len()),
        );
    }
    let heading = text(tr("Local safety copies"))
        .font(DESCRIPTION)
        .color(p.muted())
        .px(16.0)
        .pb(12.0);
    let column = ld::list_column(p, heading, "recovery-scroll", list);
    let mut detail = col().key("recovery-detail").gap(12.0).grow(1.0);
    if let Some(message) = &state.message {
        detail = detail.child(
            text(message.clone())
                .font(DESCRIPTION)
                .color(p.muted())
                .wrap(),
        );
    }
    let Some(entry) = state
        .selected
        .as_ref()
        .and_then(|id| state.entries.iter().find(|entry| &entry.id == id))
    else {
        return ld::body(
            p,
            "recovery-body",
            column,
            detail.child(
                text(tr("Select a recovery item."))
                    .font(DESCRIPTION)
                    .color(p.muted()),
            ),
        );
    };
    detail = detail.child(text(status(entry)).font(p.k.ty.body_medium).color(p.text()));
    detail = detail.child(
        text(tr_fill(
            "Saved audio: {} s",
            &(entry.saved_audio_ms / 1000).to_string(),
        ))
        .font(DESCRIPTION)
        .color(p.muted()),
    );
    if let Some(model) = &entry.model {
        detail = detail.child(
            text(format!(
                "{model} · {}",
                entry.language.as_deref().unwrap_or_default()
            ))
            .font(DESCRIPTION)
            .color(p.muted()),
        );
    }
    if entry.kept {
        detail = detail.child(
            text(tr("Kept until you resolve or delete it."))
                .font(DESCRIPTION)
                .color(p.muted())
                .wrap(),
        );
    }
    if entry.error.as_deref() == Some("model_unavailable") {
        detail = detail.child(
            text(tr(
                "Select the original model in Settings, or retry with the current loaded model.",
            ))
            .font(DESCRIPTION)
            .color(p.muted())
            .wrap(),
        );
    }
    detail = detail.child(
        text(tr(
            "Resolved copies expire: audio after one hour, text after 24 hours.",
        ))
        .font(DESCRIPTION)
        .color(p.muted())
        .wrap(),
    );
    let unavailable = state.pending.is_some()
        || state.dialog.is_some()
        || entry.capture == "recording"
        || entry.deleting;
    let id = entry.id.clone();
    let mut actions = row().key("recovery-actions").gap(8.0).flex_wrap();
    if entry.audio_available {
        let retry_id = id.clone();
        actions = actions.child(button_disabled(
            p,
            "recovery-retry",
            tr("Retry locally"),
            ButtonKind::Primary,
            unavailable || entry.retrying,
            move |ui: &mut Ui, _| {
                request(
                    ui,
                    RecoveryCommand::Retry {
                        id: retry_id.clone(),
                        current_model: false,
                    },
                )
            },
        ));
        let current_id = id.clone();
        actions = actions.child(button_disabled(
            p,
            "recovery-retry-current",
            tr("Retry with current model"),
            ButtonKind::Secondary,
            unavailable || entry.retrying,
            move |ui: &mut Ui, _| {
                request(
                    ui,
                    RecoveryCommand::Retry {
                        id: current_id.clone(),
                        current_model: true,
                    },
                )
            },
        ));
        let play_id = id.clone();
        actions = actions.child(button_disabled(
            p,
            "recovery-play",
            if state.playback.as_ref().is_some_and(Playback::playing) {
                tr("Stop")
            } else {
                tr("Play")
            },
            ButtonKind::Secondary,
            unavailable,
            move |ui: &mut Ui, _| play(ui, &play_id),
        ));
    }
    detail = detail.child(actions);
    if entry.has_raw_text || entry.has_final_text {
        let raw_id = id.clone();
        let final_id = id.clone();
        detail = detail.child(
            row()
                .key("recovery-text-kind")
                .gap(8.0)
                .child(button_disabled(
                    p,
                    "recovery-show-raw",
                    tr("Raw text"),
                    ButtonKind::Secondary,
                    unavailable || !entry.has_raw_text,
                    move |ui: &mut Ui, _| show_text(ui, raw_id.clone(), true),
                ))
                .child(button_disabled(
                    p,
                    "recovery-show-final",
                    tr("Final text"),
                    ButtonKind::Secondary,
                    unavailable || !entry.has_final_text,
                    move |ui: &mut Ui, _| show_text(ui, final_id.clone(), false),
                )),
        );
    }
    if let Some(content) = &state.content {
        detail = detail.child(button(
            p,
            "recovery-copy",
            tr("Copy"),
            ButtonKind::Secondary,
            |ui: &mut Ui, tx| {
                if let Some(content) = &ui.history.recovery.content {
                    tx.copy(content.clone());
                    ui.history.recovery.message = Some(tr("Copied").into());
                }
            },
        ));
        let preview: String = content.chars().take(8_192).collect();
        detail = detail.child(
            text(preview)
                .key("recovery-text")
                .font(DESCRIPTION)
                .color(p.text())
                .wrap(),
        );
    }
    let keep_id = id.clone();
    let resolve_id = id.clone();
    detail = detail.child(
        row()
            .key("recovery-retention-actions")
            .gap(8.0)
            .flex_wrap()
            .child(button_disabled(
                p,
                "recovery-keep",
                tr("Keep"),
                ButtonKind::Secondary,
                unavailable,
                move |ui: &mut Ui, _| {
                    request(
                        ui,
                        RecoveryCommand::Resolve {
                            id: keep_id.clone(),
                            keep: true,
                        },
                    )
                },
            ))
            .child(button_disabled(
                p,
                "recovery-resolve",
                tr("Resolve"),
                ButtonKind::Secondary,
                unavailable,
                move |ui: &mut Ui, _| {
                    request(
                        ui,
                        RecoveryCommand::Resolve {
                            id: resolve_id.clone(),
                            keep: false,
                        },
                    )
                },
            ))
            .child(button_disabled(
                p,
                "recovery-export-audio",
                tr("Export audio"),
                ButtonKind::Secondary,
                unavailable || !entry.audio_available,
                |ui: &mut Ui, _| choose_export(ui, true),
            ))
            .child(button_disabled(
                p,
                "recovery-export-text",
                tr("Export text"),
                ButtonKind::Secondary,
                unavailable || state.content.is_none(),
                |ui: &mut Ui, _| choose_export(ui, false),
            )),
    );
    detail = detail.child(button_disabled(
        p,
        "recovery-delete",
        if state.armed {
            tr("Delete permanently")
        } else {
            tr("Delete")
        },
        ButtonKind::Danger,
        state.pending.is_some() || entry.deleting,
        move |ui: &mut Ui, _| {
            if ui.history.recovery.armed {
                request(ui, RecoveryCommand::Delete { id: id.clone() });
                ui.history.recovery.armed = false;
            } else {
                ui.history.recovery.armed = true;
                ui.history.recovery.message = Some(
                    tr("Delete this safety copy permanently? Its History entry is kept.").into(),
                );
            }
        },
    ));
    ld::body(
        p,
        "recovery-body",
        column,
        ld::card_scroll(p, "recovery-detail-scroll", detail),
    )
}
fn status(entry: &RecoverySummary) -> &'static str {
    if entry.deleting {
        tr("Deleting…")
    } else if entry.retrying {
        tr("Transcribing…")
    } else if entry.capture == "unknown_format" {
        tr("Unreadable recovery item")
    } else if entry.capture == "recording" {
        tr("Recording")
    } else if entry.delivery == "unknown" {
        tr("Delivery unconfirmed")
    } else if entry.delivery == "failed" {
        tr("Delivery failed")
    } else if entry.error.is_some() {
        tr("Needs attention")
    } else if entry.delivery == "reported_success" || entry.delivery == "suppressed" {
        tr("Complete")
    } else if entry.has_raw_text || entry.has_final_text {
        tr("Saved transcript")
    } else {
        tr("Interrupted")
    }
}
fn request(ui: &mut Ui, command: RecoveryCommand) {
    let waker = ui.modes.waker.clone();
    ui.history.recovery.request(command, Kind::Action, waker);
}
fn select(ui: &mut Ui, id: String) {
    let state = &mut ui.history.recovery;
    if let Some(playback) = &mut state.playback {
        playback.stop();
    }
    state.selected = Some(id.clone());
    state.content = None;
    state.armed = false;
    state.message = None;
    let raw = state
        .entries
        .iter()
        .find(|entry| entry.id == id)
        .is_some_and(|entry| !entry.has_final_text);
    show_text(ui, id, raw);
}
fn show_text(ui: &mut Ui, id: String, raw: bool) {
    let waker = ui.modes.waker.clone();
    let state = &mut ui.history.recovery;
    state.raw = raw;
    state.content = None;
    if state
        .entries
        .iter()
        .find(|entry| entry.id == id)
        .is_some_and(|entry| {
            if raw {
                entry.has_raw_text
            } else {
                entry.has_final_text
            }
        })
    {
        state.queued_text = Some((id, raw));
        if let Some(waker) = waker {
            waker();
        }
    }
}
fn choose_export(ui: &mut Ui, audio: bool) {
    let Some(id) = ui.history.recovery.selected.clone() else {
        return;
    };
    let waker = ui.modes.waker.clone();
    let extension = if audio { "wav" } else { "txt" };
    let dialog = crate::desktop::file_dialog::open(
        Dialog {
            title: tr(if audio { "Export audio" } else { "Export text" }).into(),
            target: Target::Save {
                file_name: format!("hex-recovery-{id}.{extension}"),
            },
            directory: None,
        },
        move || {
            if let Some(waker) = &waker {
                waker();
            }
        },
    );
    ui.history.recovery.export = Some((id, audio, ui.history.recovery.raw));
    ui.history.recovery.dialog = Some(dialog);
}
fn play(ui: &mut Ui, id: &str) {
    if let Some(playback) = &mut ui.history.recovery.playback
        && playback.playing()
    {
        playback.stop();
        return;
    }
    #[cfg(target_os = "macos")]
    if let Some(runtime) = crate::macos_rune::runtime() {
        let Ok(id) = crate::recovery::SessionId::try_from(id.to_owned()) else {
            return;
        };
        let waker = ui.modes.waker.clone();
        ui.history.recovery.playback = Some(Playback::start_recovery(
            runtime.recovery(),
            id,
            move || {
                if let Some(waker) = &waker {
                    waker();
                }
            },
        ));
    }
    #[cfg(not(target_os = "macos"))]
    let _ = id;
}
