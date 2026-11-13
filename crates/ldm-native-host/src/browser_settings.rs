use std::{io, path::PathBuf};

use anyhow::{Context, Result};
use ldm_types::browser_integration::{
    BrowserIntegrationPreferences, BROWSER_INTEGRATION_PREFERENCES_FILENAME,
    DESKTOP_APP_DATA_DIRECTORY,
};

pub fn load_browser_integration_preferences() -> Result<BrowserIntegrationPreferences> {
    let Some(app_data_dir) = std::env::var_os("APPDATA").map(PathBuf::from) else {
        return Ok(BrowserIntegrationPreferences::default());
    };
    let path = app_data_dir
        .join(DESKTOP_APP_DATA_DIRECTORY)
        .join(BROWSER_INTEGRATION_PREFERENCES_FILENAME);
    let payload = match std::fs::read(&path) {
        Ok(payload) => payload,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(BrowserIntegrationPreferences::default())
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to read browser integration settings '{}'",
                    path.display()
                )
            })
        }
    };

    serde_json::from_slice::<BrowserIntegrationPreferences>(&payload)
        .map(BrowserIntegrationPreferences::normalized)
        .with_context(|| {
            format!(
                "failed to parse browser integration settings '{}'",
                path.display()
            )
        })
}
