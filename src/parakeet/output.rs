//! Ordered result preservation and delivery, extracted from the existing worker.
use super::*;

pub(super) fn finish_output(
    job: OutputJob,
    paste: &mut impl FnMut(&str, PasteMode, &dyn Fn() -> bool) -> Result<()>,
    last_transcript: &mut Option<String>,
    history: Option<&History>,
) -> WorkerEvent {
    match job {
        OutputJob::PreparePaste => unreachable!("paste preparation bypasses ordered output"),
        OutputJob::Completed {
            job_id,
            control,
            target,
            result,
        } if !control.is_cancelled() => {
            let result = *result;
            let processing = result
                .as_ref()
                .ok()
                .and_then(|result| result.processing.clone());
            let result = result.and_then(|mut completed| {
                if control.is_cancelled() {
                    return Err("dictation was cancelled".into());
                }
                let paste_started = Instant::now();
                if let Some(session) = &control.recovery {
                    let failure = if matches!(target, TranscriptionTarget::CopyOnlyPaste | TranscriptionTarget::RawCopyOnlyPaste) {
                        Some(crate::recovery::Failure::Interrupted)
                    } else if completed.processing.as_ref().is_some_and(|processing| processing.fallback.is_some()) {
                        Some(crate::recovery::Failure::Processing)
                    } else { None };
                    if let Some(failure) = failure {
                        session.preserve_failure(failure).map_err(|_| "The local safety copy could not preserve the interruption. Nothing was delivered.".to_string())?;
                    }
                    session.prepare_delivery(&completed.text).map_err(|_| "The local safety copy could not save the final transcript. Nothing was delivered.".to_string())?;
                }
                if !completed.text.trim().is_empty() {
                    if control.recovery.is_some() && matches!(target,
                        TranscriptionTarget::Paste | TranscriptionTarget::CopyOnlyPaste
                        | TranscriptionTarget::RawCopyOnlyPaste | TranscriptionTarget::RawPaste)
                    { *last_transcript = Some(completed.text.clone()); }
                    let commit = || control.begin_output();
                    let delivery = match target {
                        TranscriptionTarget::Paste
                        | TranscriptionTarget::CopyOnlyPaste
                        | TranscriptionTarget::RawCopyOnlyPaste
                        | TranscriptionTarget::RawPaste => {
                            // Labs › Copy dictation only, read as the output
                            // commits: dictations go to the clipboard
                            // unpasted; Paste last dictation still pastes.
                            // A rescued microphone-loss dictation is always
                            // copied: the disruption may have taken focus.
                            let mode = if crate::labs::copy_only()
                                || matches!(
                                    target,
                                    TranscriptionTarget::CopyOnlyPaste
                                        | TranscriptionTarget::RawCopyOnlyPaste
                                )
                            {
                                PasteMode::CopyOnly
                            } else {
                                PasteMode::Continue
                            };
                            // Labs › Return to app: the app the capture
                            // started in comes back first. When it can't,
                            // nothing is pasted; the text stays for Paste
                            // last dictation, and History keeps only pasted
                            // output, as on any failed paste. A cancelled job
                            // leaves the last result alone, as it does
                            // everywhere else.
                            let focus = if matches!(mode, PasteMode::Continue) {
                                crate::paste::return_to_app::bring_to_front(completed.return_to)
                            } else { Ok(()) };
                            match focus {
                                Err(error) => {
                                    if !control.is_cancelled() { *last_transcript = Some(completed.text.clone()); }
                                    Err(error)
                                },
                                Ok(()) => paste(&completed.text, mode, &commit),
                            }
                        }
                        TranscriptionTarget::Service
                        | TranscriptionTarget::Print
                        | TranscriptionTarget::RawPrint => commit()
                            .then_some(())
                            .ok_or_else(|| eyre!("dictation was cancelled")),
                    }
                    .map_err(|error| error.to_string());
                    if let Some(session) = &control.recovery && !control.is_cancelled() {
                        if let Some(history) = history {
                            record_history(history, target, &mut completed, Some(session), Some(delivery.is_ok()));
                        } else if completed.recording.is_some() && session.preserve_failure(crate::recovery::Failure::Storage).is_err() {
                            tracing::warn!("could not preserve unavailable retained-audio handoff");
                        }
                    }
                    delivery?;
                    if matches!(
                        target,
                        TranscriptionTarget::Paste
                            | TranscriptionTarget::CopyOnlyPaste
                            | TranscriptionTarget::RawCopyOnlyPaste
                            | TranscriptionTarget::RawPaste
                    ) {
                        *last_transcript = Some(completed.text.clone());
                    }
                    if control.recovery.is_none()
                        && let Some(history) = history { record_history(history, target, &mut completed, None, None); }
                }
                tracing::info!(
                    audio_ms = completed.timings.audio_ms,
                    queue_ms = completed.timings.queue_ms,
                    prepare_ms = completed.timings.prepare_ms,
                    inference_ms = completed.timings.inference_ms,
                    paste_ms = paste_started.elapsed().as_millis(),
                    total_ms = completed.timings.total_started.elapsed().as_millis(),
                    "dictation pipeline completed"
                );
                Ok(completed.text)
            });
            if let Some(session) = &control.recovery {
                let delivery = if result.as_ref().is_ok_and(|text| text.trim().is_empty()) {
                    crate::recovery::Delivery::Suppressed
                } else if result.is_ok() {
                    crate::recovery::Delivery::ReportedSuccess
                } else {
                    crate::recovery::Delivery::Failed
                };
                if session.outcome(delivery).is_err() {
                    tracing::warn!(
                        session = session.id().as_str(),
                        "recovery delivery outcome remains uncertain"
                    );
                }
            }
            if matches!(
                target,
                TranscriptionTarget::Print | TranscriptionTarget::RawPrint
            ) && !control.is_cancelled()
            {
                control.answer_print(match &result {
                    Ok(text) if text.trim().is_empty() => {
                        crate::control::PrintedTranscript::NoSpeech
                    }
                    Ok(text) => crate::control::PrintedTranscript::Text(text.clone()),
                    Err(error) => crate::control::PrintedTranscript::Failed(error.clone()),
                });
            }
            if control.is_cancelled() {
                WorkerEvent::Cancelled { job_id }
            } else {
                WorkerEvent::Completed {
                    job_id,
                    target,
                    result,
                    processing,
                }
            }
        }
        OutputJob::Completed { job_id, .. } | OutputJob::Cancelled { job_id } => {
            WorkerEvent::Cancelled { job_id }
        }
        OutputJob::Paste { kind, .. } => {
            let result = match kind {
                PasteKind::LastTranscript => last_transcript
                    .as_deref()
                    .ok_or_else(|| "no previous transcript is available".to_string())
                    .and_then(|text| {
                        paste(text, PasteMode::Continue, &|| true)
                            .map_err(|error| error.to_string())?;
                        Ok(text.to_string())
                    }),
                PasteKind::MeetingDelta => {
                    paste_meeting_delta(paste).map_err(|error| error.to_string())
                }
            };
            WorkerEvent::Pasted { kind, result }
        }
    }
}

/// Record one successfully pasted result. History failures must never fail
/// the paste that already happened.
fn record_history(
    history: &History,
    target: TranscriptionTarget,
    completed: &mut CompletedTranscript,
    recovery: Option<&crate::recovery::Session>,
    delivered: Option<bool>,
) {
    let kind = match target {
        TranscriptionTarget::Paste | TranscriptionTarget::CopyOnlyPaste => HistoryKind::Dictation,
        TranscriptionTarget::RawPaste | TranscriptionTarget::RawCopyOnlyPaste => HistoryKind::Raw,
        TranscriptionTarget::Print | TranscriptionTarget::RawPrint => HistoryKind::Printed,
        TranscriptionTarget::Service => return,
    };
    let wanted_audio = completed.recording.is_some();
    let draft = HistoryDraft {
        kind,
        raw_text: completed.raw.clone(),
        final_text: completed.text.clone(),
        application: completed.application.clone(),
        processing: completed.processing.clone(),
        audio_ms: completed.timings.audio_ms,
        inference_ms: completed.timings.inference_ms as u64,
        total_ms: completed.timings.total_started.elapsed().as_millis() as u64,
        recording: completed.recording.take(),
        model: crate::history::dictation_model(),
    };
    let result = history.record_correlated(
        draft,
        recovery.map(|session| session.id().as_str().to_owned()),
        delivered,
    );
    let failed = match result {
        Ok(Some(id)) => wanted_audio && history.recording_path(id).is_none(),
        Ok(None) => false, // History retention is intentionally off.
        Err(error) => {
            tracing::warn!(%error, "could not record dictation history");
            true
        }
    };
    if failed
        && let Some(session) = recovery
        && session
            .preserve_failure(crate::recovery::Failure::Storage)
            .is_err()
    {
        tracing::warn!("could not preserve failed History handoff");
    }
}

fn paste_meeting_delta(
    paste: &mut impl FnMut(&str, PasteMode, &dyn Fn() -> bool) -> Result<()>,
) -> Result<String> {
    meeting::consume_active_or_latest_delta(|text| paste(text, PasteMode::Standalone, &|| true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recovery::{Client, Delivery, Session};

    fn job(session: Session) -> OutputJob {
        OutputJob::Completed {
            job_id: DictationJobId(1),
            control: Arc::new(JobControl {
                recovery: Some(session),
                ..Default::default()
            }),
            target: TranscriptionTarget::Paste,
            result: Box::new(Ok(CompletedTranscript {
                text: "synthetic processed result".into(),
                raw: "synthetic raw result".into(),
                application: None,
                return_to: None,
                timings: JobTimings {
                    total_started: Instant::now(),
                    queue_ms: 0,
                    audio_ms: 100,
                    prepare_ms: 0,
                    inference_ms: 0,
                },
                processing: None,
                recording: None,
            })),
        }
    }

    #[test]
    fn protected_output_is_saved_before_delivery_and_survives_delivery_failure() {
        let parent = std::env::temp_dir().canonicalize().unwrap();
        let mut nonce = [0_u8; 16];
        getrandom::fill(&mut nonce).unwrap();
        let nonce: String = nonce.iter().map(|byte| format!("{byte:02x}")).collect();
        let root = parent.join(format!("hex-output-recovery-{nonce}"));
        let client = Client::start(root.clone()).unwrap();
        let session = client.begin(Default::default()).unwrap();
        session.offer(0, &[0.25; 100]).unwrap();
        session.seal(100).unwrap();
        session.save_raw("synthetic raw result").unwrap();
        let id = session.id().clone();
        let mut last = None;
        let result = finish_output(
            job(session.clone()),
            &mut |text, _, commit| {
                assert_eq!(client.text(id.clone(), false).unwrap(), text);
                assert_eq!(
                    client.list().unwrap()[0]
                        .snapshot
                        .as_ref()
                        .unwrap()
                        .delivery,
                    Delivery::Attempting
                );
                assert!(commit());
                Err(eyre!("synthetic paste failure"))
            },
            &mut last,
            None,
        );
        assert!(matches!(
            result,
            WorkerEvent::Completed { result: Err(_), .. }
        ));
        assert_eq!(last.as_deref(), Some("synthetic processed result"));
        assert_eq!(
            client.list().unwrap()[0]
                .snapshot
                .as_ref()
                .unwrap()
                .delivery,
            Delivery::Failed
        );
        assert_eq!(
            client.text(id.clone(), true).unwrap(),
            "synthetic raw result"
        );
        client.delete(id).unwrap();
        let result = finish_output(
            job(session),
            &mut |_, _, _| panic!("deleted output cannot deliver"),
            &mut last,
            None,
        );
        assert!(matches!(
            result,
            WorkerEvent::Completed { result: Err(_), .. }
        ));
        client.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn failed_retained_wav_handoff_preserves_the_source_after_delivery() {
        let mut nonce = [0_u8; 16];
        getrandom::fill(&mut nonce).unwrap();
        let nonce: String = nonce.iter().map(|byte| format!("{byte:02x}")).collect();
        let base = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("hex-output-handoff-{nonce}"));
        let client = Client::start(base.join("recovery")).unwrap();
        let session = client.begin(Default::default()).unwrap();
        assert_eq!(client.list().unwrap().len(), 1);
        session.offer(0, &[0.25; 100]).unwrap();
        session.seal(100).unwrap();
        session.save_raw("synthetic raw result").unwrap();
        let history = History::new(crate::history::HistoryStore::open(
            base.join("history.json"),
            crate::history::HistoryRetention::Week,
            crate::history::now_ms(),
        ));
        std::fs::write(base.join("history-audio"), b"blocked directory").unwrap();
        let mut output = job(session.clone());
        if let OutputJob::Completed { result, .. } = &mut output {
            result.as_mut().as_mut().unwrap().recording = Some(
                crate::history::recordings::Recording::from_samples(vec![0.25; 100]),
            );
        }
        let result = finish_output(
            output,
            &mut |_, _, commit| {
                assert!(commit());
                Ok(())
            },
            &mut None,
            Some(&history),
        );
        assert!(matches!(
            result,
            WorkerEvent::Completed { result: Ok(_), .. }
        ));
        let entries = client.list().unwrap();
        let snapshot = entries[0].snapshot.as_ref().unwrap();
        assert_eq!(snapshot.delivery, Delivery::ReportedSuccess);
        assert_eq!(snapshot.error, Some(crate::recovery::Failure::Storage));
        assert!(snapshot.resolved_ms.is_none());
        assert_eq!(
            client.text(session.id().clone(), true).unwrap(),
            "synthetic raw result"
        );
        assert!(
            client
                .reader(session.id().clone())
                .unwrap()
                .next()
                .unwrap()
                .is_some()
        );
        drop(history);
        client.shutdown().unwrap();
        std::fs::remove_dir_all(base).unwrap();
    }
}
