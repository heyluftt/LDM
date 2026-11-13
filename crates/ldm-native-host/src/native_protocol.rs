use std::io::{self, Read, Write};

use anyhow::{bail, Context, Result};
use ldm_types::messaging::{DesktopToExtensionMessage, ExtensionToDesktopMessage};

pub const MAX_NATIVE_MESSAGE_BYTES: usize = 1024 * 1024;

pub fn read_message(reader: &mut impl Read) -> Result<Option<ExtensionToDesktopMessage>> {
    let mut length_buffer = [0_u8; 4];
    match reader.read_exact(&mut length_buffer) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error).context("failed to read native message length"),
    }

    let length = u32::from_le_bytes(length_buffer) as usize;
    if length > MAX_NATIVE_MESSAGE_BYTES {
        bail!("native message exceeded size limit");
    }
    let mut payload = vec![0_u8; length];
    reader
        .read_exact(&mut payload)
        .context("failed to read native message payload")?;

    serde_json::from_slice(&payload)
        .context("failed to parse native message")
        .map(Some)
}

pub fn write_message(writer: &mut impl Write, message: &DesktopToExtensionMessage) -> Result<()> {
    let payload = serde_json::to_vec(message).context("failed to serialize native response")?;
    let length = (payload.len() as u32).to_le_bytes();
    writer
        .write_all(&length)
        .context("failed to write native response length")?;
    writer
        .write_all(&payload)
        .context("failed to write native response payload")?;
    writer.flush().context("failed to flush native response")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use ldm_types::{
        adapter::{DetectedDownloadCandidate, DetectionSource},
        browser_integration::BrowserIntegrationPreferences,
        download_job::{CreateDownloadJobRequest, DownloadKind},
        SCHEMA_VERSION,
    };
    use serde_json::Value;

    use super::*;

    #[test]
    fn reads_every_extension_message_variant() {
        for message in extension_messages() {
            let mut reader = Cursor::new(frame(&message));

            let read = read_message(&mut reader)
                .unwrap()
                .expect("one framed message");

            assert_eq!(json(&read), json(&message));
            assert!(read_message(&mut reader).unwrap().is_none());
        }
    }

    #[test]
    fn reads_consecutive_frames_in_order() {
        let messages = extension_messages();
        let stream = messages.iter().flat_map(frame).collect::<Vec<_>>();
        let mut reader = Cursor::new(stream);

        for message in &messages {
            let read = read_message(&mut reader).unwrap().unwrap();
            assert_eq!(json(&read), json(message));
        }
        assert!(read_message(&mut reader).unwrap().is_none());
    }

    #[test]
    fn writes_every_desktop_message_variant_as_a_length_prefixed_frame() {
        for message in desktop_messages() {
            let mut writer = Cursor::new(Vec::new());

            write_message(&mut writer, &message).unwrap();

            let bytes = writer.into_inner();
            let length = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
            assert_eq!(length, bytes.len() - 4);
            let decoded: DesktopToExtensionMessage = serde_json::from_slice(&bytes[4..]).unwrap();
            assert_eq!(json(&decoded), json(&message));
        }
    }

    #[test]
    fn written_frames_use_the_extension_contract_field_names() {
        let mut writer = Cursor::new(Vec::new());
        write_message(
            &mut writer,
            &DesktopToExtensionMessage::JobAccepted {
                schema_version: SCHEMA_VERSION,
                job_id: uuid::Uuid::nil(),
            },
        )
        .unwrap();
        let payload: Value = serde_json::from_slice(&writer.into_inner()[4..]).unwrap();

        assert_eq!(payload["type"], "jobAccepted");
        assert_eq!(payload["schemaVersion"], SCHEMA_VERSION);
        assert_eq!(payload["jobId"], uuid::Uuid::nil().to_string());

        let mut writer = Cursor::new(Vec::new());
        write_message(
            &mut writer,
            &DesktopToExtensionMessage::DesktopStarting {
                schema_version: SCHEMA_VERSION,
                detail: "LDM.exe".to_string(),
            },
        )
        .unwrap();
        let payload: Value = serde_json::from_slice(&writer.into_inner()[4..]).unwrap();
        assert_eq!(payload["type"], "desktopStarting");
    }

    #[test]
    fn empty_stream_is_end_of_input() {
        let mut reader = Cursor::new(Vec::<u8>::new());

        assert!(read_message(&mut reader).unwrap().is_none());
    }

    #[test]
    fn rejects_frames_above_the_size_limit() {
        let mut bytes = ((MAX_NATIVE_MESSAGE_BYTES + 1) as u32)
            .to_le_bytes()
            .to_vec();
        bytes.extend_from_slice(b"{}");

        let error = read_message(&mut Cursor::new(bytes)).unwrap_err();

        assert!(error.to_string().contains("size limit"), "{error:#}");
    }

    #[test]
    fn accepts_a_frame_exactly_at_the_size_limit() {
        let mut payload = serde_json::to_vec(&ExtensionToDesktopMessage::Ping {
            schema_version: SCHEMA_VERSION,
        })
        .unwrap();
        payload.resize(MAX_NATIVE_MESSAGE_BYTES, b' ');
        let mut bytes = (payload.len() as u32).to_le_bytes().to_vec();
        bytes.extend(payload);

        let read = read_message(&mut Cursor::new(bytes)).unwrap();

        assert!(matches!(
            read,
            Some(ExtensionToDesktopMessage::Ping { schema_version }) if schema_version == SCHEMA_VERSION
        ));
    }

    #[test]
    fn truncated_payload_is_an_error() {
        let mut bytes = frame(&ExtensionToDesktopMessage::Ping {
            schema_version: SCHEMA_VERSION,
        });
        bytes.truncate(bytes.len() - 3);

        let error = read_message(&mut Cursor::new(bytes)).unwrap_err();

        assert!(error.to_string().contains("payload"), "{error:#}");
    }

    #[test]
    fn malformed_payload_is_an_error() {
        let mut bytes = 8_u32.to_le_bytes().to_vec();
        bytes.extend_from_slice(b"not json");

        let error = read_message(&mut Cursor::new(bytes)).unwrap_err();

        assert!(error.to_string().contains("parse"), "{error:#}");
    }

    #[test]
    fn unknown_message_type_is_an_error() {
        let payload = br#"{"type":"selfDestruct","schemaVersion":1}"#;
        let mut bytes = (payload.len() as u32).to_le_bytes().to_vec();
        bytes.extend_from_slice(payload);

        assert!(read_message(&mut Cursor::new(bytes)).is_err());
    }

    fn frame(message: &ExtensionToDesktopMessage) -> Vec<u8> {
        let payload = serde_json::to_vec(message).unwrap();
        let mut frame = (payload.len() as u32).to_le_bytes().to_vec();
        frame.extend(payload);
        frame
    }

    fn json(value: &impl serde::Serialize) -> Value {
        serde_json::to_value(value).unwrap()
    }

    fn extension_messages() -> Vec<ExtensionToDesktopMessage> {
        vec![
            ExtensionToDesktopMessage::Ping {
                schema_version: SCHEMA_VERSION,
            },
            ExtensionToDesktopMessage::GetBrowserIntegrationPreferences {
                schema_version: SCHEMA_VERSION,
            },
            ExtensionToDesktopMessage::DetectedCandidates {
                schema_version: SCHEMA_VERSION,
                candidates: vec![candidate()],
            },
            ExtensionToDesktopMessage::CreateJob {
                schema_version: SCHEMA_VERSION,
                job: Box::new(job_request()),
            },
        ]
    }

    fn desktop_messages() -> Vec<DesktopToExtensionMessage> {
        vec![
            DesktopToExtensionMessage::Pong {
                schema_version: SCHEMA_VERSION,
            },
            DesktopToExtensionMessage::BrowserIntegrationPreferences {
                schema_version: SCHEMA_VERSION,
                preferences: BrowserIntegrationPreferences::default(),
            },
            DesktopToExtensionMessage::JobAccepted {
                schema_version: SCHEMA_VERSION,
                job_id: uuid::Uuid::new_v4(),
            },
            DesktopToExtensionMessage::rejected("no".to_string()),
            DesktopToExtensionMessage::DesktopStarting {
                schema_version: SCHEMA_VERSION,
                detail: r"C:\LDM\ldm-desktop.exe".to_string(),
            },
        ]
    }

    fn candidate() -> DetectedDownloadCandidate {
        DetectedDownloadCandidate {
            id: "candidate-1".to_string(),
            url: "https://example.test/video.mp4".to_string(),
            source_fallback_urls: vec!["https://example.test/video-alt.mp4".to_string()],
            source: DetectionSource::Html5Video,
            quality_height: Some(1080),
            bitrate: None,
            quality_label: Some("1080p".to_string()),
            headers: Some(std::collections::BTreeMap::from([(
                "Accept-Language".to_string(),
                "de-DE".to_string(),
            )])),
            title: Some("Owned Video".to_string()),
            page_url: Some("https://example.test/watch".to_string()),
            content_type: Some("video/mp4".to_string()),
            content_length: Some(1024),
            suggested_filename: Some("video.mp4".to_string()),
            adapter_id: "generic-html5-video".to_string(),
            provider_id: None,
            requires_usage_notice: Some(false),
            kind: DownloadKind::Video,
        }
    }

    fn job_request() -> CreateDownloadJobRequest {
        CreateDownloadJobRequest {
            source_url: "https://example.test/video.mp4".to_string(),
            source_fallback_urls: Vec::new(),
            page_url: Some("https://example.test/watch".to_string()),
            page_title: Some("Owned Video".to_string()),
            referrer: None,
            suggested_filename: Some("video.mp4".to_string()),
            content_type: Some("video/mp4".to_string()),
            content_length: Some(1024),
            kind: DownloadKind::Video,
            adapter_id: "generic-html5-video".to_string(),
            provider_id: None,
            selected_option_id: None,
            headers: None,
            prefer_range_request: true,
            quality_report: None,
        }
    }
}
