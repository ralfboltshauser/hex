//! Subprocess fixtures use the production store, not an alternative backend.
use super::*;
use std::sync::atomic::{AtomicBool, AtomicU8};

pub(in crate::recovery) fn crash_fixture(root: PathBuf, kind: &str) -> io::Result<()> {
    let mut store = Store::open(root)?;
    let id = SessionId::random()?;
    let health = Arc::new(Health {
        started: Instant::now(),
        ready: AtomicBool::new(false),
        cancelled: AtomicBool::new(false),
        fault: AtomicU8::new(0),
        acknowledged_ms: AtomicU64::new(1),
        pending: std::sync::Mutex::new(std::collections::VecDeque::new()),
    });
    store.begin(id.clone(), Default::default(), health)?;
    store.audio(&id, 0, vec![0.25; FRAME_SAMPLES])?;
    store.checkpoint(&id, None)?;
    if kind != "audio" {
        store.checkpoint(&id, Some(FRAME_SAMPLES as u64))?;
        store.text(
            &id,
            "synthetic baseline".into(),
            true,
            false,
            TextSource::Capture,
        )?;
    }
    println!("BASE {}", id.as_str());
    std::io::stdout().flush()?;
    super::super::faults::arm();
    match kind {
        "audio" => {
            store.audio(&id, FRAME_SAMPLES as u64, vec![0.5; FRAME_SAMPLES])?;
            store.checkpoint(&id, None)?;
        }
        "text" => {
            store.text(
                &id,
                "synthetic final".into(),
                false,
                true,
                TextSource::Capture,
            )?;
        }
        "delete" => {
            store.delete(&id)?;
        }
        _ => return Err(io::Error::other("unknown crash fixture")),
    }
    Ok(())
}

pub(in crate::recovery) fn failed_text_commit(
    root: PathBuf,
    edge: &'static str,
    kind: io::ErrorKind,
) -> io::Result<()> {
    let mut store = Store::open(root.clone())?;
    let id = SessionId::random()?;
    let health = Arc::new(Health {
        started: Instant::now(),
        ready: AtomicBool::new(false),
        cancelled: AtomicBool::new(false),
        fault: AtomicU8::new(0),
        acknowledged_ms: AtomicU64::new(1),
        pending: std::sync::Mutex::new(std::collections::VecDeque::new()),
    });
    store.begin(id.clone(), Default::default(), health)?;
    store.audio(&id, 0, vec![0.25; FRAME_SAMPLES])?;
    store.checkpoint(&id, Some(FRAME_SAMPLES as u64))?;
    store.text(
        &id,
        "acknowledged synthetic raw".into(),
        true,
        false,
        TextSource::Capture,
    )?;
    super::super::faults::fail_once(edge, kind);
    assert!(
        store
            .text(
                &id,
                "uncertain synthetic final".into(),
                false,
                true,
                TextSource::Capture
            )
            .is_err()
    );
    assert_eq!(store.read_text(&id, true)?, "acknowledged synthetic raw");
    assert!(
        store
            .text(
                &id,
                "must remain fenced".into(),
                false,
                true,
                TextSource::Capture
            )
            .is_err()
    );
    assert!(
        store
            .text(
                &id,
                "manual retry remains fenced".into(),
                true,
                false,
                TextSource::Replay(Default::default())
            )
            .is_err()
    );
    drop(store);
    let mut reopened = Store::open(root)?;
    assert_eq!(reopened.read_text(&id, true)?, "acknowledged synthetic raw");
    reopened.text(
        &id,
        "reconciled synthetic final".into(),
        false,
        true,
        TextSource::Capture,
    )?;
    assert_eq!(
        reopened.read_text(&id, false)?,
        "reconciled synthetic final"
    );
    Ok(())
}
