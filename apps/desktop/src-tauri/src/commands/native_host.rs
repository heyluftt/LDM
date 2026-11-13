use tauri::State;

use crate::app_state::AppState;
use ldm_types::browser_integration::ExtensionContact;

use crate::native_host::{register, NativeHostRegistration};

/// Registers the native host (again) and reports the outcome.
/// Re-running it is cheap and self-healing, so the settings view calls it whenever it opens.
#[tauri::command]
pub fn native_host_registration(state: State<'_, AppState>) -> NativeHostRegistration {
    let preferences = state.browser_integration_preferences();
    register(preferences.extension_id.as_deref())
}

/// When a browser last started the native host for the extension; `null` when it never did.
/// The getting-started panel shows this as "extension connected".
#[tauri::command]
pub fn extension_contact() -> Option<ExtensionContact> {
    crate::native_host::extension_contact()
}
