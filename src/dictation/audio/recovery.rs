//! Copies the existing normalized, intentional capture into recovery. No input,
//! resampler, inference, filesystem operations or waits belong here.
use super::*;
use crate::recovery::{Client, Failure, Session};

#[derive(Default)]
pub(super) struct Protection {
    pub(super) client: Option<Client>,
    enabled: bool,
    session: Option<Session>,
    sent: usize,
    gap_samples: u64,
}
impl Protection {
    pub(super) fn begin(&mut self, hotkey: bool) {
        self.session = None;
        self.sent = 0;
        self.gap_samples = 0;
        self.enabled = hotkey && crate::labs::recording_recovery();
    }
    pub(super) fn update(
        &mut self,
        capture: &DictationCapture,
        finishing: bool,
    ) -> Result<(), Failure> {
        if !self.enabled || (!finishing && !capture.is_intentional()) {
            return Ok(());
        }
        if self.session.is_none() {
            let (_, selection) = crate::app_settings::transcription_selection();
            self.session = Some(
                self.client
                    .as_ref()
                    .ok_or(Failure::Storage)?
                    .begin(selection)?,
            );
        }
        let session = self.session.as_ref().unwrap();
        if let Some(error) = session.fault() {
            return Err(error);
        }
        for chunk in capture.recorded_from(self.sent).chunks(16_000) {
            session.offer(self.sent as u64, chunk)?;
            self.sent += chunk.len();
        }
        if self.gap_samples > 0 {
            session.gap(self.gap_samples.min(16_000 * 60))?;
            self.gap_samples = 0;
        }
        Ok(())
    }
    pub(super) fn note_gap(&mut self, duration: Duration) {
        if self.enabled {
            self.gap_samples = self
                .gap_samples
                .saturating_add((duration.as_secs_f64() * 16_000.0).ceil() as u64);
        }
    }
    pub(super) fn finish(&mut self, clip: &mut DictationClip) -> Result<(), Failure> {
        if !self.enabled {
            return Ok(());
        }
        let session = self.session.take().ok_or(Failure::Storage)?;
        if let Some(error) = session.fault() {
            return Err(error);
        }
        // Already offered samples beyond a delayed release are provisional.
        // Sealing atomically limits replay, then trims the physical tail.
        for chunk in clip.samples[self.sent.min(clip.samples.len())..].chunks(16_000) {
            session.offer(self.sent as u64, chunk)?;
            self.sent += chunk.len();
        }
        session.seal(clip.samples.len() as u64)?;
        clip.recovery = Some(Box::new(session));
        self.enabled = false;
        Ok(())
    }
    pub(super) fn cancel(&mut self) {
        if let Some(session) = self.session.take()
            && session.cancel().is_err()
        {
            tracing::warn!("cancelled dictation safety-copy deletion remains pending");
        }
        self.enabled = false;
        self.sent = 0;
    }
    fn fault(&self) -> bool {
        self.session.as_ref().and_then(Session::fault).is_some()
    }
}
impl Owner {
    pub(super) fn advance_protection(&mut self, finishing: bool) -> bool {
        if self.recovery.update(&self.capture, finishing).is_err() || self.recovery.fault() {
            // Retain the saved prefix; this is a persistence failure, not an
            // explicit cancel. The abandoned session stays unresolved.
            self.recovery.enabled = false;
            self.recovery.session = None;
            self.capture.cancel();
            self.state.recording.store(false, Ordering::Release);
            let _ = self.events.send(DictationAudioEvent::ProtectionFailed);
            false
        } else {
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protection_requires_opt_in_shortcut_and_intentional_audio() {
        let mut nonce = [0_u8; 16];
        getrandom::fill(&mut nonce).unwrap();
        let nonce: String = nonce.iter().map(|byte| format!("{byte:02x}")).collect();
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("hex-capture-protection-{nonce}"));
        let client = Client::start(root.clone()).unwrap();
        for (enabled, hotkey) in [(false, true), (true, false), (true, true)] {
            let labs = crate::app_settings::LabsSettings {
                recording_recovery: enabled,
                ..Default::default()
            };
            crate::labs::apply(&labs);
            let mut protection = Protection {
                client: Some(client.clone()),
                ..Default::default()
            };
            let start = CaptureInstant::from_nanos(10_000_000_000);
            let mut capture = DictationCapture::new(16_000);
            capture.start_hotkey_at(start, Duration::ZERO);
            capture.ingest(&[0.25; 8_000], start + Duration::from_millis(500));
            protection.begin(hotkey);
            protection.update(&capture, false).unwrap();
            assert!(
                client.list().unwrap().is_empty(),
                "tentative audio stays in RAM"
            );
            assert!(capture.mark_intentional());
            protection.update(&capture, false).unwrap();
            if enabled && hotkey {
                let Finish::Transcribe(mut clip) =
                    capture.finish(start + Duration::from_millis(500))
                else {
                    panic!("intentional clip");
                };
                protection.finish(&mut clip).unwrap();
                let session = clip.take_recovery().expect("protected shortcut handle");
                let mut reader = client.reader(session.id().clone()).unwrap();
                let mut saved = Vec::new();
                while let Some(chunk) = reader.next().unwrap() {
                    saved.extend(chunk);
                }
                assert_eq!(saved, clip.samples);
                drop(reader);
                client.delete(session.id().clone()).unwrap();
            } else {
                assert!(
                    client.list().unwrap().is_empty(),
                    "ordinary/API capture is not retained"
                );
                capture.cancel();
            }
        }
        crate::labs::apply(&Default::default());
        client.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
