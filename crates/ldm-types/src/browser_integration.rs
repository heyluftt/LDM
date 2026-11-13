use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const BROWSER_INTEGRATION_PREFERENCES_FILENAME: &str = "browser-integration.json";
pub const DESKTOP_APP_DATA_DIRECTORY: &str = "dev.ldm.desktop";

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
#[derive(TS)]
#[ts(export)]
pub enum BrowserDownloadMode {
    Off,
    Ask,
    Auto,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[derive(TS)]
#[ts(export, optional_fields)]
pub struct BrowserDownloadDomainRule {
    pub host: String,
    pub action: BrowserDownloadMode,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[derive(TS)]
#[ts(export, optional_fields)]
pub struct BrowserDownloadKinds {
    pub video: bool,
    pub audio: bool,
    pub program: bool,
    pub document: bool,
    pub archive: bool,
    pub image: bool,
    pub other: bool,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[derive(TS)]
#[ts(export, optional_fields)]
pub struct BrowserIntegrationPreferences {
    pub mode: BrowserDownloadMode,
    pub selected_kinds: BrowserDownloadKinds,
    pub domain_rules: Vec<BrowserDownloadDomainRule>,
    pub warn_on_executable: bool,
    /// The id (32 letters a-p) of one more extension that may start the native host, next to
    /// `LDM_EXTENSION_ID`: a build with a different manifest key, such as a fork.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension_id: Option<String>,
}

/// The file in which the native host notes that a browser started it, next to its log.
pub const EXTENSION_CONTACT_FILENAME: &str = "extension-contact.json";

/// The last time a browser started the native host for an extension. A browser does that only
/// for an extension that is installed and named in the host manifest, so the desktop app reads
/// this as "the extension is set up".
#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[derive(TS)]
#[ts(export)]
pub struct ExtensionContact {
    /// Unix seconds.
    pub at: u64,
    /// The extension's origin as the browser passed it: `chrome-extension://<id>/`.
    pub origin: String,
}

/// The id of the LDM extension. It follows from the public key in the extension manifest
/// (`apps/extension/src/shared/extension-identity.ts`) and is therefore the same on every
/// machine, so the desktop app registers the native host for it without being told.
pub const LDM_EXTENSION_ID: &str = "dpfifoelihelgibggmccbedplhjpfmdc";

impl BrowserIntegrationPreferences {
    pub fn normalized(mut self) -> Self {
        self.domain_rules = self
            .domain_rules
            .into_iter()
            .filter_map(|rule| {
                let host = rule.host.trim().trim_matches('.').to_ascii_lowercase();
                is_valid_domain_rule_host(&host).then_some(BrowserDownloadDomainRule {
                    host,
                    action: rule.action,
                })
            })
            .collect();
        self.domain_rules
            .sort_by(|left, right| left.host.cmp(&right.host));
        self.domain_rules
            .dedup_by(|left, right| left.host == right.host);
        self.extension_id = self
            .extension_id
            .as_deref()
            .and_then(normalize_extension_id);
        self
    }
}

/// Accepts the id as shown on the browser's extensions page, or a pasted
/// `chrome-extension://<id>/` origin, in any case; anything that is not 32 letters a-p is `None`.
pub fn normalize_extension_id(value: &str) -> Option<String> {
    let trimmed = value.trim().to_ascii_lowercase();
    let id = trimmed
        .strip_prefix("chrome-extension://")
        .unwrap_or(&trimmed)
        .trim_matches('/');
    (id.len() == 32 && id.bytes().all(|byte| (b'a'..=b'p').contains(&byte))).then(|| id.to_string())
}

impl Default for BrowserIntegrationPreferences {
    fn default() -> Self {
        Self {
            mode: BrowserDownloadMode::Auto,
            selected_kinds: BrowserDownloadKinds {
                video: true,
                audio: true,
                program: true,
                document: true,
                archive: true,
                image: true,
                other: false,
            },
            domain_rules: Vec::new(),
            warn_on_executable: true,
            extension_id: None,
        }
    }
}

fn is_valid_domain_rule_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_domain_rules() {
        let preferences = BrowserIntegrationPreferences {
            domain_rules: vec![
                BrowserDownloadDomainRule {
                    host: " Example.COM. ".to_string(),
                    action: BrowserDownloadMode::Ask,
                },
                BrowserDownloadDomainRule {
                    host: "bad host".to_string(),
                    action: BrowserDownloadMode::Auto,
                },
            ],
            ..BrowserIntegrationPreferences::default()
        }
        .normalized();

        assert_eq!(
            preferences.domain_rules,
            vec![BrowserDownloadDomainRule {
                host: "example.com".to_string(),
                action: BrowserDownloadMode::Ask,
            }]
        );
    }

    #[test]
    fn the_extension_id_is_the_one_the_extension_declares() {
        let identity = include_str!("../../../apps/extension/src/shared/extension-identity.ts");

        assert_eq!(
            normalize_extension_id(LDM_EXTENSION_ID).as_deref(),
            Some(LDM_EXTENSION_ID)
        );
        assert!(
            identity.contains(&format!("EXTENSION_ID = \"{LDM_EXTENSION_ID}\"")),
            "LDM_EXTENSION_ID and EXTENSION_ID in extension-identity.ts must be the same"
        );
    }

    #[test]
    fn normalizes_extension_ids() {
        assert_eq!(
            normalize_extension_id(" ABCDEFGHIJKLMNOPABCDEFGHIJKLMNOP "),
            Some("abcdefghijklmnopabcdefghijklmnop".to_string())
        );
        assert_eq!(
            normalize_extension_id("chrome-extension://abcdefghijklmnopabcdefghijklmnop/"),
            Some("abcdefghijklmnopabcdefghijklmnop".to_string())
        );
        assert_eq!(normalize_extension_id(""), None);
        assert_eq!(
            normalize_extension_id("abcdefghijklmnopabcdefghijklmno"),
            None
        );
        assert_eq!(
            normalize_extension_id("abcdefghijklmnopabcdefghijklmnoq"),
            None
        );
    }

    #[test]
    fn preferences_drop_an_invalid_extension_id_and_keep_the_field_optional() {
        let preferences = BrowserIntegrationPreferences {
            extension_id: Some("not an id".to_string()),
            ..BrowserIntegrationPreferences::default()
        }
        .normalized();
        assert_eq!(preferences.extension_id, None);

        let json = serde_json::to_value(&preferences).unwrap();
        assert!(json.get("extensionId").is_none());
        let parsed: BrowserIntegrationPreferences = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, preferences);

        let with_id: BrowserIntegrationPreferences = serde_json::from_str(
            r#"{"mode":"auto","selectedKinds":{"video":true,"audio":true,"program":true,"document":true,"archive":true,"image":true,"other":false},"domainRules":[],"warnOnExecutable":true,"extensionId":"abcdefghijklmnopabcdefghijklmnop"}"#,
        )
        .unwrap();
        assert_eq!(
            with_id.extension_id.as_deref(),
            Some("abcdefghijklmnopabcdefghijklmnop")
        );
    }
}
