use super::*;
use std::io::{Read, Write};
use std::time::{Duration, Instant};

struct Root(std::path::PathBuf);
impl Root {
    fn new() -> Self {
        let parent = std::env::temp_dir().canonicalize().unwrap();
        Self(parent.join(format!(
            "hex-recovery-test-{}",
            SessionId::random().unwrap().as_str()
        )))
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn ready(session: &Session) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !session.ready() && session.fault().is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(session.ready(), "admission must be durably acknowledged");
}

#[test]
fn acknowledged_capture_text_and_delivery_survive_restart() {
    let root = Root::new();
    let client = Client::start(root.0.clone()).unwrap();
    let session = client.begin(Default::default()).unwrap();
    ready(&session);
    for index in 0..35 {
        session
            .offer(index * 16_000, &vec![index as f32 / 35.0; 16_000])
            .unwrap();
    }
    session.seal(34 * 16_000 + 777).unwrap();
    session.save_raw("synthetic raw transcript").unwrap();
    session
        .prepare_delivery("synthetic final transcript")
        .unwrap();
    let id = session.id().clone();
    assert_eq!(
        client.text(id.clone(), true).unwrap(),
        "synthetic raw transcript"
    );
    let mut reader = client.reader(id.clone()).unwrap();
    let mut samples = Vec::new();
    while let Some(chunk) = reader.next().unwrap() {
        samples.extend(chunk);
    }
    assert_eq!(samples.len(), 34 * 16_000 + 777);
    assert_eq!(samples[32 * 16_000], 32.0 / 35.0);
    drop(reader);
    client.shutdown().unwrap();
    let reopened = Client::start(root.0.clone()).unwrap();
    let list = reopened.list().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(
        list[0].snapshot.as_ref().unwrap().delivery,
        Delivery::Unknown
    );
    assert_eq!(
        reopened.text(id.clone(), false).unwrap(),
        "synthetic final transcript"
    );
    reopened.delete(id).unwrap();
    assert!(reopened.list().unwrap().is_empty());
    reopened.shutdown().unwrap();
}

#[test]
fn framed_audio_rejects_damage_and_oversized_headers() {
    let mut bytes = Vec::new();
    super::format::write_frame(&mut bytes, 0, 0, &[0.25; 64]).unwrap();
    assert_eq!(
        super::format::read_frame(&mut bytes.as_slice())
            .unwrap()
            .unwrap()
            .samples,
        [0.25; 64]
    );
    bytes[30] ^= 1;
    assert!(super::format::read_frame(&mut bytes.as_slice()).is_err());
    bytes[20..24].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(super::format::read_frame(&mut bytes.as_slice()).is_err());
    assert!(super::format::write_frame(&mut Vec::new(), 0, 0, &[f32::NAN]).is_err());
}

#[test]
fn strict_storage_rejects_symlinks_hardlinks_and_public_files() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = Root::new();
    let directory = super::fs::Directory::open(&root.0).unwrap();
    let mut owned = directory.file("owned", true).unwrap();
    owned.write_all(b"private").unwrap();
    symlink("owned", root.0.join("link")).unwrap();
    assert!(directory.file("link", false).is_err());
    std::fs::hard_link(root.0.join("owned"), root.0.join("hardlink")).unwrap();
    assert!(directory.file("owned", false).is_err());
    std::fs::remove_file(root.0.join("hardlink")).unwrap();
    std::fs::set_permissions(root.0.join("owned"), std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(directory.file("owned", false).is_err());
    assert!(directory.file("../outside", true).is_err());
    let mut content = String::new();
    std::fs::File::open(root.0.join("owned"))
        .unwrap()
        .read_to_string(&mut content)
        .unwrap();
    assert_eq!(content, "private");
}

#[test]
#[ignore = "invoked only by the isolated production-store crash test"]
fn production_store_crash_child() {
    let root = std::env::var_os("HEX_RECOVERY_TEST_ROOT").expect("subprocess scratch root");
    let kind = std::env::var("HEX_RECOVERY_TEST_KIND").unwrap();
    super::store::test_support::crash_fixture(root.into(), &kind).unwrap();
}

#[test]
fn production_store_commit_edges_survive_process_death() {
    use std::process::Command;
    let binary = std::env::current_exe().unwrap();
    for (kind, edges) in [
        (
            "audio",
            &[
                "audio_written",
                "audio_synced",
                "manifest_written",
                "manifest_synced",
                "manifest_renamed",
                "manifest_ack",
            ][..],
        ),
        (
            "text",
            &[
                "text_written",
                "text_synced",
                "text_published",
                "manifest_written",
                "manifest_synced",
                "manifest_renamed",
                "manifest_ack",
            ][..],
        ),
        (
            "delete",
            &[
                "tombstone_synced",
                "delete_ack",
                "files_removed",
                "directory_removed",
                "delete_complete",
            ][..],
        ),
    ] {
        for edge in edges {
            let root = Root::new();
            let output = Command::new(&binary)
                .args([
                    "--exact",
                    "recovery::tests::production_store_crash_child",
                    "--ignored",
                    "--nocapture",
                ])
                .env("HEX_RECOVERY_TEST_ROOT", &root.0)
                .env("HEX_RECOVERY_TEST_KIND", kind)
                .env("HEX_RECOVERY_TEST_EDGE", edge)
                .output()
                .unwrap();
            assert_eq!(
                output.status.code(),
                Some(86),
                "{kind}/{edge}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let stdout = String::from_utf8(output.stdout).unwrap();
            let id = stdout
                .lines()
                .find_map(|line| line.strip_prefix("BASE "))
                .unwrap()
                .to_owned();
            let id = SessionId::try_from(id).unwrap();
            if kind == "audio" && *edge == "audio_written" {
                let path = root.0.join(id.as_str()).join("audio-00000000.pcm");
                let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
                file.set_len(file.metadata().unwrap().len() - 7).unwrap();
            }
            let client = Client::start(root.0.clone()).unwrap();
            let entries = client.list().unwrap();
            if kind == "delete" {
                assert!(
                    entries.is_empty(),
                    "{edge}: deletion must finish on restart"
                );
            } else {
                assert_eq!(entries.len(), 1);
                if kind == "text" {
                    assert_eq!(client.text(id.clone(), true).unwrap(), "synthetic baseline");
                }
                let mut reader = client.reader(id).unwrap();
                let first = reader.next().unwrap().unwrap();
                assert_eq!(first, vec![0.25; 16_000], "{kind}/{edge}");
                let mut count = first.len();
                while let Some(chunk) = reader.next().unwrap() {
                    count += chunk.len();
                }
                assert_eq!(
                    count,
                    if kind == "audio" && *edge != "audio_written" {
                        32_000
                    } else {
                        16_000
                    },
                    "{kind}/{edge}: validated tail extent"
                );
            }
            client.shutdown().unwrap();
        }
    }
}

#[test]
fn deletion_fences_late_output_and_waits_for_audio_readers() {
    let root = Root::new();
    let client = Client::start(root.0.clone()).unwrap();
    let session = client.begin(Default::default()).unwrap();
    ready(&session);
    session.offer(0, &[0.25; 1_000]).unwrap();
    session.seal(1_000).unwrap();
    session.save_raw("synthetic saved text").unwrap();
    let mut reader = client.reader(session.id().clone()).unwrap();
    assert_eq!(client.delete(session.id().clone()), Err(Failure::Busy));
    assert!(
        client.list().unwrap()[0]
            .snapshot
            .as_ref()
            .unwrap()
            .deleting
    );
    assert!(
        session
            .prepare_delivery("a late result must not resurrect this item")
            .is_err()
    );
    assert!(reader.next().is_err(), "deletion must stop active readers");
    drop(reader);
    let deadline = Instant::now() + Duration::from_secs(2);
    while !client.list().unwrap().is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(client.list().unwrap().is_empty());
    client.shutdown().unwrap();
    let reopened = Client::start(root.0.clone()).unwrap();
    assert!(reopened.list().unwrap().is_empty());
    reopened.shutdown().unwrap();
}

#[test]
fn transcript_checksum_detects_valid_json_corruption() {
    let root = Root::new();
    let client = Client::start(root.0.clone()).unwrap();
    let session = client.begin(Default::default()).unwrap();
    ready(&session);
    session.offer(0, &[0.5; 100]).unwrap();
    session.seal(100).unwrap();
    session.save_raw("synthetic original").unwrap();
    let path = root
        .0
        .join(session.id().as_str())
        .join("text-00000000.json");
    let mut text: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    text["text"] = "synthetic tampered".into();
    std::fs::write(path, serde_json::to_vec(&text).unwrap()).unwrap();
    assert!(client.text(session.id().clone(), true).is_err());
    assert_eq!(
        client
            .reader(session.id().clone())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .len(),
        100
    );
    client.shutdown().unwrap();
}

#[test]
fn audio_gaps_are_checksummed_without_inventing_samples() {
    let root = Root::new();
    let client = Client::start(root.0.clone()).unwrap();
    let session = client.begin(Default::default()).unwrap();
    ready(&session);
    session.offer(0, &[0.25; 100]).unwrap();
    session.gap(800).unwrap();
    session.offer(100, &[0.5; 100]).unwrap();
    session.seal(200).unwrap();
    let snapshot = client.list().unwrap().remove(0).snapshot.unwrap();
    assert_eq!(snapshot.gap_count, 1);
    assert_eq!(snapshot.error, Some(Failure::Interrupted));
    let mut reader = client.reader(session.id().clone()).unwrap();
    let mut samples = Vec::new();
    while let Some(chunk) = reader.next().unwrap() {
        samples.extend(chunk);
    }
    assert_eq!(samples, [&[0.25; 100][..], &[0.5; 100][..]].concat());
    drop(reader);
    client.shutdown().unwrap();
}

#[test]
#[ignore = "explicit synthetic release workload; writes only to an isolated scratch root"]
fn measure_durable_release_to_delivery_overhead() {
    let root = Root::new();
    let client = Client::start(root.0.clone()).unwrap();
    let mut elapsed_ms = Vec::new();
    for _ in 0..60 {
        let session = client.begin(Default::default()).unwrap();
        ready(&session);
        session.offer(0, &[0.25; 16_000]).unwrap();
        let started = Instant::now();
        session.seal(16_000).unwrap();
        session
            .save_raw("Synthetic transcript for a local durability measurement.")
            .unwrap();
        session
            .prepare_delivery("Synthetic transcript for a local durability measurement.")
            .unwrap();
        elapsed_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        session.outcome(Delivery::ReportedSuccess).unwrap();
        client.delete(session.id().clone()).unwrap();
    }
    elapsed_ms.sort_by(f64::total_cmp);
    println!(
        "durable_release_overhead {}",
        serde_json::json!({
            "samples": elapsed_ms.len(), "p50_ms": elapsed_ms[30], "p95_ms": elapsed_ms[56],
            "max_ms": elapsed_ms[59], "includes": "seal, raw transcript and final transcript with delivery intent",
            "excludes": "inference, processing, OS delivery, initial admission and outcome"
        })
    );
    client.shutdown().unwrap();
}

#[test]
fn failed_commits_preserve_raw_text_fence_late_writes_and_reconcile_orphan_revisions() {
    for edge in [
        "manifest_written",
        "manifest_synced",
        "manifest_renamed",
        "manifest_ack",
    ] {
        for kind in [
            std::io::ErrorKind::StorageFull,
            std::io::ErrorKind::PermissionDenied,
        ] {
            let root = Root::new();
            super::store::test_support::failed_text_commit(root.0.clone(), edge, kind).unwrap();
        }
    }
}

#[test]
#[ignore = "explicit two-hour synthetic storage workload; about 440 MiB scratch space"]
fn two_hour_storage_workload_stays_bounded_and_reclaims_segments() {
    fn peak_resident_bytes() -> u64 {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
        // SAFETY: getrusage initializes the structure on success.
        assert_eq!(
            unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) },
            0
        );
        // SAFETY: getrusage succeeded; on macOS ru_maxrss is bytes.
        unsafe { usage.assume_init() }.ru_maxrss as u64
    }
    let root = Root::new();
    let client = Client::start(root.0.clone()).unwrap();
    let session = client.begin(Default::default()).unwrap();
    ready(&session);
    let before = peak_resident_bytes();
    let samples = [0.25; 16_000];
    let started = Instant::now();
    for second in 0..7200_u64 {
        session.offer(second * 16_000, &samples).unwrap();
        if second % 16 == 15 {
            assert_eq!(client.list().unwrap().len(), 1);
        }
    }
    session.seal(7200 * 16_000).unwrap();
    session.save_raw("Synthetic two hour workload.").unwrap();
    session
        .prepare_delivery("Synthetic two hour workload.")
        .unwrap();
    session.outcome(Delivery::ReportedSuccess).unwrap();
    let extra = peak_resident_bytes().saturating_sub(before);
    assert!(
        extra < 64 * 1024 * 1024,
        "bounded storage overhead: {extra} bytes"
    );
    let names = std::fs::read_dir(root.0.join(session.id().as_str()))
        .unwrap()
        .count();
    assert_eq!(
        names, 243,
        "240 audio segments, manifest and two text revisions"
    );
    println!(
        "storage_soak {}",
        serde_json::json!({"audio_hours": 2,
        "extra_peak_resident_bytes": extra, "files": names,
        "elapsed_ms": started.elapsed().as_millis(), "charged_bytes": client.list().unwrap()[0].bytes})
    );
    assert_eq!(client.delete(session.id().clone()), Err(Failure::Busy));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !client.list().unwrap().is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(client.list().unwrap().is_empty());
    client.shutdown().unwrap();
}

#[test]
fn another_writer_cannot_open_or_mutate_the_locked_library() {
    let root = Root::new();
    let owner = Client::start(root.0.clone()).unwrap();
    let session = owner.begin(Default::default()).unwrap();
    ready(&session);
    session.offer(0, &[0.25; 100]).unwrap();
    session.seal(100).unwrap();
    session.save_raw("owned synthetic text").unwrap();
    let contender = Client::start(root.0.clone()).unwrap();
    assert!(contender.list().is_err());
    assert!(contender.delete(session.id().clone()).is_err());
    assert_eq!(
        owner.text(session.id().clone(), true).unwrap(),
        "owned synthetic text"
    );
    let _ = contender.shutdown();
    owner.shutdown().unwrap();
}

#[test]
fn newer_formats_remain_visible_and_untouched_until_explicit_deletion() {
    let root = Root::new();
    let client = Client::start(root.0.clone()).unwrap();
    let session = client.begin(Default::default()).unwrap();
    ready(&session);
    session.offer(0, &[0.25; 100]).unwrap();
    session.seal(100).unwrap();
    session.save_raw("synthetic future data").unwrap();
    let id = session.id().clone();
    client.shutdown().unwrap();
    let manifest = root.0.join(id.as_str()).join("manifest.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    value["version"] = 999.into();
    let bytes = serde_json::to_vec(&value).unwrap();
    std::fs::write(&manifest, &bytes).unwrap();
    let client = Client::start(root.0.clone()).unwrap();
    let entries = client.list().unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].snapshot.is_none());
    assert!(client.text(id.clone(), true).is_err());
    assert!(client.resolve(id.clone(), false).is_err());
    assert_eq!(std::fs::read(&manifest).unwrap(), bytes);
    client.delete(id).unwrap();
    assert!(client.list().unwrap().is_empty());
    client.shutdown().unwrap();
}

#[test]
fn a_processing_failure_stays_unresolved_after_successful_fallback_delivery() {
    let root = Root::new();
    let client = Client::start(root.0.clone()).unwrap();
    let session = client.begin(Default::default()).unwrap();
    ready(&session);
    session.offer(0, &[0.25; 100]).unwrap();
    session.seal(100).unwrap();
    session.save_raw("original synthetic transcript").unwrap();
    session.preserve_failure(Failure::Processing).unwrap();
    session
        .prepare_delivery("fallback synthetic transcript")
        .unwrap();
    session.outcome(Delivery::ReportedSuccess).unwrap();
    let id = session.id().clone();
    client.shutdown().unwrap();
    let client = Client::start(root.0.clone()).unwrap();
    let entries = client.list().unwrap();
    let snapshot = entries[0].snapshot.as_ref().unwrap();
    assert_eq!(snapshot.error, Some(Failure::Processing));
    assert_eq!(snapshot.delivery, Delivery::ReportedSuccess);
    assert!(snapshot.resolved_ms.is_none());
    assert_eq!(
        client.text(id.clone(), true).unwrap(),
        "original synthetic transcript"
    );
    assert!(client.reader(id).unwrap().next().unwrap().is_some());
    client.shutdown().unwrap();
}
