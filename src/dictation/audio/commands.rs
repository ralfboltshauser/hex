//! Audio-owner command transitions. The capture timeline remains in its owner.
use super::*;

impl Owner {
    pub(super) fn handle(&mut self, command: Command) -> bool {
        match command {
            Command::ConfigureRecovery(client) => self.recovery.client = Some(client),
            Command::Start { at, kind, reply } => {
                // Draining can discover a failure belonging to the previous capture or idle
                // period. Validate and advance the new capture only after that drain.
                if self.input.is_open() && !self.input.is_recovering() {
                    self.drain_through(at);
                }
                if self.input.is_recovering() {
                    let _ = reply.send(false);
                    return true;
                }
                self.state.capture_generation.fetch_add(1, Ordering::AcqRel);
                if self.input.is_open() {
                    self.start_capture_at(kind, at);
                } else {
                    self.pending_capture = Some(PendingCapture {
                        at,
                        kind,
                        intentional_at: None,
                        _capture: CaptureMarker::new(),
                    });
                    self.input.request_open();
                }
                self.state.recording.store(true, Ordering::Release);
                let _ = reply.send(true);
            }
            Command::BecomeIntentional { at, reply } => {
                self.drain_through(at);
                let became_intentional = if let Some(pending) = &mut self.pending_capture {
                    pending.mark_intentional(at);
                    false
                } else {
                    self.capture.become_intentional(at)
                };
                let _ = reply.send(became_intentional);
            }
            Command::MarkIntentional { reply } => {
                let became_intentional = if let Some(pending) = &mut self.pending_capture {
                    pending.mark_intentional(pending.at + crate::dictation::MINIMUM_HOLD_DURATION);
                    false
                } else {
                    self.capture.mark_intentional()
                };
                let _ = reply.send(became_intentional);
            }
            Command::Finish { at, tail, reply } => {
                self.drain_through(at);
                let announced = self.pending_capture.is_none()
                    && !tail.is_zero()
                    && self.capture.would_transcribe(at);
                if announced {
                    // A release that will transcribe sounds at once, not after
                    // its tail, and a muted output is restored first.
                    self.capture
                        .end_recording_environment_then(feedback::deferred_stop());
                }
                let (tail, deferred) = if announced {
                    self.drain_tail(at, feedback::stop_tone_length())
                } else {
                    (tail, None)
                };
                if self.capture.would_transcribe(at) {
                    self.advance_protection(true);
                }
                let mut finish = if self.pending_capture.take().is_some() {
                    if !self.capture_sink_open() {
                        self.input.cancel_open();
                    }
                    Finish::Discard
                } else {
                    self.capture.finish_with_tail(at, tail)
                };
                if let Finish::Transcribe(clip) = &mut finish {
                    if self.recovery.finish(clip).is_err() {
                        self.capture.cancel();
                        self.state.recording.store(false, Ordering::Release);
                        let _ = self.events.send(DictationAudioEvent::ProtectionFailed);
                        let _ = reply.send(Finish::Discard);
                        return true;
                    }
                    clip.set_source(self.clip_source());
                    if clip.is_digital_silence() {
                        self.reopen_silent_stream();
                    }
                }
                self.state.recording.store(false, Ordering::Release);
                let _ = reply.send(finish);
                if let Some(event) = deferred {
                    self.handle_input(event);
                }
            }
            Command::Cancel { reply } => {
                self.pending_capture = None;
                if !self.input.is_open() && !self.capture_sink_open() {
                    self.input.cancel_open();
                }
                self.recovery.cancel();
                self.capture.cancel();
                self.state.recording.store(false, Ordering::Release);
                let _ = reply.send(());
            }
            Command::SelectMicrophone { revision, device } => {
                self.input.request_selection(revision, device.as_deref());
            }
            Command::SetReleaseWhileIdle(release) => {
                self.release_while_idle = release;
            }
            #[cfg(test)]
            Command::InvalidateRecognition { reply } => {
                let _ = reply.send(self.next_recognition_generation());
            }
            #[cfg(hex_capture)]
            Command::OpenCaptureSink { sink, since, reply } => {
                self.capture_sink = Some(CaptureSink::new(sink, since));
                if !self.input.is_active() {
                    self.input.request_open();
                }
                let _ = reply.send(());
            }
            #[cfg(hex_capture)]
            Command::CloseCaptureSink { through, reply } => {
                if self.capture_sink.is_some() && self.input.is_open() {
                    self.drain_through(through);
                }
                let _ = reply.send(self.capture_sink.take().map(CaptureSink::close));
            }
            Command::Shutdown => return false,
        }
        true
    }
}
