mod reader;
pub use reader::AudioReader;
mod actor;
mod maintenance;
#[cfg(test)]
pub(super) mod test_support;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

use super::client::{Command, Health, encode_fault};
use super::format::{self, FRAME_SAMPLES, Manifest, SEGMENT_SAMPLES, Text, VERSION};
use super::fs::Directory;
use super::{CaptureState, Delivery, Failure, SessionId, Snapshot};
use crate::transcription::models::TranscriptionSelection;

const QUOTA: u64 = 4 * 1024 * 1024 * 1024;
const RESERVE: u64 = 64 * 1024 * 1024;
const FREE: u64 = 512 * 1024 * 1024;
const SESSION_RESERVE: u64 = 256 * 1024;
const CHECKPOINT: Duration = Duration::from_secs(2);
const DAY: u64 = 24 * 60 * 60 * 1000;

#[derive(Clone, Debug, serde::Serialize)]
pub struct Entry {
    pub id: SessionId,
    pub snapshot: Option<Snapshot>,
    pub bytes: u64,
}

// A failed capture remains fenced; an explicit validated replay can still
// publish its own transcript without reopening the old capture generation.
enum TextSource {
    Capture,
    Replay(TranscriptionSelection),
}

struct Stored {
    directory: Directory,
    manifest: Option<Manifest>,
    health: Option<Arc<Health>>,
    pending: Vec<f32>,
    written: u64,
    sequence: u32,
    track: Option<(u64, File)>,
    charged: u64,
    file_count: usize,
    checkpoint: Instant,
    readers: Arc<AtomicU64>,
    read_cancel: Arc<AtomicBool>,
    blocked: bool,
    durable: Option<Manifest>,
}
pub(super) struct Store {
    root: Directory,
    lock: File,
    sessions: BTreeMap<SessionId, Stored>,
    used: u64,
    files: usize,
    changes: Arc<AtomicU64>,
}

impl Store {
    pub(super) fn open(path: PathBuf) -> io::Result<Self> {
        use fs2::FileExt;
        let root = Directory::open(&path)?;
        let lock = match root.file("store.lock", true) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                root.file("store.lock", false)?
            }
            Err(error) => return Err(error),
        };
        lock.try_lock_exclusive()?;
        root.sync(&lock)?;
        let mut store = Self {
            root,
            lock,
            sessions: BTreeMap::new(),
            used: 0,
            files: 0,
            changes: Arc::new(AtomicU64::new(1)),
        };
        // Unknown files are never opportunistically removed; inability to account
        // for a root is a failed admission, not an empty library.
        let entries = store.root.entries()?;
        for name in &entries {
            if let Some(id) = name.strip_prefix("delete-") {
                let id = SessionId::try_from(id.to_owned()).map_err(io::Error::other)?;
                loop {
                    match store.finish_delete(&id) {
                        Ok(()) => break,
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                        Err(error) => return Err(error),
                    }
                }
            }
        }
        for name in store.root.entries()? {
            if name == "store.lock" {
                continue;
            }
            let id = SessionId::try_from(name).map_err(io::Error::other)?;
            let directory = store.root.child(id.as_str(), false)?;
            let entries = directory.entries()?;
            store.files += entries.len() + 1;
            if store.files > 100_000 || store.sessions.len() >= 2_000 {
                return Err(io::Error::other("recovery metadata capacity reached"));
            }
            let file_count = entries.len() + 1;
            let mut charged = SESSION_RESERVE;
            for name in &entries {
                charged = charged.saturating_add(
                    directory
                        .file(name, false)?
                        .metadata()?
                        .len()
                        .saturating_add(4096),
                );
            }
            let manifest = directory
                .file("manifest.json", false)
                .and_then(|file| format::bounded_json::<Manifest>(file, format::MAX_MANIFEST))
                .ok()
                .filter(|manifest| manifest.version == VERSION && manifest.snapshot.id == id)
                .map(|mut manifest| {
                    // A failed publication may leave an immutable revision
                    // outside the acknowledged manifest. Never reuse its name.
                    let next = entries
                        .iter()
                        .filter_map(|name| {
                            name.strip_prefix("text-")?
                                .strip_suffix(".json")?
                                .parse::<u32>()
                                .ok()
                        })
                        .max()
                        .and_then(|revision| revision.checked_add(1));
                    if let Some(next) = next {
                        manifest.next_text = manifest.next_text.max(next);
                    }
                    manifest
                });
            store.used = store.used.saturating_add(charged);
            store.sessions.insert(
                id,
                Stored {
                    directory,
                    durable: manifest.clone(),
                    manifest,
                    health: None,
                    pending: Vec::new(),
                    written: 0,
                    sequence: 0,
                    track: None,
                    charged,
                    file_count,
                    checkpoint: Instant::now(),
                    readers: Arc::new(AtomicU64::new(0)),
                    read_cancel: Arc::new(AtomicBool::new(false)),
                    blocked: false,
                },
            );
        }
        // Never infer successful transcription or delivery from a stale state.
        let ids: Vec<_> = store.sessions.keys().cloned().collect();
        for id in ids {
            if let Some(manifest) = store
                .sessions
                .get_mut(&id)
                .and_then(|session| session.manifest.as_mut())
            {
                let mut changed = false;
                if manifest.snapshot.retrying {
                    manifest.snapshot.retrying = false;
                    manifest.snapshot.error = Some(Failure::Transcription);
                    changed = true;
                }
                if manifest.snapshot.capture == CaptureState::Recording {
                    manifest.snapshot.capture = CaptureState::Interrupted;
                    manifest.snapshot.error = Some(Failure::Interrupted);
                    changed = true;
                }
                if manifest.snapshot.delivery == Delivery::Attempting {
                    manifest.snapshot.delivery = Delivery::Unknown;
                    changed = true;
                }
                if changed {
                    store.persist(&id)?;
                }
            }
        }
        Ok(store)
    }

    pub(super) fn watch(&mut self, revision: Arc<AtomicU64>) {
        self.changes = revision;
        self.changes.fetch_add(1, Ordering::Release);
    }

    fn ensure(&self, extra: u64) -> io::Result<()> {
        if self.used.saturating_add(extra) > QUOTA
            || self.files >= 100_000
            || self.root.free_bytes()? < FREE.saturating_add(extra)
        {
            return Err(io::Error::new(
                io::ErrorKind::StorageFull,
                "recovery capacity reached",
            ));
        }
        Ok(())
    }
    fn charge(&mut self, id: &SessionId, extra: u64) -> io::Result<()> {
        if !self.sessions.contains_key(id) {
            return Err(io::Error::other("recovery session missing"));
        }
        self.ensure(extra)?;
        self.used += extra;
        self.sessions
            .get_mut(id)
            .ok_or_else(|| io::Error::other("recovery session missing"))?
            .charged += extra;
        Ok(())
    }
    fn persist(&mut self, id: &SessionId) -> io::Result<()> {
        let session = self
            .sessions
            .get_mut(id)
            .ok_or_else(|| io::Error::other("recovery session missing"))?;
        if session.blocked {
            return Err(io::Error::other("recovery commit requires reconciliation"));
        }
        let manifest = session
            .manifest
            .as_mut()
            .ok_or_else(|| io::Error::other("unreadable recovery format"))?;
        manifest.snapshot.revision = manifest
            .snapshot
            .revision
            .checked_add(1)
            .ok_or_else(|| io::Error::other("revision exhausted"))?;
        let bytes = serde_json::to_vec(manifest)?;
        if bytes.len() as u64 > format::MAX_MANIFEST {
            return Err(io::Error::other("manifest too large"));
        }
        // Leftover staging files are evidence from an interrupted commit; this
        // session is not actively mutating until startup reconciliation completed.
        match session.directory.remove("manifest.tmp", false) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let result = (|| {
            let mut staging = session.directory.file("manifest.tmp", true)?;
            staging.write_all(&bytes)?;
            super::faults::edge("manifest_written")?;
            staging.sync_all()?;
            super::faults::edge("manifest_synced")?;
            session.directory.rename("manifest.tmp", "manifest.json")?;
            super::faults::edge("manifest_renamed")?;
            session.directory.sync(&staging)?;
            super::faults::edge("manifest_ack")
        })();
        if result.is_err() {
            session.manifest = session.durable.clone();
            session.blocked = true;
            if let Some(health) = &session.health {
                health.fault.store(1, Ordering::Release);
            }
        } else {
            session.durable = session.manifest.clone();
            self.changes.fetch_add(1, Ordering::Release);
        }
        result
    }

    fn begin(
        &mut self,
        id: SessionId,
        selection: TranscriptionSelection,
        health: Arc<Health>,
    ) -> io::Result<()> {
        if health.cancelled.load(Ordering::Acquire) || health.fault.load(Ordering::Acquire) != 0 {
            return Err(io::Error::other("capture admission cancelled"));
        }
        self.ensure(RESERVE + SESSION_RESERVE)?;
        if self.sessions.len() >= 2_000 || self.files > 99_998 {
            return Err(io::Error::new(io::ErrorKind::StorageFull, "session limit"));
        }
        let directory = self.root.child(id.as_str(), true)?;
        let snapshot = Snapshot {
            id: id.clone(),
            created_ms: crate::desktop::events::now_ms(),
            revision: 0,
            capture: CaptureState::Recording,
            samples: 0,
            selection,
            delivery: Delivery::NotAttempted,
            raw_revision: None,
            final_revision: None,
            resolved_ms: None,
            keep: false,
            retrying: false,
            deleting: false,
            user_resolved: false,
            gap_count: 0,
            audio_removed: false,
            error: None,
        };
        self.sessions.insert(
            id.clone(),
            Stored {
                directory,
                durable: None,
                manifest: Some(Manifest {
                    version: VERSION,
                    snapshot,
                    next_text: 0,
                }),
                health: Some(health.clone()),
                pending: Vec::with_capacity(FRAME_SAMPLES),
                written: 0,
                sequence: 0,
                track: None,
                charged: SESSION_RESERVE,
                file_count: 2,
                checkpoint: Instant::now(),
                readers: Arc::new(AtomicU64::new(0)),
                read_cancel: Arc::new(AtomicBool::new(false)),
                blocked: false,
            },
        );
        self.used += SESSION_RESERVE;
        self.files += 2;
        self.persist(&id)?;
        self.root.sync(&self.lock)?;
        health.ready.store(true, Ordering::Release);
        Ok(())
    }
    fn audio(&mut self, id: &SessionId, start: u64, samples: Vec<f32>) -> io::Result<()> {
        let session = self
            .sessions
            .get_mut(id)
            .ok_or_else(|| io::Error::other("session missing"))?;
        if session.blocked
            || session
                .health
                .as_ref()
                .is_none_or(|health| health.fault.load(Ordering::Acquire) != 0)
            || session
                .manifest
                .as_ref()
                .is_none_or(|manifest| manifest.snapshot.capture != CaptureState::Recording)
            || start != session.written + session.pending.len() as u64
            || samples.len() > FRAME_SAMPLES
        {
            return Err(io::Error::other("invalid recording generation or offset"));
        }
        session.pending.extend(samples);
        if session.pending.len() >= FRAME_SAMPLES {
            self.flush_audio(id, false)?;
        }
        Ok(())
    }
    fn flush_audio(&mut self, id: &SessionId, all: bool) -> io::Result<()> {
        let session = self
            .sessions
            .get(id)
            .ok_or_else(|| io::Error::other("session missing"))?;
        if session.blocked
            || session
                .manifest
                .as_ref()
                .is_none_or(|manifest| manifest.snapshot.capture != CaptureState::Recording)
        {
            return Err(io::Error::other("capture writes fenced"));
        }
        let pending = std::mem::take(
            &mut self
                .sessions
                .get_mut(id)
                .ok_or_else(|| io::Error::other("session missing"))?
                .pending,
        );
        let mut remaining = pending.as_slice();
        while remaining.len() >= FRAME_SAMPLES || (all && !remaining.is_empty()) {
            let written = self.sessions[id].written;
            let take = remaining
                .len()
                .min(FRAME_SAMPLES)
                .min((SEGMENT_SAMPLES - written % SEGMENT_SAMPLES) as usize);
            let chunk = &remaining[..take];
            let charge = chunk.len() as u64 * 4 + 4096;
            self.ensure(charge + RESERVE)?;
            self.charge(id, charge)?;
            let session = self.sessions.get_mut(id).unwrap();
            let segment = session.written / SEGMENT_SAMPLES;
            if session
                .track
                .as_ref()
                .is_none_or(|(current, _)| *current != segment)
            {
                if let Some((_, file)) = session.track.take() {
                    file.sync_all()?;
                }
                let file = session.directory.file(&audio_name(segment), true)?;
                self.files += 1;
                session.file_count += 1;
                session.track = Some((segment, file));
            }
            format::write_frame(
                &mut session.track.as_mut().unwrap().1,
                session.sequence,
                session.written,
                chunk,
            )?;
            super::faults::edge("audio_written")?;
            session.written += chunk.len() as u64;
            session.sequence = session
                .sequence
                .checked_add(1)
                .ok_or_else(|| io::Error::other("frame sequence exhausted"))?;
            remaining = &remaining[take..];
        }
        self.sessions
            .get_mut(id)
            .unwrap()
            .pending
            .extend_from_slice(remaining);
        Ok(())
    }
    fn gap(&mut self, id: &SessionId, missing: u64) -> io::Result<()> {
        self.flush_audio(id, true)?;
        self.charge(id, 4096)?;
        let session = self
            .sessions
            .get_mut(id)
            .ok_or_else(|| io::Error::other("session missing"))?;
        if session.blocked {
            return Err(io::Error::other("session fenced"));
        }
        let segment = session.written / SEGMENT_SAMPLES;
        if session
            .track
            .as_ref()
            .is_none_or(|(current, _)| *current != segment)
        {
            if let Some((_, file)) = session.track.take() {
                file.sync_all()?;
            }
            session.track = Some((segment, session.directory.file(&audio_name(segment), true)?));
            self.files += 1;
            session.file_count += 1;
        }
        format::write_gap(
            &mut session.track.as_mut().unwrap().1,
            session.sequence,
            session.written,
            missing,
        )?;
        session.sequence = session
            .sequence
            .checked_add(1)
            .ok_or_else(|| io::Error::other("sequence exhausted"))?;
        let snapshot = &mut session.manifest.as_mut().unwrap().snapshot;
        snapshot.gap_count = snapshot.gap_count.saturating_add(1);
        snapshot.error = Some(Failure::Interrupted);
        Ok(())
    }
    fn checkpoint(&mut self, id: &SessionId, final_samples: Option<u64>) -> io::Result<()> {
        self.flush_audio(id, true)?;
        let session = self.sessions.get_mut(id).unwrap();
        if let Some((_, file)) = &session.track {
            file.sync_all()?;
        }
        super::faults::edge("audio_synced")?;
        let samples = final_samples.unwrap_or(session.written);
        if samples > session.written {
            return Err(io::Error::other("seal exceeds saved audio"));
        }
        let manifest = session.manifest.as_mut().unwrap();
        manifest.snapshot.samples = samples;
        if final_samples.is_some() {
            manifest.snapshot.capture = CaptureState::Sealed;
        }
        self.persist(id)?;
        if let Some(limit) = final_samples {
            self.trim_sealed_tail(id, limit)?;
        }
        let session = self.sessions.get_mut(id).unwrap();
        session.checkpoint = Instant::now();
        if let Some(health) = &session.health {
            health.acknowledged_ms.store(
                health.started.elapsed().as_millis() as u64 + 1,
                Ordering::Release,
            );
        }
        Ok(())
    }
    fn text(
        &mut self,
        id: &SessionId,
        text: String,
        raw: bool,
        intent: bool,
        source: TextSource,
    ) -> io::Result<()> {
        if text.len() > format::MAX_TEXT {
            return Err(io::Error::other("recovery text exceeds limit"));
        }
        self.charge(
            id,
            (text.len() as u64)
                .saturating_mul(6)
                .saturating_add(64 * 1024),
        )?;
        let session = self
            .sessions
            .get_mut(id)
            .ok_or_else(|| io::Error::other("session missing"))?;
        let capture_fault = session
            .health
            .as_ref()
            .is_some_and(|health| health.fault.load(Ordering::Acquire) != 0);
        if session.blocked || (matches!(&source, TextSource::Capture) && capture_fault) {
            return Err(io::Error::other("capture persistence failed"));
        }
        let manifest = session
            .manifest
            .as_mut()
            .ok_or_else(|| io::Error::other("unreadable session"))?;
        if manifest.snapshot.capture != CaptureState::Sealed {
            return Err(io::Error::other("capture not sealed"));
        }
        let revision = manifest.next_text;
        manifest.next_text = revision
            .checked_add(1)
            .ok_or_else(|| io::Error::other("text revision exhausted"))?;
        let text = Text {
            version: VERSION,
            session: id.clone(),
            revision,
            raw,
            samples: manifest.snapshot.samples,
            checksum: <sha2::Sha256 as sha2::Digest>::digest(text.as_bytes()).into(),
            text,
            selection: match source {
                TextSource::Capture => manifest.snapshot.selection.clone(),
                TextSource::Replay(selection) => selection,
            },
        };
        let mut file = session.directory.file(&text_name(revision), true)?;
        self.files += 1;
        session.file_count += 1;
        {
            let mut buffered = std::io::BufWriter::with_capacity(64 * 1024, &mut file);
            serde_json::to_writer(&mut buffered, &text)?;
            buffered.flush()?;
        }
        super::faults::edge("text_written")?;
        file.sync_all()?;
        super::faults::edge("text_synced")?;
        // fcntl(2): a full device barrier also persists earlier fsync'd
        // writes on this device. The final manifest commit supplies it.
        session.directory.sync_entries()?;
        super::faults::edge("text_published")?;
        if raw {
            manifest.snapshot.raw_revision = Some(revision);
        } else {
            manifest.snapshot.final_revision = Some(revision);
        }
        if intent {
            manifest.snapshot.delivery = if text.text.is_empty() {
                Delivery::Suppressed
            } else {
                Delivery::Attempting
            };
        }
        if raw && text.text.trim().is_empty() {
            manifest.snapshot.error = Some(Failure::EmptyResult);
        }
        self.persist(id)
    }
    fn read_text(&self, id: &SessionId, raw: bool) -> io::Result<String> {
        let session = self
            .sessions
            .get(id)
            .ok_or_else(|| io::Error::other("session missing"))?;
        let manifest = session
            .manifest
            .as_ref()
            .ok_or_else(|| io::Error::other("unreadable session"))?;
        let revision = if raw {
            manifest.snapshot.raw_revision
        } else {
            manifest.snapshot.final_revision
        }
        .ok_or_else(|| io::Error::other("text unavailable"))?;
        let text: Text = format::bounded_json(
            session.directory.file(&text_name(revision), false)?,
            format::MAX_TEXT as u64 * 6 + 4096,
        )?;
        if text.version != VERSION
            || text.session != *id
            || text.revision != revision
            || text.raw != raw
            || text.text.len() > format::MAX_TEXT
            || text.checksum
                != <sha2::Sha256 as sha2::Digest>::digest(text.text.as_bytes()).as_slice()
        {
            return Err(io::Error::other("invalid saved transcript"));
        }
        Ok(text.text)
    }
    fn reader(&self, id: &SessionId) -> io::Result<AudioReader> {
        let session = self
            .sessions
            .get(id)
            .ok_or_else(|| io::Error::other("session missing"))?;
        let manifest = session
            .manifest
            .as_ref()
            .ok_or_else(|| io::Error::other("unreadable session"))?;
        if session.blocked
            || session
                .health
                .as_ref()
                .is_some_and(|_| manifest.snapshot.capture == CaptureState::Recording)
            || manifest.snapshot.audio_removed
        {
            return Err(io::Error::other(
                "audio unavailable while recording or expired",
            ));
        }
        let directory = session.directory.child_self()?;
        session.readers.fetch_add(1, Ordering::AcqRel);
        Ok(AudioReader {
            directory,
            id: id.clone(),
            revision: manifest.snapshot.revision,
            selection: manifest.snapshot.selection.clone(),
            complete: false,
            readers: session.readers.clone(),
            read_cancel: session.read_cancel.clone(),
            limit: if manifest.snapshot.capture == CaptureState::Interrupted {
                QUOTA / 4
            } else {
                manifest.snapshot.samples
            },
            acknowledged: manifest.snapshot.samples,
            gaps: 0,
            salvage: manifest.snapshot.capture == CaptureState::Interrupted,
            at: 0,
            sequence: 0,
            segment: None,
        })
    }
    fn delete(&mut self, id: &SessionId) -> io::Result<()> {
        if !self.sessions.contains_key(id) {
            return Ok(());
        }
        if let Some(session) = self.sessions.get(id) {
            if session
                .manifest
                .as_ref()
                .is_some_and(|manifest| manifest.snapshot.deleting)
                && session.readers.load(Ordering::Acquire) != 0
            {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "deletion waiting for active audio",
                ));
            }
            if let Some(health) = &session.health {
                health.fault.store(1, Ordering::Release);
            }
        }
        let marker = format!("delete-{}", id.as_str());
        let file = match self.root.file(&marker, true) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                self.root.file(&marker, false)?
            }
            Err(error) => return Err(error),
        };
        file.sync_all()?;
        super::faults::edge("tombstone_synced")?;
        self.root.sync(&file)?;
        super::faults::edge("delete_ack")?;
        if let Some(session) = self.sessions.get_mut(id) {
            session.blocked = true;
            session.read_cancel.store(true, Ordering::Release);
            if let Some(manifest) = &mut session.manifest {
                manifest.snapshot.deleting = true;
            }
            self.changes.fetch_add(1, Ordering::Release);
            if session.readers.load(Ordering::Acquire) != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "deletion waiting for audio reader",
                ));
            }
        }
        self.finish_delete(id)?;
        if let Some(session) = self.sessions.remove(id) {
            self.used = self.used.saturating_sub(session.charged);
            self.files = self.files.saturating_sub(session.file_count);
        }
        self.changes.fetch_add(1, Ordering::Release);
        Ok(())
    }
    fn finish_delete(&mut self, id: &SessionId) -> io::Result<()> {
        let marker = format!("delete-{}", id.as_str());
        let barrier = self.root.file(&marker, false)?;
        match self.root.child(id.as_str(), false) {
            Ok(directory) => {
                let names = directory.entries()?;
                let more = names.len() > 64;
                for name in names.into_iter().take(64) {
                    directory.remove(&name, false)?;
                }
                directory.sync(&barrier)?;
                if more {
                    return Err(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "deletion continuing",
                    ));
                }
                super::faults::edge("files_removed")?;
                self.root.remove(id.as_str(), true)?;
                self.root.sync(&barrier)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        super::faults::edge("directory_removed")?;
        self.root.remove(&marker, false)?;
        self.root.sync(&barrier)?;
        super::faults::edge("delete_complete")
    }
    fn fault(&mut self, id: &SessionId, failure: Failure) {
        if let Some(session) = self.sessions.get_mut(id) {
            if let Some(health) = &session.health {
                health.fault.store(encode_fault(failure), Ordering::Release);
            }
            if let Some(manifest) = &mut session.manifest {
                manifest.snapshot.error = Some(failure);
                if manifest.snapshot.capture == CaptureState::Recording {
                    manifest.snapshot.capture = CaptureState::Interrupted;
                }
            }
            // This is best effort only. An earlier checkpoint remains the
            // recovery authority if disk failure prevents terminal metadata.
            let _ = self.persist(id);
        }
    }
}

fn classify(error: &io::Error) -> Failure {
    if error.kind() == io::ErrorKind::WouldBlock {
        Failure::Busy
    } else if error.kind() == io::ErrorKind::StorageFull {
        Failure::Capacity
    } else {
        Failure::Storage
    }
}
fn audio_name(segment: u64) -> String {
    format!("audio-{segment:08}.pcm")
}
fn text_name(revision: u32) -> String {
    format!("text-{revision:08}.json")
}
