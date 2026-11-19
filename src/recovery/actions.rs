//! Product actions reuse the existing control protocol and transcription engine.
use super::{Client, Failure, SessionId};
use crate::control::{
    ControlReply,
    recovery::{RecoveryCommand, RecoverySummary},
};
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

pub fn execute(client: &Client, command: RecoveryCommand) -> ControlReply {
    match run(client, command) {
        Ok(reply) => reply,
        Err(error) => ControlReply::error("recovery-failed", error),
    }
}
fn id(value: String) -> Result<SessionId, String> {
    SessionId::try_from(value).map_err(str::to_owned)
}
fn failed(error: Failure) -> String {
    match error {
        Failure::Capacity => {
            "Recovery storage is full. Resolve or delete saved work before recording again."
        }
        Failure::Overflow => "Recovery is busy. Try again after the current operation finishes.",
        Failure::Stalled => {
            "Recovery storage did not respond. Check Recovery before repeating the action."
        }
        _ => "The recovery operation could not be confirmed. Check Recovery before repeating it.",
    }
    .into()
}
fn run(client: &Client, command: RecoveryCommand) -> Result<ControlReply, String> {
    match command {
        RecoveryCommand::List => {
            let entries = client
                .list()
                .map_err(failed)?
                .into_iter()
                .map(|entry| {
                    let entry: super::Entry = entry;
                    let snapshot = entry.snapshot;
                    RecoverySummary {
                        id: entry.id.as_str().to_owned(),
                        bytes: entry.bytes,
                        created_ms: snapshot.as_ref().map(|value| value.created_ms),
                        saved_audio_ms: snapshot.as_ref().map_or(0, |value| value.samples / 16),
                        capture: snapshot
                            .as_ref()
                            .map_or_else(|| "unknown_format".into(), |value| label(&value.capture)),
                        delivery: snapshot
                            .as_ref()
                            .map_or_else(|| "unknown".into(), |value| label(&value.delivery)),
                        error: snapshot
                            .as_ref()
                            .and_then(|value| value.error.map(|error| label(&error))),
                        model: snapshot
                            .as_ref()
                            .map(|value| value.selection.model.as_str().to_owned()),
                        language: snapshot
                            .as_ref()
                            .map(|value| value.selection.language.clone()),
                        has_raw_text: snapshot
                            .as_ref()
                            .is_some_and(|value| value.raw_revision.is_some()),
                        has_final_text: snapshot
                            .as_ref()
                            .is_some_and(|value| value.final_revision.is_some()),
                        audio_available: snapshot
                            .as_ref()
                            .is_some_and(|value| !value.audio_removed),
                        kept: snapshot.as_ref().is_some_and(|value| value.keep),
                        retrying: snapshot.as_ref().is_some_and(|value| value.retrying),
                        deleting: snapshot.as_ref().is_some_and(|value| value.deleting),
                    }
                })
                .collect();
            Ok(ControlReply::Recovery { entries })
        }
        RecoveryCommand::Text { id: value, raw } => Ok(ControlReply::Transcript {
            text: client.text(id(value)?, raw).map_err(failed)?,
        }),
        RecoveryCommand::Resolve { id: value, keep } => {
            client.resolve(id(value)?, keep).map_err(failed)?;
            Ok(ControlReply::done(if keep {
                "Recovery item kept"
            } else {
                "Recovery item resolved; normal safety-copy expiry applies"
            }))
        }
        RecoveryCommand::Delete { id: value } => match client.delete(id(value)?) {
            Ok(()) => Ok(ControlReply::done("Recovery safety copy deleted")),
            Err(Failure::Busy) => Ok(ControlReply::done(
                "Deletion accepted; waiting for active audio work to finish",
            )),
            Err(error) => Err(failed(error)),
        },
        RecoveryCommand::ExportAudio { id: value, path } => {
            let mut reader = client.reader(id(value)?).map_err(failed)?;
            export(&path, |file| {
                let mut writer = hound::WavWriter::new(std::io::BufWriter::new(file), hound::WavSpec {
                    channels: 1, sample_rate: 16_000, bits_per_sample: 32, sample_format: hound::SampleFormat::Float,
                }).map_err(io::Error::other)?;
                while let Some(samples) = reader.next()? {
                    for sample in samples { writer.write_sample(sample).map_err(io::Error::other)?; }
                }
                writer.finalize().map_err(io::Error::other)
            }).map_err(|_| "Could not export audio. Choose a new writable file path; existing files are never overwritten.".to_string())?;
            Ok(ControlReply::done("Recovery audio exported"))
        }
        RecoveryCommand::ExportText {
            id: value,
            path,
            raw,
        } => {
            let text = client.text(id(value)?, raw).map_err(failed)?;
            export(&path, |file| file.write_all(text.as_bytes()))
                .map_err(|_| "Could not export text. Choose a new writable file path; existing files are never overwritten.".to_string())?;
            Ok(ControlReply::done("Recovery text exported"))
        }
        RecoveryCommand::Retry {
            id: value,
            current_model,
        } => {
            super::retry::start(client.clone(), id(value)?, current_model)?;
            Ok(ControlReply::done(
                "Local recovery transcription started; nothing will be pasted",
            ))
        }
    }
}
fn label(value: &impl serde::Serialize) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".into())
}

fn export(path: &Path, write: impl FnOnce(&mut File) -> io::Result<()>) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|value| !value.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let staging = parent.join(format!(".hex-export-{}", SessionId::random()?.as_str()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&staging)?;
        // Keep the created descriptor through the barrier. Reopening the
        // pathname could synchronize a replacement in a shared directory.
        write(&mut file)?;
        file.sync_all()?;
        // link is an atomic create-if-absent: unlike rename, it cannot clobber
        // an existing destination between a preliminary check and publication.
        std::fs::hard_link(&staging, path)?;
        std::fs::remove_file(&staging)?;
        File::open(parent)?.sync_all()?;
        super::fs::full_sync(&file)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(staging);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn exports_are_private_never_overwrite_and_do_not_publish_partial_output() {
        let parent = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "hex-recovery-export-{}",
            SessionId::random().unwrap().as_str()
        ));
        std::fs::create_dir(&parent).unwrap();
        let path = parent.join("transcript.txt");
        export(&path, |file| file.write_all(b"synthetic complete text")).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"synthetic complete text");
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(export(&path, |file| file.write_all(b"must not replace")).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"synthetic complete text");
        let partial = parent.join("partial.txt");
        assert!(
            export(&partial, |file| {
                file.write_all(b"incomplete")?;
                Err(io::Error::other("synthetic export failure"))
            })
            .is_err()
        );
        assert!(!partial.exists());
        assert_eq!(
            std::fs::read_dir(&parent).unwrap().count(),
            1,
            "no abandoned staging files"
        );
        std::fs::remove_dir_all(parent).unwrap();
    }
}
