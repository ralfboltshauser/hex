//! Explicit local replay on the existing engine. No processing, model switching,
//! downloads, History publication or delivery is performed by recovery.
use super::{Client, Failure, SessionId};
use crate::transcription::engine::TranscriptionEngine;
use crate::transcription::models::TranscriptionSelection;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

pub(super) struct Guard(pub(super) Arc<AtomicBool>);
impl Drop for Guard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

pub(super) fn start(client: Client, id: SessionId, current_model: bool) -> Result<(), String> {
    let guard = client
        .reserve_retry()
        .map_err(|_| "Another local recovery transcription is running.".to_string())?;
    let engine = TranscriptionEngine::shared()
        .ok_or_else(|| "Start the dictation listener before retrying locally.".to_string())?;
    let reader = client
        .start_replay(id)
        .map_err(|_| "This recording is unavailable or already in use.".to_string())?;
    let selection = if current_model {
        crate::app_settings::transcription_selection().1
    } else {
        reader.selection().clone()
    };
    let worker_client = client.clone();
    // Transfer the pinned reader only after thread creation succeeds. A spawn
    // failure must release the durable retry state without needing a restart.
    let (send, receive) =
        std::sync::mpsc::sync_channel::<(super::AudioReader, TranscriptionSelection)>(1);
    let worker = std::thread::Builder::new()
        .name("hex-recovery-transcription".into())
        .spawn(move || {
            let _guard = guard;
            let Ok((mut reader, selection)) = receive.recv() else {
                return;
            };
            let result = transcribe(&engine, &selection, &mut reader);
            if worker_client
                .finish_replay(reader, selection, result)
                .is_err()
            {
                tracing::warn!(
                    "local recovery transcript could not be committed; audio remains available"
                );
            }
        });
    if worker.is_err() {
        let _ = client.finish_replay(reader, selection, Err(Failure::Transcription));
        return Err(
            "The local recovery worker could not start; saved audio remains available.".into(),
        );
    }
    if let Err(std::sync::mpsc::SendError((reader, selection))) = send.send((reader, selection)) {
        let _ = client.finish_replay(reader, selection, Err(Failure::Transcription));
        return Err("The local recovery worker stopped; saved audio remains available.".into());
    }

    Ok(())
}

fn transcribe(
    engine: &TranscriptionEngine,
    selection: &TranscriptionSelection,
    reader: &mut super::AudioReader,
) -> Result<String, Failure> {
    // PauseSegmenter is the same natural-pause policy used for meeting audio.
    // The hard ceiling also bounds a single non-preemptible model request.
    const WINDOW: usize = 8 * 16_000;
    let mut segmenter = crate::transcription::pause_segmenter::PauseSegmenter::default();
    let mut buffer = Vec::with_capacity(WINDOW + 16_000);
    let mut consumed = 0_usize;
    let mut output = String::new();
    while let Some(samples) = reader.next().map_err(|_| Failure::Storage)? {
        let cut = segmenter
            .push(&samples)
            .map(|position| position.saturating_sub(consumed));
        buffer.extend(samples);
        let take = cut
            .filter(|position| *position > 0 && *position <= buffer.len())
            .or_else(|| (buffer.len() >= WINDOW).then_some(WINDOW));
        if let Some(take) = take {
            let remainder = buffer.split_off(take);
            append(
                engine,
                selection,
                std::mem::replace(&mut buffer, remainder),
                &mut output,
            )?;
            consumed += take;
        }
    }
    if !buffer.is_empty() {
        append(engine, selection, buffer, &mut output)?;
    }
    if output.trim().is_empty() {
        return Err(Failure::EmptyResult);
    }
    Ok(output)
}

fn append(
    engine: &TranscriptionEngine,
    selection: &TranscriptionSelection,
    samples: Vec<f32>,
    output: &mut String,
) -> Result<(), Failure> {
    let deadline = Instant::now() + Duration::from_secs(30);
    while crate::transcription::engine::dictation_busy() {
        if Instant::now() >= deadline {
            return Err(Failure::Transcription);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let selection = selection.clone();
    let result = engine
        .background(move |model| {
            let model = model
                .active()
                .filter(|model| model.matches_selection(&selection))
                .ok_or(Failure::ModelUnavailable)?;
            model
                .transcribe(samples)
                .map_err(|_| Failure::Transcription)
        })
        .map_err(|_| Failure::Transcription)?
        .ok_or(Failure::Transcription)??;
    if output.len().saturating_add(result.len()).saturating_add(1) > super::format::MAX_TEXT {
        return Err(Failure::Capacity);
    }
    if !output.is_empty() && !result.trim().is_empty() {
        output.push(' ');
    }
    output.push_str(result.trim());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires an installed default model and HEX_RECOVERY_SYNTHETIC_WAV; never opens a microphone"]
    fn installed_model_replays_saved_synthetic_audio() {
        use crate::transcription::{Transcriber, WarmTranscriber};
        let wav =
            std::env::var_os("HEX_RECOVERY_SYNTHETIC_WAV").expect("explicit synthetic fixture");
        let mut wav = hound::WavReader::open(wav).unwrap();
        assert_eq!(wav.spec().channels, 1);
        assert_eq!(wav.spec().sample_rate, 16_000);
        let samples: Vec<_> = wav
            .samples::<i16>()
            .map(|sample| sample.unwrap() as f32 / 32768.0)
            .collect();
        assert!(
            samples.len() > 8 * 16_000,
            "exercise multiple replay windows"
        );
        let mut selection = TranscriptionSelection::default();
        if let Ok(model) = std::env::var("HEX_RECOVERY_TEST_MODEL") {
            selection.model = model.parse().unwrap();
        }
        // Loading an installed model is allowed only in this explicit test.
        // Product retry uses the engine's already loaded matching model.
        let model = Transcriber::load_selection(&selection).unwrap();
        let engine = Arc::new(TranscriptionEngine::start(WarmTranscriber::serving(model)).unwrap());
        let mut foreground_wait_ms = Vec::new();
        for _ in 0..10 {
            let engine_worker = engine.clone();
            let selection_worker = selection.clone();
            let window = samples[..samples.len().min(8 * 16_000)].to_vec();
            let (started, receive) = std::sync::mpsc::sync_channel(1);
            let worker = std::thread::spawn(move || {
                engine_worker
                    .background(move |model| {
                        started.send(()).unwrap();
                        model
                            .active()
                            .filter(|model| model.matches_selection(&selection_worker))
                            .unwrap()
                            .transcribe(window)
                            .unwrap()
                    })
                    .unwrap()
                    .unwrap()
            });
            receive.recv().unwrap();
            let wait = Instant::now();
            let result = engine
                .finish_with(
                    None,
                    vec![0.0; 160],
                    Box::new(|_, _| Ok("foreground marker".into())),
                )
                .unwrap()
                .wait()
                .unwrap();
            assert_eq!(result.text, "foreground marker");
            foreground_wait_ms.push(wait.elapsed().as_secs_f64() * 1000.0);
            worker.join().unwrap();
        }
        foreground_wait_ms.sort_by(f64::total_cmp);
        println!(
            "foreground_recovery_wait {}",
            serde_json::json!({"model": selection.model.as_str(),
            "runs": 10, "p50_ms": foreground_wait_ms[5], "max_ms": foreground_wait_ms[9], "audio_window_s": 8})
        );
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "hex-recovery-model-{}",
            SessionId::random().unwrap().as_str()
        ));
        let client = Client::start(root.clone()).unwrap();
        let session = client.begin(selection.clone()).unwrap();
        // A barrier ensures initialization before feeding a multi-window fixture.
        assert_eq!(client.list().unwrap().len(), 1);
        for (index, chunk) in samples.chunks(16_000).enumerate() {
            session.offer((index * 16_000) as u64, chunk).unwrap();
        }
        session.seal(samples.len() as u64).unwrap();
        let mut reader = client.start_replay(session.id().clone()).unwrap();
        let started = Instant::now();
        let text = transcribe(&engine, &selection, &mut reader).unwrap();
        for keyword in ["recording", "recover", "application", "maintenance"] {
            assert!(
                text.to_lowercase().contains(keyword),
                "missing {keyword}: {text}"
            );
        }
        println!(
            "synthetic_recovery {}",
            serde_json::json!({"audio_ms": samples.len()/16,
            "elapsed_ms": started.elapsed().as_millis(), "text": text})
        );
        client
            .finish_replay(reader, selection, Ok(text.clone()))
            .unwrap();
        assert_eq!(client.text(session.id().clone(), true).unwrap(), text);
        assert_eq!(
            client.list().unwrap()[0]
                .snapshot
                .as_ref()
                .unwrap()
                .delivery,
            crate::recovery::Delivery::NotAttempted
        );
        client.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
