//! Registers the bundled Native Messaging host for the browser extension.
//!
//! A Chromium browser finds a host by name under `HKCU\Software\<vendor>\NativeMessagingHosts\<name>`;
//! that key's default value is the path of a manifest JSON naming the host executable and the
//! extension origins allowed to start it. The desktop app writes the manifest to
//! `%LOCALAPPDATA%\LDM\native-host\` and sets the keys for Chrome, Edge, Brave, and Brave Origin
//! on every start and whenever the additional extension id in the browser integration settings
//! changes. The LDM extension has a fixed id (`LDM_EXTENSION_ID`), so this needs no input.
//! `scripts/dev/register-native-host.ps1` does the same by hand for a development host.

use std::path::{Path, PathBuf};

use ldm_types::browser_integration::{
    ExtensionContact, EXTENSION_CONTACT_FILENAME, LDM_EXTENSION_ID,
};
use serde::Serialize;

use crate::windows_registry;

pub const NATIVE_HOST_NAME: &str = "dev.ldm.native_host";
const MANIFEST_FILENAME: &str = "dev.ldm.native_host.json";
const MANIFEST_DIR: &str = r"LDM\native-host";
const HOST_EXE_NAME: &str = "ldm-native-host.exe";

/// `HKCU\<key>\dev.ldm.native_host` is set for every supported browser.
const BROWSER_REGISTRY_KEYS: &[(&str, &str)] = &[
    ("Chrome", r"Software\Google\Chrome\NativeMessagingHosts"),
    ("Edge", r"Software\Microsoft\Edge\NativeMessagingHosts"),
    (
        "Brave",
        r"Software\BraveSoftware\Brave-Browser\NativeMessagingHosts",
    ),
    (
        "Brave Origin",
        r"Software\BraveSoftware\Brave-Origin\NativeMessagingHosts",
    ),
];

/// The manifest a Chromium browser reads to start the host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NativeHostManifest {
    pub name: String,
    pub description: String,
    pub path: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub allowed_origins: Vec<String>,
}

impl NativeHostManifest {
    pub fn new(host_exe: &Path, extension_ids: &[String]) -> Self {
        Self {
            name: NATIVE_HOST_NAME.to_string(),
            description: "LDM native messaging host".to_string(),
            path: host_exe.to_string_lossy().to_string(),
            kind: "stdio".to_string(),
            allowed_origins: extension_ids
                .iter()
                .map(|extension_id| format!("chrome-extension://{extension_id}/"))
                .collect(),
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("manifest has only string fields")
    }
}

/// What the settings UI shows about the last registration attempt.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeHostRegistration {
    /// The manifest is written and every browser key points at it.
    pub registered: bool,
    /// The extensions that may start the host: the LDM extension, then the additional one.
    pub extension_ids: Vec<String>,
    pub host_path: Option<String>,
    pub manifest_path: Option<String>,
    /// Browsers whose key was set.
    pub browsers: Vec<String>,
    pub error: Option<String>,
}

/// The extensions the manifest admits: the LDM extension and, when set and different, the one
/// from the settings.
fn allowed_extension_ids(additional_extension_id: Option<&str>) -> Vec<String> {
    let mut extension_ids = vec![LDM_EXTENSION_ID.to_string()];
    if let Some(additional) = additional_extension_id.filter(|id| *id != LDM_EXTENSION_ID) {
        extension_ids.push(additional.to_string());
    }
    extension_ids
}

/// Writes the manifest for the LDM extension (and `additional_extension_id`, when there is
/// one) and points every supported browser at it.
pub fn register(additional_extension_id: Option<&str>) -> NativeHostRegistration {
    let extension_ids = allowed_extension_ids(additional_extension_id);
    let mut registration = NativeHostRegistration {
        extension_ids: extension_ids.clone(),
        ..NativeHostRegistration::default()
    };

    let host_exe = match resolve_host_exe() {
        Ok(path) => path,
        Err(error) => {
            registration.error = Some(error);
            return registration;
        }
    };
    registration.host_path = Some(host_exe.display().to_string());

    let manifest_path = match write_manifest(&host_exe, &extension_ids) {
        Ok(path) => path,
        Err(error) => {
            registration.error = Some(error);
            return registration;
        }
    };
    registration.manifest_path = Some(manifest_path.display().to_string());

    let manifest_value = manifest_path.to_string_lossy();
    let mut failures = Vec::new();
    for (browser, key) in BROWSER_REGISTRY_KEYS {
        let subkey = format!(r"{key}\{NATIVE_HOST_NAME}");
        match windows_registry::write_string_value(&subkey, "", &manifest_value) {
            Ok(()) => registration.browsers.push((*browser).to_string()),
            Err(error) => failures.push(format!("{browser} ({error})")),
        }
    }
    registration.registered = failures.is_empty();
    if !failures.is_empty() {
        registration.error = Some(format!(
            "Could not write the registry key for {}.",
            failures.join(", ")
        ));
    }
    registration
}

/// `%LOCALAPPDATA%\LDM\native-host\dev.ldm.native_host.json`.
pub fn manifest_path() -> Result<PathBuf, String> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "LOCALAPPDATA is not set.".to_string())?;
    Ok(PathBuf::from(local_app_data)
        .join(MANIFEST_DIR)
        .join(MANIFEST_FILENAME))
}

/// When a browser last started the native host for an extension, as the host noted it next to
/// the manifest; `None` when that never happened.
pub fn extension_contact() -> Option<ExtensionContact> {
    read_extension_contact(
        &manifest_path()
            .ok()?
            .with_file_name(EXTENSION_CONTACT_FILENAME),
    )
}

fn read_extension_contact(path: &Path) -> Option<ExtensionContact> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn write_manifest(host_exe: &Path, extension_ids: &[String]) -> Result<PathBuf, String> {
    let path = manifest_path()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|error| format!("Could not create {}: {error}", dir.display()))?;
    }
    std::fs::write(
        &path,
        NativeHostManifest::new(host_exe, extension_ids).to_json(),
    )
    .map_err(|error| format!("Could not write {}: {error}", path.display()))?;
    Ok(path)
}

/// The host to register: `ldm-native-host.exe` next to the running app, which is how the
/// installer lays the two out. A build under `target\` registers the release host of the same
/// target directory when it exists, so a `tauri dev` app is reachable from the extension too.
pub fn resolve_host_exe() -> Result<PathBuf, String> {
    let current_exe = std::env::current_exe()
        .map_err(|error| format!("Could not resolve the running executable: {error}"))?;
    resolve_host_exe_for(&current_exe)
}

fn resolve_host_exe_for(current_exe: &Path) -> Result<PathBuf, String> {
    if let Some(target_dir) = cargo_target_dir(current_exe) {
        let release_host = target_dir.join("release").join(HOST_EXE_NAME);
        if release_host.is_file() {
            return Ok(release_host);
        }
        return Err(format!(
            "Development build without a release native host: {} does not exist. Run `cargo build --release -p ldm-native-host` or register a host with scripts\\dev\\register-native-host.ps1.",
            release_host.display()
        ));
    }

    let sibling = current_exe
        .parent()
        .ok_or_else(|| "The running executable has no parent folder.".to_string())?
        .join(HOST_EXE_NAME);
    if sibling.is_file() {
        Ok(sibling)
    } else {
        Err(format!(
            "{} is missing next to the app. Reinstall LDM to restore it.",
            sibling.display()
        ))
    }
}

/// The nearest ancestor named `target` (a Cargo build output directory), if any.
fn cargo_target_dir(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|dir| {
            dir.file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("target"))
        })
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    #[test]
    fn manifest_names_the_host_and_allows_the_extension_origin() {
        let manifest = NativeHostManifest::new(
            Path::new(r"C:\Users\x\AppData\Local\LDM\ldm-native-host.exe"),
            &["abcdefghijklmnopabcdefghijklmnop".to_string()],
        );
        let json: Value = serde_json::from_str(&manifest.to_json()).unwrap();

        assert_eq!(json["name"], "dev.ldm.native_host");
        assert_eq!(json["type"], "stdio");
        assert_eq!(
            json["path"],
            r"C:\Users\x\AppData\Local\LDM\ldm-native-host.exe"
        );
        assert_eq!(
            json["allowed_origins"],
            serde_json::json!(["chrome-extension://abcdefghijklmnopabcdefghijklmnop/"])
        );
        assert_eq!(json.as_object().unwrap().len(), 5);
    }

    #[test]
    fn the_extension_contact_is_what_the_native_host_noted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(EXTENSION_CONTACT_FILENAME);

        assert_eq!(read_extension_contact(&path), None);

        std::fs::write(
            &path,
            format!(r#"{{"at":1763060000,"origin":"chrome-extension://{LDM_EXTENSION_ID}/"}}"#),
        )
        .unwrap();
        assert_eq!(
            read_extension_contact(&path),
            Some(ExtensionContact {
                at: 1_763_060_000,
                origin: format!("chrome-extension://{LDM_EXTENSION_ID}/"),
            })
        );

        // A file cut short by a crash is no contact, not an error.
        std::fs::write(&path, b"{\"at\":17").unwrap();
        assert_eq!(read_extension_contact(&path), None);
    }

    #[test]
    fn installed_app_registers_the_host_next_to_itself() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("ldm-desktop.exe");
        let host = dir.path().join(HOST_EXE_NAME);
        std::fs::write(&app, b"").unwrap();

        let missing = resolve_host_exe_for(&app).unwrap_err();
        assert!(missing.contains("missing next to the app"), "{missing}");

        std::fs::write(&host, b"").unwrap();
        assert_eq!(resolve_host_exe_for(&app).unwrap(), host);
    }

    #[test]
    fn development_build_registers_the_release_host_of_its_target_dir() {
        let dir = tempfile::tempdir().unwrap();
        let debug_dir = dir.path().join("target").join("debug");
        let release_dir = dir.path().join("target").join("release");
        std::fs::create_dir_all(&debug_dir).unwrap();
        std::fs::create_dir_all(&release_dir).unwrap();
        let app = debug_dir.join("ldm-desktop.exe");
        std::fs::write(&app, b"").unwrap();
        // A debug host next to the app must not be registered: the browser would keep it
        // alive and block the next cargo build of the workspace.
        std::fs::write(debug_dir.join(HOST_EXE_NAME), b"").unwrap();

        let missing = resolve_host_exe_for(&app).unwrap_err();
        assert!(missing.contains("Development build"), "{missing}");

        let release_host = release_dir.join(HOST_EXE_NAME);
        std::fs::write(&release_host, b"").unwrap();
        assert_eq!(resolve_host_exe_for(&app).unwrap(), release_host);
    }

    #[test]
    fn the_ldm_extension_is_always_allowed_and_an_additional_one_comes_second() {
        let fork = "abcdefghijklmnopabcdefghijklmnop";

        assert_eq!(allowed_extension_ids(None), [LDM_EXTENSION_ID]);
        assert_eq!(allowed_extension_ids(Some(fork)), [LDM_EXTENSION_ID, fork]);
        assert_eq!(
            allowed_extension_ids(Some(LDM_EXTENSION_ID)),
            [LDM_EXTENSION_ID]
        );
    }

    #[test]
    fn the_manifest_lists_every_allowed_extension() {
        let manifest = NativeHostManifest::new(
            Path::new(r"C:\Users\x\AppData\Local\LDM\ldm-native-host.exe"),
            &allowed_extension_ids(Some("abcdefghijklmnopabcdefghijklmnop")),
        );

        assert_eq!(
            manifest.allowed_origins,
            [
                format!("chrome-extension://{LDM_EXTENSION_ID}/"),
                "chrome-extension://abcdefghijklmnopabcdefghijklmnop/".to_string(),
            ]
        );
    }
}
