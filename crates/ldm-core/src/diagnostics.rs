//! Redaction of diagnostic text before it is stored or logged: URL query strings, local paths,
//! sensitive header values, and token-like assignments. `sanitize_diagnostic_message` treats its
//! input as free text; `sanitize_diagnostic_report` keeps a JSON report's structure intact.

use serde_json::{json, Value};
use url::Url;

const MAX_DIAGNOSTIC_MESSAGE_LEN: usize = 4096;
/// Upper bound for a whole JSON report (quality or error report). A larger report is replaced by
/// a small object that says so, instead of a cut-off string that no longer parses.
pub const MAX_DIAGNOSTIC_REPORT_LEN: usize = 256 * 1024;
const SENSITIVE_HEADER_NAMES: &[&str] = &[
    "authorization:",
    "proxy-authorization:",
    "cookie:",
    "set-cookie:",
];
const SENSITIVE_ASSIGNMENTS: &[&str] = &[
    "token=",
    "secret=",
    "password=",
    "api_key=",
    "apikey=",
    "signature=",
    "sig=",
];

pub fn sanitize_diagnostic_message(message: &str) -> String {
    truncate_message(&sanitize_diagnostic_text(message))
}

/// Sanitizes a job report. A report that is a JSON object or array keeps its structure: every
/// string value goes through the same redaction as a message, keys and non-string values stay
/// as they are, and the result is serialized compactly. Anything else is treated as a message.
/// A report over `MAX_DIAGNOSTIC_REPORT_LEN` bytes (before or after redaction) becomes
/// `{"truncated": true, ...}` so the UI can still parse it.
pub fn sanitize_diagnostic_report(value: &str) -> String {
    if value.len() > MAX_DIAGNOSTIC_REPORT_LEN {
        return truncated_report(value.len());
    }

    let mut report = match serde_json::from_str::<Value>(value) {
        Ok(report) if report.is_object() || report.is_array() => report,
        _ => return sanitize_diagnostic_message(value),
    };
    sanitize_json_strings(&mut report);
    let serialized = report.to_string();
    if serialized.len() > MAX_DIAGNOSTIC_REPORT_LEN {
        return truncated_report(serialized.len());
    }
    serialized
}

fn sanitize_json_strings(value: &mut Value) {
    match value {
        Value::String(text) => *text = sanitize_diagnostic_text(text),
        Value::Array(items) => items.iter_mut().for_each(sanitize_json_strings),
        Value::Object(fields) => fields.values_mut().for_each(sanitize_json_strings),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn truncated_report(length: usize) -> String {
    json!({
        "truncated": true,
        "originalBytes": length,
        "note": format!(
            "The report was {length} bytes, more than the {MAX_DIAGNOSTIC_REPORT_LEN} bytes LDM keeps, and was dropped."
        ),
    })
    .to_string()
}

fn sanitize_diagnostic_text(message: &str) -> String {
    message
        .lines()
        .map(sanitize_diagnostic_line)
        .collect::<Vec<_>>()
        .join("\n")
}

fn sanitize_diagnostic_line(line: &str) -> String {
    let line = redact_sensitive_header_value(line);
    let line = redact_urls(&line);
    let line = redact_local_paths(&line);
    redact_sensitive_assignments(&line)
}

fn redact_sensitive_header_value(line: &str) -> String {
    let lowercase = line.to_ascii_lowercase();
    let Some((index, name)) = SENSITIVE_HEADER_NAMES
        .iter()
        .filter_map(|name| lowercase.find(name).map(|index| (index, *name)))
        .min_by_key(|(index, _)| *index)
    else {
        return line.to_string();
    };

    format!("{}[redacted]", &line[..index + name.len()])
}

fn redact_urls(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut remaining = value;

    while let Some(index) = next_url_start(remaining) {
        output.push_str(&remaining[..index]);
        let url_and_rest = &remaining[index..];
        let end = url_and_rest
            .find(|ch: char| ch.is_whitespace() || matches!(ch, '"' | '\'' | '<' | '>'))
            .unwrap_or(url_and_rest.len());
        let candidate = &url_and_rest[..end];
        output.push_str(&redact_url(candidate));
        remaining = &url_and_rest[end..];
    }

    output.push_str(remaining);
    output
}

fn next_url_start(value: &str) -> Option<usize> {
    [value.find("https://"), value.find("http://")]
        .into_iter()
        .flatten()
        .min()
}

fn redact_url(value: &str) -> String {
    let trimmed = value.trim_end_matches([',', '.', ')', ']', '}']);
    let suffix = &value[trimmed.len()..];
    let Ok(mut url) = Url::parse(trimmed) else {
        return value.to_string();
    };
    let _ = url.set_username("");
    let _ = url.set_password(None);
    if url.query().is_some() {
        url.set_query(Some("redacted"));
    }
    url.set_fragment(None);
    format!("{url}{suffix}")
}

fn redact_local_paths(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut index = 0;
    let bytes = value.as_bytes();

    while index < bytes.len() {
        if is_local_path_start(bytes, index) {
            output.push_str("[local path]");
            index += local_path_len(&value[index..]);
        } else {
            let ch = value[index..].chars().next().expect("valid UTF-8 boundary");
            output.push(ch);
            index += ch.len_utf8();
        }
    }

    output
}

fn is_local_path_start(bytes: &[u8], index: usize) -> bool {
    let remainder = &bytes[index..];
    let has_boundary = index == 0
        || bytes[index - 1].is_ascii_whitespace()
        || matches!(bytes[index - 1], b'"' | b'\'' | b'(' | b'[' | b'{' | b'=');
    let windows_drive = remainder.len() >= 3
        && has_boundary
        && remainder[0].is_ascii_alphabetic()
        && remainder[1] == b':'
        && matches!(remainder[2], b'\\' | b'/');
    let windows_unc = has_boundary && remainder.starts_with(br"\\");
    let unix_home =
        has_boundary && (remainder.starts_with(b"/Users/") || remainder.starts_with(b"/home/"));
    windows_drive || windows_unc || unix_home
}

fn local_path_len(value: &str) -> usize {
    value
        .find(|ch: char| ch.is_whitespace() || matches!(ch, '"' | '\'' | '<' | '>'))
        .unwrap_or(value.len())
}

fn redact_sensitive_assignments(value: &str) -> String {
    let mut output = value.to_string();
    for name in SENSITIVE_ASSIGNMENTS {
        output = redact_assignment(&output, name);
    }
    output
}

fn redact_assignment(value: &str, name: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut remaining = value;

    loop {
        let lowercase = remaining.to_ascii_lowercase();
        let Some(index) = lowercase.find(name) else {
            output.push_str(remaining);
            return output;
        };
        output.push_str(&remaining[..index + name.len()]);
        output.push_str("[redacted]");
        let tail = &remaining[index + name.len()..];
        let end = tail
            .find(|ch: char| ch.is_whitespace() || matches!(ch, '&' | '"' | '\'' | '<' | '>'))
            .unwrap_or(tail.len());
        remaining = &tail[end..];
    }
}

fn truncate_message(value: &str) -> String {
    if value.len() <= MAX_DIAGNOSTIC_MESSAGE_LEN {
        return value.to_string();
    }

    let mut end = MAX_DIAGNOSTIC_MESSAGE_LEN;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &value[..end])
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use super::{
        sanitize_diagnostic_message, sanitize_diagnostic_report, MAX_DIAGNOSTIC_MESSAGE_LEN,
        MAX_DIAGNOSTIC_REPORT_LEN,
    };

    #[test]
    fn redacts_url_queries_headers_assignments_and_local_paths() {
        let sanitized = sanitize_diagnostic_message(
            "GET https://cdn.example.test/video.mp4?signature=visible&expires=1#part failed\nAuthorization: visible\noutput C:\\Users\\Example\\Downloads\\video.mp4 token=visible",
        );

        assert!(
            sanitized.contains("https://cdn.example.test/video.mp4?redacted"),
            "{sanitized}"
        );
        assert!(sanitized.contains("Authorization:[redacted]"));
        assert!(sanitized.contains("[local path]"));
        assert!(sanitized.contains("token=[redacted]"));
        assert!(!sanitized.contains("visible"));
    }

    #[test]
    fn large_json_report_keeps_its_structure_and_redacts_url_queries() {
        let candidates = (0..120)
            .map(|index| {
                json!({
                    "id": format!("candidate-{index}"),
                    "url": format!("https://hv-h.cdn.example.test/videos/{index}/master.m3u8?signature=visible{index}&expires=1"),
                    "qualityHeight": 1080,
                    "headers": ["accept", "user-agent"],
                    "nested": { "pageUrl": "https://example.test/view_video.php?viewkey=visible" }
                })
            })
            .collect::<Vec<_>>();
        let report = json!({
            "version": 1,
            "selectedAfterPrepare": { "url": "https://cdn.example.test/master.m3u8?token=visible" },
            "candidates": candidates,
        })
        .to_string();
        assert!(
            report.len() > 20 * 1024,
            "fixture is {} bytes",
            report.len()
        );
        assert!(report.len() > MAX_DIAGNOSTIC_MESSAGE_LEN);

        let sanitized = sanitize_diagnostic_report(&report);

        let parsed: Value = serde_json::from_str(&sanitized).expect("sanitized report is JSON");
        assert_eq!(parsed["version"], 1);
        assert_eq!(parsed["candidates"].as_array().map(Vec::len), Some(120));
        assert_eq!(
            parsed["candidates"][7]["url"],
            "https://hv-h.cdn.example.test/videos/7/master.m3u8?redacted"
        );
        assert_eq!(
            parsed["candidates"][7]["nested"]["pageUrl"],
            "https://example.test/view_video.php?redacted"
        );
        assert_eq!(parsed["candidates"][7]["qualityHeight"], 1080);
        assert_eq!(parsed["candidates"][7]["headers"][1], "user-agent");
        assert_eq!(
            parsed["selectedAfterPrepare"]["url"],
            "https://cdn.example.test/master.m3u8?redacted"
        );
        assert!(!sanitized.contains("visible"));
    }

    #[test]
    fn json_report_redacts_paths_and_assignments_but_not_keys() {
        let sanitized = sanitize_diagnostic_report(
            r#"{"error":"output C:\\Users\\Example\\video.mp4 token=visible","token=":"kept key","count":3,"flag":null}"#,
        );

        let parsed: Value = serde_json::from_str(&sanitized).unwrap();
        assert_eq!(parsed["error"], "output [local path] token=[redacted]");
        assert_eq!(parsed["token="], "kept key");
        assert_eq!(parsed["count"], 3);
        assert!(parsed["flag"].is_null());
    }

    #[test]
    fn oversized_json_report_becomes_a_truncation_marker() {
        let padding = "x".repeat(MAX_DIAGNOSTIC_REPORT_LEN);
        let report = json!({ "padding": padding }).to_string();

        let sanitized = sanitize_diagnostic_report(&report);

        let parsed: Value = serde_json::from_str(&sanitized).unwrap();
        assert_eq!(parsed["truncated"], true);
        assert_eq!(parsed["originalBytes"], report.len());
        assert!(parsed["note"].as_str().unwrap().contains("dropped"));
        assert!(sanitized.len() < 1024);
    }

    #[test]
    fn non_json_report_is_sanitized_as_a_message() {
        let report = format!(
            "GET https://cdn.example.test/video.mp4?signature=visible failed\n{}",
            "y".repeat(MAX_DIAGNOSTIC_MESSAGE_LEN)
        );

        let sanitized = sanitize_diagnostic_report(&report);

        assert_eq!(sanitized, sanitize_diagnostic_message(&report));
        assert!(sanitized.starts_with("GET https://cdn.example.test/video.mp4?redacted failed"));
        assert!(sanitized.ends_with("..."));
        assert!(sanitized.len() <= MAX_DIAGNOSTIC_MESSAGE_LEN + 3);

        assert_eq!(sanitize_diagnostic_report("404"), "404");
        assert_eq!(sanitize_diagnostic_report("\"quoted\""), "\"quoted\"");
    }
}
