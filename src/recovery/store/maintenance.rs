//! One retention policy and bounded, deadline-driven reclamation.
use super::*;

impl Store {
    pub(super) fn next_wait(&self, now_ms: u64) -> Option<Duration> {
        self.sessions
            .values()
            .filter_map(|session| {
                if session
                    .health
                    .as_ref()
                    .is_some_and(|health| health.cancelled.load(Ordering::Acquire))
                {
                    return Some(Duration::from_secs(1));
                }
                let snapshot = &session.manifest.as_ref()?.snapshot;
                if snapshot.deleting {
                    return Some(Duration::from_secs(1));
                }
                if snapshot.capture == CaptureState::Recording {
                    return Some(CHECKPOINT.saturating_sub(session.checkpoint.elapsed()));
                }
                let resolved = snapshot.resolved_ms?;
                if !resolvable(snapshot) || now_ms < resolved {
                    return None;
                }
                let window = if snapshot.audio_removed {
                    DAY
                } else {
                    DAY / 24
                };
                Some(
                    Duration::from_millis(resolved.saturating_add(window).saturating_sub(now_ms))
                        .max(Duration::from_secs(1)),
                )
            })
            .min()
    }

    pub(super) fn maintenance(&mut self) {
        let now = crate::desktop::events::now_ms();
        let due: Vec<_> = self
            .sessions
            .iter()
            .filter_map(|(id, session)| {
                if session
                    .health
                    .as_ref()
                    .is_some_and(|health| health.cancelled.load(Ordering::Acquire))
                {
                    return Some(id.clone());
                }
                let snapshot = &session.manifest.as_ref()?.snapshot;
                if snapshot.deleting {
                    return Some(id.clone());
                }
                if snapshot.capture == CaptureState::Recording
                    && session.checkpoint.elapsed() >= CHECKPOINT
                {
                    return Some(id.clone());
                }
                if eligible(snapshot, now, DAY / 24) {
                    Some(id.clone())
                } else {
                    None
                }
            })
            .take(8)
            .collect();
        for id in due {
            if self.sessions[&id]
                .health
                .as_ref()
                .is_some_and(|health| health.cancelled.load(Ordering::Acquire))
            {
                if self.delete(&id).is_err() {
                    tracing::warn!(
                        session = id.as_str(),
                        "cancelled safety-copy deletion remains pending"
                    );
                }
                continue;
            }
            if self.sessions[&id]
                .manifest
                .as_ref()
                .is_some_and(|manifest| manifest.snapshot.deleting)
            {
                let _ = self.delete(&id);
                continue;
            }
            let snapshot = self.sessions[&id]
                .manifest
                .as_ref()
                .unwrap()
                .snapshot
                .clone();
            if snapshot.capture == CaptureState::Recording {
                let fault = self.sessions[&id].health.as_ref().and_then(|health| {
                    let value = health.fault.load(Ordering::Acquire);
                    (value != 0).then_some(Failure::Storage)
                });
                if let Some(fault) = fault {
                    self.fault(&id, fault);
                } else if let Err(error) = self.checkpoint(&id, None) {
                    self.fault(&id, classify(&error));
                }
                continue;
            }
            if self.sessions[&id].readers.load(Ordering::Acquire) != 0 {
                continue;
            }
            if !snapshot.user_resolved
                && (self.read_text(&id, true).is_err() || self.read_text(&id, false).is_err())
            {
                self.fault(&id, Failure::Storage);
                continue;
            }
            if eligible(&snapshot, now, DAY) {
                if self.delete(&id).is_err() {
                    // A failed explicit deletion stays visible and retries on a
                    // later deadline; never turn it into successful reclamation.
                    tracing::warn!(session = id.as_str(), "recovery cleanup remains pending");
                }
            } else if !snapshot.audio_removed && self.remove_audio(&id).is_err() {
                tracing::warn!(
                    session = id.as_str(),
                    "recovery audio cleanup remains pending"
                );
            }
        }
    }

    fn remove_audio(&mut self, id: &SessionId) -> io::Result<()> {
        let session = self
            .sessions
            .get_mut(id)
            .ok_or_else(|| io::Error::other("session missing"))?;
        if session.readers.load(Ordering::Acquire) != 0 {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "audio reader active",
            ));
        }
        // Automatic expiry requires readable text; otherwise a corrupt text
        // reference must never destroy the remaining recoverable audio.
        if !session.manifest.as_ref().unwrap().snapshot.user_resolved {
            self.read_text(id, true)?;
            self.read_text(id, false)?;
        }
        let session = self.sessions.get_mut(id).unwrap();
        session.track = None;
        for name in session.directory.entries()? {
            if audio_segment(&name).is_some() {
                session.directory.remove(&name, false)?;
            }
        }
        session.directory.sync(&self.lock)?;
        session.manifest.as_mut().unwrap().snapshot.audio_removed = true;
        self.persist(id)?;
        self.recount(id)
    }

    pub(super) fn recount(&mut self, id: &SessionId) -> io::Result<()> {
        let session = self
            .sessions
            .get_mut(id)
            .ok_or_else(|| io::Error::other("session missing"))?;
        let mut charged = SESSION_RESERVE;
        let entries = session.directory.entries()?;
        let file_count = entries.len() + 1;
        for name in entries {
            charged = charged.saturating_add(
                session
                    .directory
                    .file(&name, false)?
                    .metadata()?
                    .len()
                    .saturating_add(4096),
            );
        }
        self.used = self
            .used
            .saturating_sub(session.charged)
            .saturating_add(charged);
        session.charged = charged;
        self.files = self
            .files
            .saturating_sub(session.file_count)
            .saturating_add(file_count);
        session.file_count = file_count;
        Ok(())
    }

    pub(super) fn trim_sealed_tail(&mut self, id: &SessionId, limit: u64) -> io::Result<()> {
        let written = self.sessions[id].written;
        if written <= limit {
            return Ok(());
        }
        // At most one 30-second segment is rewritten. The final extent is
        // already durable, so readers are safe before/after this replacement.
        self.ensure(SEGMENT_SAMPLES * 4 + 64 * 1024)?;
        let session = self.sessions.get_mut(id).unwrap();
        session.track = None;
        let last = limit.saturating_sub(1) / SEGMENT_SAMPLES;
        if limit > 0 {
            let name = audio_name(last);
            let mut source = session.directory.file(&name, false)?;
            match session.directory.remove("seal.tmp", false) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            let mut staging = session.directory.file("seal.tmp", true)?;
            while let Some(mut frame) = format::read_frame(&mut source)? {
                if frame.start >= limit {
                    break;
                }
                frame
                    .samples
                    .truncate((limit - frame.start).min(frame.samples.len() as u64) as usize);
                if let Some(missing) = frame.missing {
                    format::write_gap(&mut staging, frame.sequence, frame.start, missing)?;
                } else {
                    format::write_frame(&mut staging, frame.sequence, frame.start, &frame.samples)?;
                }
            }
            staging.sync_all()?;
            session.directory.rename("seal.tmp", &name)?;
            session.directory.sync(&staging)?;
        }
        for name in session.directory.entries()? {
            if audio_segment(&name).is_some_and(|segment| limit == 0 || segment > last) {
                session.directory.remove(&name, false)?;
            }
        }
        session.directory.sync(&self.lock)?;
        self.recount(id)
    }
}

fn resolvable(snapshot: &Snapshot) -> bool {
    snapshot.capture != CaptureState::Recording
        && !snapshot.keep
        && !snapshot.retrying
        && !snapshot.deleting
        && (snapshot.user_resolved
            || (snapshot.error.is_none()
                && matches!(
                    snapshot.delivery,
                    Delivery::ReportedSuccess | Delivery::Suppressed
                )))
}
fn eligible(snapshot: &Snapshot, now: u64, window: u64) -> bool {
    resolvable(snapshot)
        && snapshot
            .resolved_ms
            .is_some_and(|at| now >= at && now - at >= window)
}
fn audio_segment(name: &str) -> Option<u64> {
    let number = name.strip_prefix("audio-")?.strip_suffix(".pcm")?;
    if number.len() != 8 || !number.bytes().all(|value| value.is_ascii_digit()) {
        return None;
    }
    number.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unresolved_failures_and_running_retries_never_expire() {
        let mut snapshot = Snapshot {
            id: SessionId::random().unwrap(),
            created_ms: 0,
            revision: 1,
            capture: CaptureState::Sealed,
            samples: 100,
            selection: Default::default(),
            delivery: Delivery::ReportedSuccess,
            raw_revision: Some(0),
            final_revision: Some(1),
            resolved_ms: Some(1),
            keep: false,
            retrying: false,
            deleting: false,
            user_resolved: false,
            gap_count: 0,
            audio_removed: false,
            error: None,
        };
        assert!(eligible(&snapshot, DAY * 2, DAY));
        snapshot.error = Some(Failure::Interrupted);
        assert!(!eligible(&snapshot, DAY * 2, DAY));
        snapshot.user_resolved = true;
        assert!(eligible(&snapshot, DAY * 2, DAY));
        snapshot.retrying = true;
        assert!(!eligible(&snapshot, DAY * 2, DAY));
        snapshot.retrying = false;
        snapshot.keep = true;
        assert!(!eligible(&snapshot, DAY * 2, DAY));
        snapshot.keep = false;
        assert!(!eligible(&snapshot, 0, DAY));
    }
}
