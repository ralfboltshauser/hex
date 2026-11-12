//! Nonblocking capture sink and bounded requests to the single disk owner.
use super::format::FRAME_SAMPLES;
use super::store::{AudioReader, Entry};
use super::{Delivery, Failure, SessionId};
use crate::transcription::models::TranscriptionSelection;
use std::collections::VecDeque;
use std::io;
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    mpsc,
};
use std::thread;
use std::time::{Duration, Instant};
const TIMEOUT: Duration = Duration::from_secs(3);
const STALE: u64 = 5_000;

#[derive(Clone)]
pub struct Client(Arc<Connection>);
struct Connection {
    commands: mpsc::SyncSender<Command>,
    closed: AtomicBool,
    retrying: Arc<AtomicBool>,
    revision: Arc<AtomicU64>,
}
pub(super) struct Health {
    pub(super) started: Instant,
    pub(super) ready: AtomicBool,
    pub(super) cancelled: AtomicBool,
    pub(super) fault: AtomicU8,
    pub(super) acknowledged_ms: AtomicU64,
    pub(super) pending: Mutex<VecDeque<u64>>,
}
#[derive(Clone)]
pub struct Session {
    client: Client,
    id: SessionId,
    health: Arc<Health>,
}

pub(super) type Reply<T> = mpsc::SyncSender<Result<T, Failure>>;
pub(super) enum Command {
    Begin {
        id: SessionId,
        selection: TranscriptionSelection,
        health: Arc<Health>,
    },
    Audio {
        id: SessionId,
        start: u64,
        samples: Vec<f32>,
    },
    Gap {
        id: SessionId,
        missing: u64,
    },
    Seal {
        id: SessionId,
        samples: u64,
    },
    Text {
        id: SessionId,
        text: String,
        raw: bool,
        intent: bool,
        reply: Reply<()>,
    },
    Outcome {
        id: SessionId,
        delivery: Delivery,
        reply: Reply<()>,
    },
    ReplayStart {
        id: SessionId,
        reply: Reply<AudioReader>,
    },
    ReplayFinish {
        reader: AudioReader,
        selection: TranscriptionSelection,
        result: Result<String, Failure>,
        reply: Reply<()>,
    },
    List(Reply<Vec<Entry>>),
    ReadText {
        id: SessionId,
        raw: bool,
        reply: Reply<String>,
    },
    Reader {
        id: SessionId,
        reply: Reply<AudioReader>,
    },
    Resolve {
        id: SessionId,
        keep: bool,
        reply: Reply<()>,
    },
    Delete {
        id: SessionId,
        reply: Reply<()>,
    },
    Stop(Reply<()>),
}

impl Client {
    /// Setup and reconciliation happen off the caller's thread. Begin remains
    /// unacknowledged until the root lock, quota scan and admission are durable.
    pub fn start(root: PathBuf) -> io::Result<Self> {
        let (commands, receiver) = mpsc::sync_channel(64);
        let client = Self(Arc::new(Connection {
            commands,
            closed: AtomicBool::new(false),
            retrying: Arc::new(AtomicBool::new(false)),
            revision: Arc::new(AtomicU64::new(1)),
        }));
        let revision = client.0.revision.clone();
        thread::Builder::new()
            .name("hex-recovery-store".into())
            .spawn(move || {
                match super::store::Store::open(root) {
                    Ok(mut store) => {
                        store.watch(revision);
                        store.run(receiver);
                    }
                    Err(_) => {
                        // Keep replying deterministically instead of leaving admitted
                        // callers waiting on a worker which failed setup.
                        while let Ok(command) = receiver.recv() {
                            let stopping = matches!(&command, Command::Stop(_));
                            command.fail(Failure::Storage);
                            if stopping {
                                break;
                            }
                        }
                    }
                }
            })?;
        Ok(client)
    }

    fn send(&self, command: Command) -> Result<(), Failure> {
        if self.0.closed.load(Ordering::Acquire) {
            return Err(Failure::Storage);
        }
        self.0
            .commands
            .try_send(command)
            .map_err(|_| Failure::Overflow)
    }
    fn call<T>(&self, command: impl FnOnce(Reply<T>) -> Command) -> Result<T, Failure> {
        let (send, receive) = mpsc::sync_channel(1);
        self.send(command(send))?;
        receive
            .recv_timeout(TIMEOUT)
            .map_err(|_| Failure::Stalled)?
    }
    pub fn begin(&self, selection: TranscriptionSelection) -> Result<Session, Failure> {
        let id = SessionId::random().map_err(|_| Failure::Storage)?;
        let health = Arc::new(Health {
            started: Instant::now(),
            ready: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            fault: AtomicU8::new(0),
            acknowledged_ms: AtomicU64::new(1),
            pending: Mutex::new(VecDeque::new()),
        });
        self.send(Command::Begin {
            id: id.clone(),
            selection,
            health: health.clone(),
        })?;
        Ok(Session {
            client: self.clone(),
            id,
            health,
        })
    }
    pub(super) fn reserve_retry(&self) -> Result<super::retry::Guard, Failure> {
        self.0
            .retrying
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| Failure::Overflow)?;
        Ok(super::retry::Guard(self.0.retrying.clone()))
    }
    pub(super) fn start_replay(&self, id: SessionId) -> Result<AudioReader, Failure> {
        self.call(|reply| Command::ReplayStart { id, reply })
    }
    pub(super) fn finish_replay(
        &self,
        reader: AudioReader,
        selection: TranscriptionSelection,
        result: Result<String, Failure>,
    ) -> Result<(), Failure> {
        self.call(|reply| Command::ReplayFinish {
            reader,
            selection,
            result,
            reply,
        })
    }
    pub fn revision(&self) -> u64 {
        self.0.revision.load(Ordering::Acquire)
    }
    pub fn list(&self) -> Result<Vec<Entry>, Failure> {
        self.call(Command::List)
    }
    pub fn text(&self, id: SessionId, raw: bool) -> Result<String, Failure> {
        self.call(|reply| Command::ReadText { id, raw, reply })
    }
    pub fn reader(&self, id: SessionId) -> Result<AudioReader, Failure> {
        self.call(|reply| Command::Reader { id, reply })
    }
    pub fn resolve(&self, id: SessionId, keep: bool) -> Result<(), Failure> {
        self.call(|reply| Command::Resolve { id, keep, reply })
    }
    pub fn delete(&self, id: SessionId) -> Result<(), Failure> {
        self.call(|reply| Command::Delete { id, reply })
    }
    pub fn shutdown(&self) -> Result<(), Failure> {
        let result = self.call(Command::Stop);
        self.0.closed.store(true, Ordering::Release);
        result
    }
}

impl Session {
    fn elapsed(&self) -> u64 {
        self.health.started.elapsed().as_millis() as u64 + 1
    }
    pub fn id(&self) -> &SessionId {
        &self.id
    }
    #[cfg(test)]
    pub fn ready(&self) -> bool {
        self.health.ready.load(Ordering::Acquire)
    }
    pub fn fault(&self) -> Option<Failure> {
        let value = self.health.fault.load(Ordering::Acquire);
        if value != 0 {
            return Some(decode_fault(value));
        }
        let now = self.elapsed();
        if now.saturating_sub(self.health.acknowledged_ms.load(Ordering::Acquire)) > STALE {
            self.latch(Failure::Stalled);
            return Some(Failure::Stalled);
        }
        let oldest = self
            .health
            .pending
            .lock()
            .unwrap()
            .front()
            .copied()
            .unwrap_or(0);
        if oldest != 0 && now.saturating_sub(oldest) > 1_000 {
            self.latch(Failure::Overflow);
            return Some(Failure::Overflow);
        }
        None
    }
    fn latch(&self, failure: Failure) {
        let _ = self.health.fault.compare_exchange(
            0,
            encode_fault(failure),
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
    pub fn offer(&self, start: u64, samples: &[f32]) -> Result<(), Failure> {
        if let Some(failure) = self.fault() {
            return Err(failure);
        }
        if samples.len() > FRAME_SAMPLES {
            self.latch(Failure::Overflow);
            return Err(Failure::Overflow);
        }
        // Capture runs on the audio owner, never in the native callback. The
        // tiny queue lock serializes timestamps with the consumer's removal.
        let mut pending = self.health.pending.lock().unwrap();
        pending.push_back(self.elapsed());
        let result = self.client.send(Command::Audio {
            id: self.id.clone(),
            start,
            samples: samples.to_vec(),
        });
        if let Err(error) = result {
            pending.pop_back();
            self.latch(error);
        }
        result
    }

    pub fn gap(&self, missing: u64) -> Result<(), Failure> {
        self.client
            .send(Command::Gap {
                id: self.id.clone(),
                missing,
            })
            .inspect_err(|error| self.latch(*error))
    }
    pub fn seal(&self, samples: u64) -> Result<(), Failure> {
        self.client
            .send(Command::Seal {
                id: self.id.clone(),
                samples,
            })
            .inspect_err(|error| self.latch(*error))
    }
    pub fn save_raw(&self, text: &str) -> Result<(), Failure> {
        self.client.call(|reply| Command::Text {
            id: self.id.clone(),
            text: text.to_owned(),
            raw: true,
            intent: false,
            reply,
        })
    }
    pub fn prepare_delivery(&self, text: &str) -> Result<(), Failure> {
        self.client.call(|reply| Command::Text {
            id: self.id.clone(),
            text: text.to_owned(),
            raw: false,
            intent: true,
            reply,
        })
    }
    pub fn outcome(&self, delivery: Delivery) -> Result<(), Failure> {
        self.client.call(|reply| Command::Outcome {
            id: self.id.clone(),
            delivery,
            reply,
        })
    }
    pub fn cancel(&self) -> Result<(), Failure> {
        self.health.cancelled.store(true, Ordering::Release);
        self.latch(Failure::Interrupted);
        let (reply, _received) = mpsc::sync_channel(1);
        match self.client.send(Command::Delete {
            id: self.id.clone(),
            reply,
        }) {
            Ok(()) | Err(Failure::Overflow) => Ok(()),
            Err(error) => Err(error),
        }
    }
}

pub(super) fn encode_fault(failure: Failure) -> u8 {
    match failure {
        Failure::Capacity => 2,
        Failure::Overflow => 3,
        Failure::Stalled => 4,
        _ => 1,
    }
}
fn decode_fault(value: u8) -> Failure {
    match value {
        2 => Failure::Capacity,
        3 => Failure::Overflow,
        4 => Failure::Stalled,
        _ => Failure::Storage,
    }
}

impl Command {
    pub(super) fn fail(self, error: Failure) {
        match self {
            Self::Begin { health, .. } => {
                health.fault.store(encode_fault(error), Ordering::Release);
            }
            Self::Text { reply, .. }
            | Self::Outcome { reply, .. }
            | Self::Resolve { reply, .. }
            | Self::Delete { reply, .. }
            | Self::ReplayFinish { reply, .. }
            | Self::Stop(reply) => {
                let _ = reply.send(Err(error));
            }
            Self::List(reply) => {
                let _ = reply.send(Err(error));
            }
            Self::ReadText { reply, .. } => {
                let _ = reply.send(Err(error));
            }
            Self::Reader { reply, .. } | Self::ReplayStart { reply, .. } => {
                let _ = reply.send(Err(error));
            }
            Self::Audio { .. } | Self::Gap { .. } | Self::Seal { .. } => {}
        }
    }
}
