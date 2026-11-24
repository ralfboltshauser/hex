//! One retention policy and bounded, deadline-driven reclamation.
use super::*;

impl Store {
    pub(super) fn next_wait(&self, now_ms: u64) -> Option<Duration> {
        self.sessions
            .values()
            .filter_map(|session| {
                if cancelled(session) {
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
                Some(
                    Duration::from_millis(expiry_at(snapshot, resolved).saturating_sub(now_ms))
                        .max(Duration::from_secs(1)),
                )
            })
            .min()
    }

    pub(super) fn maintenance(&mut self) {
        let now = crate::desktop::events::now_ms();
        let tick = Instant::now();
        // Cleanup batches must yield to recordings with an outstanding checkpoint.
        let checkpoint_due = |session: &Stored| {
            tick.duration_since(session.checkpoint) >= CHECKPOINT
                && session.manifest.as_ref().is_some_and(|manifest| {
                    !manifest.snapshot.deleting
                        && manifest.snapshot.capture == CaptureState::Recording
                })
        };
        let checkpoints = self
            .sessions
            .iter()
            .filter(|(_, session)| checkpoint_due(session));
        let cleanup = self.sessions.iter().filter_map(|(id, session)| {
            if cancelled(session) {
                return (!checkpoint_due(session)).then_some(id);
            }
            let snapshot = &session.manifest.as_ref()?.snapshot;
            (snapshot.deleting
                || (resolvable(snapshot)
                    && snapshot
                        .resolved_ms
                        .is_some_and(|resolved| now >= expiry_at(snapshot, resolved))))
            .then_some(id)
        });
        let due: Vec<_> = checkpoints
            .map(|(id, _)| id)
            .chain(cleanup)
            .take(8)
            .cloned()
            .collect();
        for id in due {
            if cancelled(&self.sessions[&id]) {
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

fn cancelled(session: &Stored) -> bool {
    session
        .health
        .as_ref()
        .is_some_and(|health| health.cancelled.load(Ordering::Acquire))
}

fn expiry_at(snapshot: &Snapshot, resolved: u64) -> u64 {
    let window = if snapshot.audio_removed {
        DAY
    } else {
        DAY / 24
    };
    resolved.saturating_add(window)
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
    fn expired_audio_does_not_starve_recording_checkpoints() {
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "hex-recovery-expiry-{}",
            SessionId::random().unwrap().as_str()
        ));
        let result = {
            let mut store = Store::open(root.clone()).unwrap();
            let now = crate::desktop::events::now_ms();
            let mut begin = |index| {
                let id = SessionId::try_from(format!("{index:032x}")).unwrap();
                let health = Arc::new(Health {
                    started: Instant::now(),
                    ready: AtomicBool::new(false),
                    cancelled: AtomicBool::new(false),
                    fault: std::sync::atomic::AtomicU8::new(0),
                    acknowledged_ms: AtomicU64::new(1),
                    pending: Default::default(),
                });
                store.begin(id.clone(), Default::default(), health).unwrap();
                id
            };
            let expired: Vec<_> = (0..8).map(&mut begin).collect();
            let active = begin(8);
            let due_audio = begin(9);
            for id in expired.iter().chain([&due_audio]) {
                store.audio(id, 0, vec![0.25; 16]).unwrap();
                store.checkpoint(id, Some(16)).unwrap();
                let snapshot = &mut store
                    .sessions
                    .get_mut(id)
                    .unwrap()
                    .manifest
                    .as_mut()
                    .unwrap()
                    .snapshot;
                snapshot.user_resolved = true;
                snapshot.resolved_ms = Some(now - DAY / 12);
                if id != &due_audio {
                    store.remove_audio(id).unwrap();
                }
            }
            store.audio(&active, 0, vec![0.25; 16]).unwrap();
            store.sessions.get_mut(&active).unwrap().checkpoint = Instant::now() - CHECKPOINT;
            store.maintenance();
            let first = store.sessions[&active]
                .manifest
                .as_ref()
                .unwrap()
                .snapshot
                .samples;
            let retained_before_expiry = expired.iter().all(|id| store.sessions.contains_key(id));
            let reclaimed_due_audio = store.sessions[&due_audio]
                .manifest
                .as_ref()
                .unwrap()
                .snapshot
                .audio_removed;

            // Even genuine text expiry must yield to a due recording checkpoint.
            for id in &expired {
                store
                    .sessions
                    .get_mut(id)
                    .unwrap()
                    .manifest
                    .as_mut()
                    .unwrap()
                    .snapshot
                    .resolved_ms = Some(now - DAY);
            }
            store.audio(&active, 16, vec![0.25; 16]).unwrap();
            store.sessions.get_mut(&active).unwrap().checkpoint = Instant::now() - CHECKPOINT;
            store.maintenance();
            let second = store.sessions[&active]
                .manifest
                .as_ref()
                .unwrap()
                .snapshot
                .samples;
            store.maintenance();
            let removed_after_expiry = expired.iter().all(|id| !store.sessions.contains_key(id));
            (
                first,
                retained_before_expiry,
                reclaimed_due_audio,
                second,
                removed_after_expiry,
            )
        };
        std::fs::remove_dir_all(root).unwrap();
        assert_eq!(result, (16, true, true, 32, true));
    }

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
