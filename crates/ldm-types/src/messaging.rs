use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::adapter::DetectedDownloadCandidate;
use crate::browser_integration::BrowserIntegrationPreferences;
use crate::download_job::CreateDownloadJobRequest;
use crate::SCHEMA_VERSION;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
#[derive(TS)]
#[ts(export)]
pub enum ExtensionToDesktopMessage {
    Ping {
        #[serde(rename = "schemaVersion")]
        schema_version: u16,
    },
    GetBrowserIntegrationPreferences {
        #[serde(rename = "schemaVersion")]
        schema_version: u16,
    },
    DetectedCandidates {
        #[serde(rename = "schemaVersion")]
        schema_version: u16,
        candidates: Vec<DetectedDownloadCandidate>,
    },
    CreateJob {
        #[serde(rename = "schemaVersion")]
        schema_version: u16,
        job: Box<CreateDownloadJobRequest>,
    },
}

impl ExtensionToDesktopMessage {
    pub fn schema_version(&self) -> u16 {
        match self {
            Self::Ping { schema_version }
            | Self::GetBrowserIntegrationPreferences { schema_version }
            | Self::DetectedCandidates { schema_version, .. }
            | Self::CreateJob { schema_version, .. } => *schema_version,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
#[derive(TS)]
#[ts(export)]
pub enum DesktopToExtensionMessage {
    Pong {
        #[serde(rename = "schemaVersion")]
        schema_version: u16,
    },
    BrowserIntegrationPreferences {
        #[serde(rename = "schemaVersion")]
        schema_version: u16,
        preferences: BrowserIntegrationPreferences,
    },
    JobAccepted {
        #[serde(rename = "schemaVersion")]
        schema_version: u16,
        #[serde(rename = "jobId")]
        job_id: Uuid,
    },
    JobRejected {
        #[serde(rename = "schemaVersion")]
        schema_version: u16,
        reason: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        code: Option<RejectionCode>,
    },
    /// Sent before the real reply while the desktop app is being launched; the extension waits longer.
    DesktopStarting {
        #[serde(rename = "schemaVersion")]
        schema_version: u16,
        detail: String,
    },
}

/// Rejections the extension handles itself instead of showing `reason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
#[derive(TS)]
#[ts(export)]
pub enum RejectionCode {
    FirstRunSetupRequired,
    DesktopUnavailable,
    /// The server refused LDM's request; the browser keeps the download.
    SourceRefused,
    #[serde(other)]
    Unknown,
}

impl DesktopToExtensionMessage {
    pub fn rejected(reason: impl Into<String>) -> Self {
        Self::JobRejected {
            schema_version: SCHEMA_VERSION,
            reason: reason.into(),
            code: None,
        }
    }

    pub fn rejected_because(code: RejectionCode, reason: impl Into<String>) -> Self {
        Self::JobRejected {
            schema_version: SCHEMA_VERSION,
            reason: reason.into(),
            code: Some(code),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_rejection_without_a_code_looks_as_it_always_did() {
        let message = DesktopToExtensionMessage::rejected("Unsupported message schema version.");

        assert_eq!(
            serde_json::to_value(&message).unwrap(),
            json!({
                "type": "jobRejected",
                "schemaVersion": 1,
                "reason": "Unsupported message schema version.",
            })
        );
    }

    #[test]
    fn a_rejection_with_a_code_names_it_next_to_the_reason() {
        let message = DesktopToExtensionMessage::rejected_because(
            RejectionCode::FirstRunSetupRequired,
            "Open LDM and complete the first-run setup.",
        );

        assert_eq!(
            serde_json::to_value(&message).unwrap(),
            json!({
                "type": "jobRejected",
                "schemaVersion": 1,
                "reason": "Open LDM and complete the first-run setup.",
                "code": "firstRunSetupRequired",
            })
        );
    }

    #[test]
    fn a_code_from_a_newer_version_does_not_break_the_message() {
        let message: DesktopToExtensionMessage = serde_json::from_value(json!({
            "type": "jobRejected",
            "schemaVersion": 1,
            "reason": "Something new.",
            "code": "somethingNew",
        }))
        .unwrap();

        assert!(matches!(
            message,
            DesktopToExtensionMessage::JobRejected {
                code: Some(RejectionCode::Unknown),
                ..
            }
        ));
    }
}
