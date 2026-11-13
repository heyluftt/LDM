use std::{sync::OnceLock, time::Duration};

use reqwest::{Client, StatusCode};

/// How long a connection attempt may take before the download fails.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

/// How long a stalled body read may take before the download fails. A whole-request timeout
/// would abort large but healthy downloads, so only the gap between received bytes is bounded.
const READ_TIMEOUT: Duration = Duration::from_secs(60);

/// The shared HTTP client for direct downloads and segment requests.
///
/// One client per process reuses connections across jobs and guarantees that every request
/// carries connect and read timeouts, so a server that stops sending cannot hang a job forever.
pub fn download_client() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_TIMEOUT)
            .build()
            .unwrap_or_else(|_| Client::new())
    })
}

/// Statuses that say "not now" rather than "no": another attempt may succeed.
pub fn is_transient_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::REQUEST_TIMEOUT
            | StatusCode::TOO_MANY_REQUESTS
            | StatusCode::INTERNAL_SERVER_ERROR
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT
    )
}

/// Request failures another attempt can get past: connecting, timeouts, a connection that
/// breaks while the body arrives. A request that could not even be built, or one lost in a
/// redirect loop, fails the same way every time.
pub fn is_transient_request_error(error: &reqwest::Error) -> bool {
    !(error.is_builder() || error.is_redirect() || error.is_status())
}
