use ldm_types::browser_integration::BrowserIntegrationPreferences;
use tauri::State;

use crate::app_state::AppState;
use crate::error::AppError;

#[tauri::command]
pub fn browser_integration_preferences(
    state: State<'_, AppState>,
) -> BrowserIntegrationPreferences {
    state.browser_integration_preferences()
}

#[tauri::command]
pub fn set_browser_integration_preferences(
    state: State<'_, AppState>,
    preferences: BrowserIntegrationPreferences,
) -> Result<BrowserIntegrationPreferences, AppError> {
    let previous = state.browser_integration_preferences();
    let saved = state
        .inner()
        .set_browser_integration_preferences(preferences)?;
    if saved.extension_id != previous.extension_id {
        // The settings view fetches the outcome through `native_host_registration`.
        let registration = crate::native_host::register(saved.extension_id.as_deref());
        match &registration.error {
            Some(error) => tracing::warn!("native host registration failed: {error}"),
            None => tracing::info!(
                "native host registration updated (registered: {})",
                registration.registered
            ),
        }
    }
    Ok(saved)
}
