#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod browser_settings;
mod contact;
mod desktop;
mod host_log;
mod native_protocol;

use std::io::{self, Write};

use anyhow::Result;
use browser_settings::load_browser_integration_preferences;
use desktop::{desktop_start_help, forward_or_start_desktop};
use ldm_types::messaging::{DesktopToExtensionMessage, ExtensionToDesktopMessage, RejectionCode};
use ldm_types::SCHEMA_VERSION;
use native_protocol::{read_message, write_message};

#[tokio::main]
async fn main() -> Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = stdin.lock();
    let mut writer = stdout.lock();
    host_log::line("native host started");
    contact::record();

    while let Some(message) = read_message(&mut reader)? {
        let response = handle_message(message, &mut writer).await;
        write_message(&mut writer, &response)?;
    }

    host_log::line("native host exiting (stdin closed)");
    Ok(())
}

/// Answers one extension message. Interim messages (such as `DesktopStarting`) go straight to
/// `writer`; the final reply is returned so the caller writes exactly one of them.
async fn handle_message(
    message: ExtensionToDesktopMessage,
    writer: &mut impl Write,
) -> DesktopToExtensionMessage {
    if message.schema_version() != SCHEMA_VERSION {
        return DesktopToExtensionMessage::rejected(
            "Unsupported message schema version.".to_string(),
        );
    }

    match message {
        ExtensionToDesktopMessage::Ping { .. } => DesktopToExtensionMessage::Pong {
            schema_version: SCHEMA_VERSION,
        },
        ExtensionToDesktopMessage::GetBrowserIntegrationPreferences { .. } => {
            match load_browser_integration_preferences() {
                Ok(preferences) => DesktopToExtensionMessage::BrowserIntegrationPreferences {
                    schema_version: SCHEMA_VERSION,
                    preferences,
                },
                Err(error) => DesktopToExtensionMessage::rejected(format!(
                    "Browser integration settings are unavailable. {error}"
                )),
            }
        }
        ExtensionToDesktopMessage::DetectedCandidates { candidates, .. } => {
            DesktopToExtensionMessage::rejected(format!(
                "received {} candidate(s); no job was requested",
                candidates.len()
            ))
        }
        ExtensionToDesktopMessage::CreateJob { .. } => {
            let mut notify = |interim: DesktopToExtensionMessage| {
                if let Err(error) = write_message(writer, &interim) {
                    host_log::line(format!("could not send interim message: {error:#}"));
                }
            };
            match forward_or_start_desktop(&message, &mut notify).await {
                Ok(response) => response,
                Err(error) => {
                    host_log::line(format!("create job failed: {error:#}"));
                    DesktopToExtensionMessage::rejected_because(
                        RejectionCode::DesktopUnavailable,
                        format!(
                            "Desktop app could not be reached. {} {error:#}",
                            desktop_start_help()
                        ),
                    )
                }
            }
        }
    }
}
