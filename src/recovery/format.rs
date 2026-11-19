//! Version 1 recovery data. Keep parser limits and encoding in one place.
use crate::transcription::models::TranscriptionSelection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{self, Read, Write};

pub(super) const VERSION: u32 = 1;
pub(super) const RATE: u64 = 16_000;
pub(super) const SEGMENT_SAMPLES: u64 = RATE * 30;
pub(super) const FRAME_SAMPLES: usize = RATE as usize;
pub(super) const MAX_MANIFEST: u64 = 64 * 1024;
pub(super) const MAX_TEXT: usize = crate::control::recovery::MAX_TRANSCRIPT_BYTES;
const MAGIC: &[u8; 8] = b"HEXREC01";
const HEADER: usize = 24;

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct SessionId(String);

impl SessionId {
    pub(super) fn random() -> io::Result<Self> {
        let mut bytes = [0; 16];
        getrandom::fill(&mut bytes).map_err(io::Error::other)?;
        Ok(Self(
            bytes.iter().map(|byte| format!("{byte:02x}")).collect(),
        ))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for SessionId {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.len() == 32
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            Ok(Self(value))
        } else {
            Err("invalid recovery session ID")
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureState {
    Recording,
    Interrupted,
    Sealed,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Delivery {
    #[default]
    NotAttempted,
    Attempting,
    ReportedSuccess,
    Failed,
    Unknown,
    Suppressed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub id: SessionId,
    pub created_ms: u64,
    pub revision: u64,
    pub capture: CaptureState,
    /// Saved prefix while recording; authoritative final extent once sealed.
    pub samples: u64,
    pub selection: TranscriptionSelection,
    pub delivery: Delivery,
    pub raw_revision: Option<u32>,
    pub final_revision: Option<u32>,
    pub resolved_ms: Option<u64>,
    pub keep: bool,
    #[serde(default)]
    pub retrying: bool,
    #[serde(default)]
    pub deleting: bool,
    #[serde(default)]
    pub user_resolved: bool,
    #[serde(default)]
    pub gap_count: u64,
    pub audio_removed: bool,
    pub error: Option<Failure>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Failure {
    Storage,
    Capacity,
    Overflow,
    Stalled,
    Interrupted,
    EmptyResult,
    Transcription,
    Processing,
    ModelUnavailable,
    Busy,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Manifest {
    pub version: u32,
    #[serde(flatten)]
    pub snapshot: Snapshot,
    pub next_text: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Text {
    pub version: u32,
    pub session: SessionId,
    pub revision: u32,
    pub raw: bool,
    pub samples: u64,
    pub text: String,
    pub selection: TranscriptionSelection,
    pub checksum: [u8; 32],
}

pub(super) fn bounded_json<T: serde::de::DeserializeOwned>(
    file: impl Read,
    bound: u64,
) -> io::Result<T> {
    let mut bytes = Vec::new();
    file.take(bound + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > bound {
        return Err(io::Error::other("recovery file exceeds bound"));
    }
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}

pub(super) fn write_frame(
    file: &mut impl Write,
    sequence: u32,
    start: u64,
    samples: &[f32],
) -> io::Result<()> {
    if samples.is_empty() || samples.len() > FRAME_SAMPLES || samples.iter().any(|v| !v.is_finite())
    {
        return Err(io::Error::other("invalid recovery audio frame"));
    }
    let mut frame = Vec::with_capacity(HEADER + samples.len() * 4 + 32);
    frame.extend(MAGIC);
    frame.extend(sequence.to_le_bytes());
    frame.extend(start.to_le_bytes());
    frame.extend((samples.len() as u32).to_le_bytes());
    for sample in samples {
        frame.extend(sample.to_le_bytes());
    }
    frame.extend(Sha256::digest(&frame));
    file.write_all(&frame)
}

pub(super) fn write_gap(
    file: &mut impl Write,
    sequence: u32,
    end: u64,
    missing: u64,
) -> io::Result<()> {
    if missing == 0 || missing > RATE * 60 {
        return Err(io::Error::other("invalid gap duration"));
    }
    let mut record = Vec::with_capacity(HEADER + 8 + 32);
    record.extend(MAGIC);
    record.extend(sequence.to_le_bytes());
    record.extend(end.to_le_bytes());
    record.extend(0_u32.to_le_bytes());
    record.extend(missing.to_le_bytes());
    record.extend(Sha256::digest(&record));
    file.write_all(&record)
}

pub(super) struct Frame {
    pub sequence: u32,
    pub start: u64,
    pub samples: Vec<f32>,
    pub missing: Option<u64>,
}

pub(super) fn read_frame(file: &mut impl Read) -> io::Result<Option<Frame>> {
    let mut header = [0; HEADER];
    match file.read(&mut header[..1])? {
        0 => return Ok(None),
        1 => {}
        _ => unreachable!(),
    }
    file.read_exact(&mut header[1..])?;
    let sequence = u32::from_le_bytes(header[8..12].try_into().unwrap());
    let start = u64::from_le_bytes(header[12..20].try_into().unwrap());
    let count = u32::from_le_bytes(header[20..24].try_into().unwrap()) as usize;
    if &header[..8] != MAGIC || count > FRAME_SAMPLES || start.checked_add(count as u64).is_none() {
        return Err(io::Error::other("invalid recovery frame header"));
    }
    let mut payload = vec![0; if count == 0 { 8 } else { count * 4 }];
    file.read_exact(&mut payload)?;
    let mut digest = [0; 32];
    file.read_exact(&mut digest)?;
    let mut hash = Sha256::new();
    hash.update(header);
    hash.update(&payload);
    if hash.finalize().as_slice() != digest {
        return Err(io::Error::other("damaged recovery audio"));
    }
    if count == 0 {
        let missing = u64::from_le_bytes(payload.try_into().unwrap());
        if missing == 0 || missing > RATE * 60 {
            return Err(io::Error::other("invalid gap duration"));
        }
        return Ok(Some(Frame {
            sequence,
            start,
            samples: Vec::new(),
            missing: Some(missing),
        }));
    }
    let samples: Vec<_> = payload
        .as_chunks::<4>()
        .0
        .iter()
        .map(|bytes| f32::from_le_bytes(*bytes))
        .collect();
    if samples.iter().any(|v| !v.is_finite()) {
        return Err(io::Error::other("nonfinite recovery audio"));
    }
    Ok(Some(Frame {
        sequence,
        start,
        samples,
        missing: None,
    }))
}
