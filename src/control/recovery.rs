//! Recovery's additive control protocol, shared by CLI and desktop hosts.
use serde::{Deserialize, Serialize};

/// Decoded UTF-8 transcript cap; JSON may escape a byte with six bytes.
pub const MAX_TRANSCRIPT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
pub enum RecoveryCommand {
    List,
    Text {
        id: String,
        #[serde(default)]
        raw: bool,
    },
    Retry {
        id: String,
        #[serde(default)]
        current_model: bool,
    },
    ExportAudio {
        id: String,
        path: std::path::PathBuf,
    },
    ExportText {
        id: String,
        path: std::path::PathBuf,
        #[serde(default)]
        raw: bool,
    },
    Resolve {
        id: String,
        #[serde(default)]
        keep: bool,
    },
    Delete {
        id: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RecoverySummary {
    pub id: String,
    pub created_ms: Option<u64>,
    pub saved_audio_ms: u64,
    pub bytes: u64,
    pub capture: String,
    pub delivery: String,
    pub error: Option<String>,
    pub model: Option<String>,
    pub language: Option<String>,
    pub has_raw_text: bool,
    pub has_final_text: bool,
    pub audio_available: bool,
    pub kept: bool,
    pub retrying: bool,
    pub deleting: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_is_an_additive_authenticated_control_operation() {
        let request = crate::control::ControlRequest {
            version: crate::control::CONTROL_VERSION,
            command: crate::control::ControlCommand::Recovery {
                operation: RecoveryCommand::Delete {
                    id: "0123456789abcdef0123456789abcdef".into(),
                },
            },
        };
        let wire = serde_json::to_value(&request).unwrap();
        assert_eq!(wire["command"], "recovery");
        assert_eq!(wire["operation"]["action"], "delete");
        assert_eq!(
            serde_json::from_value::<crate::control::ControlRequest>(wire).unwrap(),
            request
        );
    }
}
