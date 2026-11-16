use ldm_types::download_job::CreateDownloadJobRequest;
use url::Url;

use crate::error::CoreError;

const MAX_URL_LEN: usize = 8192;
const MAX_TEXT_FIELD_LEN: usize = 512;
const MAX_HEADER_COUNT: usize = 16;
const MAX_HEADER_VALUE_LEN: usize = 4096;
const MAX_ID_LEN: usize = 64;
const MAX_FALLBACK_URLS: usize = 8;

const BLOCKED_HEADERS: &[&str] = &[
    "authorization",
    "cookie",
    "host",
    "proxy-authorization",
    "set-cookie",
];

pub fn validate_download_request(request: &CreateDownloadJobRequest) -> Result<(), CoreError> {
    validate_http_url("sourceUrl", &request.source_url)?;
    validate_source_fallback_urls(request)?;
    validate_optional_http_url("pageUrl", request.page_url.as_deref())?;
    validate_optional_http_url("referrer", request.referrer.as_deref())?;
    validate_optional_text("pageTitle", request.page_title.as_deref())?;
    validate_optional_text("suggestedFilename", request.suggested_filename.as_deref())?;
    validate_optional_text("contentType", request.content_type.as_deref())?;
    validate_id("adapterId", &request.adapter_id)?;
    validate_optional_id("providerId", request.provider_id.as_deref())?;
    validate_optional_id("selectedOptionId", request.selected_option_id.as_deref())?;
    validate_headers(request)?;
    Ok(())
}

fn validate_source_fallback_urls(request: &CreateDownloadJobRequest) -> Result<(), CoreError> {
    if request.source_fallback_urls.len() > MAX_FALLBACK_URLS {
        return Err(CoreError::InvalidRequest(
            "too many source fallback URLs".to_string(),
        ));
    }

    for url in &request.source_fallback_urls {
        validate_http_url("sourceFallbackUrls", url)?;
    }

    Ok(())
}

fn validate_http_url(field: &str, value: &str) -> Result<(), CoreError> {
    if value.len() > MAX_URL_LEN {
        return Err(CoreError::InvalidRequest(format!("{field} is too long")));
    }

    let url = Url::parse(value).map_err(|_| CoreError::InvalidUrl(field.to_string()))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(CoreError::InvalidRequest(format!(
            "{field} must not contain embedded credentials"
        )));
    }
    match url.scheme() {
        "http" | "https" => Ok(()),
        _ => Err(CoreError::InvalidRequest(format!(
            "{field} must use http or https"
        ))),
    }
}

fn validate_optional_http_url(field: &str, value: Option<&str>) -> Result<(), CoreError> {
    if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
        validate_http_url(field, value)?;
    }
    Ok(())
}

fn validate_optional_text(field: &str, value: Option<&str>) -> Result<(), CoreError> {
    if value.is_some_and(|value| value.len() > MAX_TEXT_FIELD_LEN) {
        return Err(CoreError::InvalidRequest(format!("{field} is too long")));
    }
    Ok(())
}

fn validate_id(field: &str, value: &str) -> Result<(), CoreError> {
    if value.is_empty() || value.len() > MAX_ID_LEN {
        return Err(CoreError::InvalidRequest(format!("{field} is invalid")));
    }
    if !value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(CoreError::InvalidRequest(format!(
            "{field} contains unsupported characters"
        )));
    }
    Ok(())
}

fn validate_optional_id(field: &str, value: Option<&str>) -> Result<(), CoreError> {
    if let Some(value) = value {
        validate_id(field, value)?;
    }
    Ok(())
}

fn validate_headers(request: &CreateDownloadJobRequest) -> Result<(), CoreError> {
    let Some(headers) = &request.headers else {
        return Ok(());
    };

    if headers.len() > MAX_HEADER_COUNT {
        return Err(CoreError::InvalidRequest(
            "too many request headers".to_string(),
        ));
    }

    for (name, value) in headers {
        let normalized = name.trim().to_ascii_lowercase();
        if BLOCKED_HEADERS.contains(&normalized.as_str()) {
            return Err(CoreError::InvalidRequest(format!(
                "{name} header is not allowed"
            )));
        }
        if value.len() > MAX_HEADER_VALUE_LEN {
            return Err(CoreError::InvalidRequest(format!(
                "{name} header value is too long"
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ldm_types::download_job::DownloadKind;

    use super::*;

    #[test]
    fn rejects_non_http_source_urls() {
        let mut request = valid_request();
        request.source_url = "file:///private/video.mp4".to_string();

        assert!(validate_download_request(&request).is_err());
    }

    #[test]
    fn rejects_sensitive_headers() {
        let mut request = valid_request();
        request.headers = Some(BTreeMap::from([(
            "Cookie".to_string(),
            "session=blocked".to_string(),
        )]));

        assert!(validate_download_request(&request).is_err());
    }

    #[test]
    fn rejects_non_http_source_fallback_urls() {
        let mut request = valid_request();
        request.source_fallback_urls = vec!["file:///private/fallback.m3u8".to_string()];

        assert!(validate_download_request(&request).is_err());
    }

    #[test]
    fn rejects_urls_with_embedded_credentials() {
        let mut request = valid_request();
        request.source_url = "https://user:pass@example.invalid/video.mp4".to_string();

        assert!(validate_download_request(&request).is_err());
    }

    fn valid_request() -> CreateDownloadJobRequest {
        CreateDownloadJobRequest {
            source_url: "https://example.invalid/video.mp4".to_string(),
            source_fallback_urls: Vec::new(),
            page_url: Some("https://example.invalid/page".to_string()),
            page_title: Some("Video".to_string()),
            referrer: None,
            suggested_filename: Some("video.mp4".to_string()),
            content_type: Some("video/mp4".to_string()),
            content_length: None,
            kind: DownloadKind::Video,
            adapter_id: "generic-html5-video".to_string(),
            provider_id: None,
            selected_option_id: None,
            headers: None,
            prefer_range_request: false,
            quality_report: None,
        }
    }
}
