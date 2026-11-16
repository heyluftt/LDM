use tauri::State;

use crate::app_state::{AppState, LegalStatus};
use crate::error::AppError;

#[tauri::command]
pub fn legal_status(state: State<'_, AppState>) -> LegalStatus {
    state.legal_status()
}

#[tauri::command]
pub fn accept_legal_terms(state: State<'_, AppState>) -> Result<LegalStatus, AppError> {
    state.inner().accept_current_legal_terms()
}
