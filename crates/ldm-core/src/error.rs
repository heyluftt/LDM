use thiserror::Error;

/// A failed run of the external backend (yt-dlp). `message` is what the queue shows next to
/// the job; the other fields are details for the error report, where the full context fits
/// without making that message long.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderFailure {
    /// Short, user-facing explanation.
    pub message: String,
    /// The yt-dlp output line `message` was derived from.
    pub backend_error_line: Option<String>,
    /// Why the native HLS path gave up before yt-dlp ran, when it ran at all.
    pub native_hls_error: Option<String>,
    /// Why the yt-dlp run with the page URL failed, when a stream job got that last attempt.
    pub page_url_fallback_error: Option<String>,
}

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("invalid url: {0}")]
    InvalidUrl(String),
    #[error("invalid download request: {0}")]
    InvalidRequest(String),
    #[error("download request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("file operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("filename is unavailable")]
    MissingFilename,
    #[error("server returned unsuccessful status: {0}")]
    HttpStatus(reqwest::StatusCode),
    #[error("provider backend failed: {0}")]
    ProviderBackend(String),
    #[error("provider backend failed: {}", .0.message)]
    ProviderFailure(ProviderFailure),
    /// The stream is still being broadcast. LDM saves finished videos; a running stream has no
    /// end to download to, and its playlist only lists the last few seconds.
    #[error("This is a live stream that is still running. LDM downloads finished videos only.")]
    LiveStream,
    #[error("download was canceled")]
    Canceled,
}

impl CoreError {
    /// The details of a failed external-backend run, when this error carries them.
    pub fn provider_failure(&self) -> Option<&ProviderFailure> {
        match self {
            Self::ProviderFailure(failure) => Some(failure),
            _ => None,
        }
    }

    /// Attaches the reason the native HLS path gave up to an external-backend failure, so the
    /// error report can say why yt-dlp ran at all. Errors of other kinds, and a missing reason,
    /// leave `self` unchanged; the displayed message never changes.
    pub fn with_native_hls_error(self, native_hls_error: Option<String>) -> Self {
        let Some(native_hls_error) = native_hls_error else {
            return self;
        };

        match self {
            Self::ProviderBackend(message) => Self::ProviderFailure(ProviderFailure {
                message,
                native_hls_error: Some(native_hls_error),
                ..ProviderFailure::default()
            }),
            Self::ProviderFailure(mut failure) => {
                failure.native_hls_error = Some(native_hls_error);
                Self::ProviderFailure(failure)
            }
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CoreError, ProviderFailure};

    #[test]
    fn native_hls_reason_moves_backend_failures_into_the_report_without_changing_the_message() {
        let error = CoreError::ProviderBackend("ERROR: HTTP Error 404".to_string())
            .with_native_hls_error(Some("server returned unsuccessful status: 404".to_string()));

        assert_eq!(
            error.to_string(),
            "provider backend failed: ERROR: HTTP Error 404"
        );
        assert_eq!(
            error.provider_failure(),
            Some(&ProviderFailure {
                message: "ERROR: HTTP Error 404".to_string(),
                backend_error_line: None,
                native_hls_error: Some("server returned unsuccessful status: 404".to_string()),
                page_url_fallback_error: None,
            })
        );

        let error = CoreError::ProviderFailure(ProviderFailure {
            message: "rejected".to_string(),
            backend_error_line: Some("ERROR: line".to_string()),
            native_hls_error: None,
            page_url_fallback_error: Some("ERROR: Unsupported URL".to_string()),
        })
        .with_native_hls_error(Some("native".to_string()));
        let failure = error.provider_failure().unwrap();
        assert_eq!(failure.backend_error_line.as_deref(), Some("ERROR: line"));
        assert_eq!(failure.native_hls_error.as_deref(), Some("native"));
        assert_eq!(
            failure.page_url_fallback_error.as_deref(),
            Some("ERROR: Unsupported URL")
        );
    }

    #[test]
    fn native_hls_reason_leaves_other_errors_and_missing_reasons_alone() {
        assert!(matches!(
            CoreError::Canceled.with_native_hls_error(Some("native".to_string())),
            CoreError::Canceled
        ));
        assert!(matches!(
            CoreError::ProviderBackend("x".to_string()).with_native_hls_error(None),
            CoreError::ProviderBackend(message) if message == "x"
        ));
        assert!(CoreError::MissingFilename.provider_failure().is_none());
    }
}
