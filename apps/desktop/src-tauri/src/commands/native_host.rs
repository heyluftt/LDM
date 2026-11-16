use tauri::State;

use crate::app_state::AppState;
use ldm_types::browser_integration::ExtensionContact;

use crate::native_host::{register, NativeHostRegistration};

/// Registers the native host and reports the outcome.
#[tauri::command]
pub fn native_host_registration(state: State<'_, AppState>) -> NativeHostRegistration {
    let preferences = state.browser_integration_preferences();
    register(preferences.extension_id.as_deref())
}

/// When a browser last started the native host for the extension, if ever.
#[tauri::command]
pub fn extension_contact() -> Option<ExtensionContact> {
    crate::native_host::extension_contact()
}
