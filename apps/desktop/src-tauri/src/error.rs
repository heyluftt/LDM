//! The error every command hands to the webview.
//!
//! A command used to fail with a sentence, and whoever wanted to react to one kind of failure
//! had to look for words in it. `AppError` carries a code for that next to the sentence. The
//! webview receives it as `{ "code": "...", "message": "..." }`.
//!
//! Private helpers that only ever produce one kind of failure (the state file readers and
//! writers, the registry calls) still return their text; the public function that calls them
//! attaches the code.

use ldm_core::error::CoreError;
use serde::Serialize;

/// What kind of failure it is. `apps/desktop/src/shared/api/invoke.ts` lists the same codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    /// The first-run notice has not been accepted, so nothing may be downloaded yet.
    FirstRunSetupRequired,
    /// There is no job, history entry, or stored request with that id.
    JobNotFound,
    /// The job exists, but not in a state that allows the action.
    InvalidJobState,
    /// The input itself is unusable: a malformed id, a URL LDM cannot download, a bad folder.
    InvalidRequest,
    /// The file or folder to open is no longer there.
    FileNotFound,
    /// The server refused LDM's request for a file the browser handed over; the browser keeps
    /// the download.
    SourceRefused,
    /// A state or settings file could not be read or written.
    Storage,
    /// A request to another machine failed.
    Network,
    /// Everything else; nothing the caller can do about it.
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppError {
    pub code: ErrorCode,
    /// The sentence shown to the user.
    pub message: String,
}

impl AppError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn job_not_found() -> Self {
        Self::new(ErrorCode::JobNotFound, "download job was not found")
    }

    pub fn invalid_job_state(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidJobState, message)
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidRequest, message)
    }

    pub fn file_not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::FileNotFound, message)
    }

    pub fn storage(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Storage, message)
    }

    pub fn network(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Network, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for AppError {}

impl From<CoreError> for AppError {
    fn from(error: CoreError) -> Self {
        let code = match &error {
            CoreError::InvalidUrl(_)
            | CoreError::InvalidRequest(_)
            | CoreError::MissingFilename => ErrorCode::InvalidRequest,
            CoreError::Io(_) => ErrorCode::Storage,
            CoreError::Request(_) | CoreError::HttpStatus(_) => ErrorCode::Network,
            CoreError::ProviderBackend(_)
            | CoreError::ProviderFailure(_)
            | CoreError::LiveStream
            | CoreError::Canceled => ErrorCode::Internal,
        };
        Self::new(code, error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_webview_receives_the_code_and_the_message() {
        let error = AppError::invalid_job_state("only completed downloads can be opened");

        assert_eq!(
            serde_json::to_value(&error).unwrap(),
            serde_json::json!({
                "code": "invalidJobState",
                "message": "only completed downloads can be opened",
            })
        );
        assert_eq!(error.to_string(), "only completed downloads can be opened");
    }

    #[test]
    fn every_code_has_the_name_the_frontend_lists() {
        let frontend = include_str!("../../src/shared/api/invoke.ts");
        for code in [
            ErrorCode::FirstRunSetupRequired,
            ErrorCode::JobNotFound,
            ErrorCode::InvalidJobState,
            ErrorCode::InvalidRequest,
            ErrorCode::FileNotFound,
            ErrorCode::SourceRefused,
            ErrorCode::Storage,
            ErrorCode::Network,
            ErrorCode::Internal,
        ] {
            let name = serde_json::to_string(&code).unwrap();
            assert!(frontend.contains(&name), "{name} is missing in invoke.ts");
        }
    }

    #[test]
    fn core_errors_keep_their_text_and_get_a_code() {
        let invalid = AppError::from(CoreError::InvalidUrl("ftp://example".to_string()));
        let io = AppError::from(CoreError::Io(std::io::Error::other("disk full")));

        assert_eq!(invalid.code, ErrorCode::InvalidRequest);
        assert_eq!(invalid.message, "invalid url: ftp://example");
        assert_eq!(io.code, ErrorCode::Storage);
    }
}
