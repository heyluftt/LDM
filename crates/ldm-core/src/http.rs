use std::{sync::OnceLock, time::Duration};

use reqwest::{Client, StatusCode};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

/// Allowed gap between received bytes. No whole-request timeout, so large downloads survive.
const READ_TIMEOUT: Duration = Duration::from_secs(60);

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

/// Failures another attempt can get past: connect, timeout, broken body.
pub fn is_transient_request_error(error: &reqwest::Error) -> bool {
    !(error.is_builder() || error.is_redirect() || error.is_status())
}
