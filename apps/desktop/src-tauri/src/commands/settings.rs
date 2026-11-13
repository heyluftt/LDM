use ldm_core::storage::{default_target_folder, validate_target_folder};
use ldm_types::download_job::DownloadKind;
use ldm_types::download_settings::{MAX_MAX_CONCURRENT_DOWNLOADS, MIN_MAX_CONCURRENT_DOWNLOADS};
use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::app_state::AppState;
use crate::commands::downloads::schedule_pending_jobs;
use crate::error::AppError;

/// One row of the target-folder settings: the folder in use and whether the user chose it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetFolder {
    pub kind: DownloadKind,
    pub path: String,
    pub default_path: String,
    pub custom: bool,
}

#[tauri::command]
pub fn target_folders(state: State<'_, AppState>) -> Vec<TargetFolder> {
    target_folder_rows(state.inner())
}

/// The parallel-download limit and the range the settings UI may offer.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConcurrencySettings {
    pub max_concurrent_downloads: u8,
    pub min: u8,
    pub max: u8,
}

#[tauri::command]
pub fn concurrency_settings(state: State<'_, AppState>) -> ConcurrencySettings {
    concurrency_rows(state.inner())
}

/// Stores the parallel-download limit (clamped to the supported range) and starts waiting
/// jobs if the limit went up.
#[tauri::command]
pub fn set_max_concurrent_downloads(
    state: State<'_, AppState>,
    value: u8,
) -> Result<ConcurrencySettings, AppError> {
    let mut settings = state.download_settings();
    settings.max_concurrent_downloads = value;
    state.set_download_settings(settings)?;
    schedule_pending_jobs(state.inner());
    Ok(concurrency_rows(state.inner()))
}

fn concurrency_rows(state: &AppState) -> ConcurrencySettings {
    ConcurrencySettings {
        max_concurrent_downloads: state.download_settings().max_concurrent_downloads,
        min: MIN_MAX_CONCURRENT_DOWNLOADS,
        max: MAX_MAX_CONCURRENT_DOWNLOADS,
    }
}

/// Stores `path` as the folder for `kind`, or clears the override when `path` is `None`.
#[tauri::command]
pub fn set_target_folder(
    state: State<'_, AppState>,
    kind: DownloadKind,
    path: Option<String>,
) -> Result<Vec<TargetFolder>, AppError> {
    apply_target_folder(state.inner(), kind, path)?;
    Ok(target_folder_rows(state.inner()))
}

/// Opens the system folder picker and stores the chosen folder for `kind`.
#[tauri::command]
pub async fn pick_target_folder(
    app: AppHandle,
    state: State<'_, AppState>,
    kind: DownloadKind,
) -> Result<Vec<TargetFolder>, AppError> {
    let current = state.target_folder_for(kind);
    let dialog = app
        .dialog()
        .file()
        .set_title(format!("Choose the {} folder", kind_label(kind)))
        .set_directory(current);

    let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_pick_folder())
        .await
        .map_err(|error| AppError::internal(format!("folder picker failed: {error}")))?;

    if let Some(picked) = picked {
        let path = picked.into_path().map_err(|error| {
            AppError::invalid_request(format!("folder picker returned an unusable path: {error}"))
        })?;
        apply_target_folder(
            state.inner(),
            kind,
            Some(path.to_string_lossy().to_string()),
        )?;
    }

    Ok(target_folder_rows(state.inner()))
}

fn apply_target_folder(
    state: &AppState,
    kind: DownloadKind,
    path: Option<String>,
) -> Result<(), AppError> {
    let mut settings = state.download_settings();
    match path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(value) => {
            let validated = validate_target_folder(value)?;
            settings
                .target_folders
                .insert(kind, validated.to_string_lossy().to_string());
        }
        None => {
            settings.target_folders.remove(&kind);
        }
    }
    state.set_download_settings(settings)?;
    Ok(())
}

fn target_folder_rows(state: &AppState) -> Vec<TargetFolder> {
    let settings = state.download_settings();
    DownloadKind::ALL
        .iter()
        .map(|kind| {
            let default_path = default_target_folder(*kind);
            let custom = settings.target_folder_override(*kind).map(str::to_string);
            TargetFolder {
                kind: *kind,
                path: custom
                    .clone()
                    .unwrap_or_else(|| default_path.to_string_lossy().to_string()),
                default_path: default_path.to_string_lossy().to_string(),
                custom: custom.is_some(),
            }
        })
        .collect()
}

fn kind_label(kind: DownloadKind) -> &'static str {
    match kind {
        DownloadKind::Video => "video",
        DownloadKind::Audio => "audio",
        DownloadKind::Program => "program",
        DownloadKind::Document => "document",
        DownloadKind::Archive => "archive",
        DownloadKind::Image => "image",
        DownloadKind::Other => "other downloads",
    }
}
