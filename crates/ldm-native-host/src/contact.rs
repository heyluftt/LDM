//! Notes that a browser started this host.
//!
//! A browser starts the host only for an extension that is installed and named in the host
//! manifest, and it passes that extension's origin as the first argument. The note therefore
//! tells the desktop app that the extension is set up, which it shows a new user as a step
//! that is done. Nothing else reaches the desktop app before the first download: this host
//! answers pings and preference requests itself.

use std::{fs, path::PathBuf};

use ldm_types::browser_integration::{ExtensionContact, EXTENSION_CONTACT_FILENAME};
use ldm_types::timestamp::unix_seconds_now;

use crate::host_log;

const EXTENSION_ORIGIN_PREFIX: &str = "chrome-extension://";

/// Writes the note next to the log when an extension started this process. A failure is
/// logged and otherwise ignored; relaying messages does not depend on it.
pub fn record() {
    let Some(origin) = calling_extension_origin(std::env::args().skip(1)) else {
        return;
    };
    let Some(path) = contact_path() else {
        return;
    };
    let contact = ExtensionContact {
        at: unix_seconds_now(),
        origin,
    };
    let written = serde_json::to_vec(&contact)
        .map_err(std::io::Error::other)
        .and_then(|payload| fs::write(&path, payload));
    if let Err(error) = written {
        host_log::line(format!("could not note the extension contact: {error}"));
    }
}

fn contact_path() -> Option<PathBuf> {
    Some(
        host_log::log_path()?
            .parent()?
            .join(EXTENSION_CONTACT_FILENAME),
    )
}

/// The origin Chromium passes for the extension that opened the connection.
fn calling_extension_origin(mut arguments: impl Iterator<Item = String>) -> Option<String> {
    arguments.find(|argument| argument.starts_with(EXTENSION_ORIGIN_PREFIX))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin_of(arguments: &[&str]) -> Option<String> {
        calling_extension_origin(arguments.iter().map(|argument| (*argument).to_string()))
    }

    #[test]
    fn the_origin_is_the_argument_a_browser_passes() {
        assert_eq!(
            origin_of(&[
                "chrome-extension://dpfifoelihelgibggmccbedplhjpfmdc/",
                "--parent-window=0",
            ])
            .as_deref(),
            Some("chrome-extension://dpfifoelihelgibggmccbedplhjpfmdc/")
        );
    }

    #[test]
    fn a_start_by_hand_is_not_a_contact() {
        assert_eq!(origin_of(&[]), None);
        assert_eq!(origin_of(&["--help"]), None);
    }
}
