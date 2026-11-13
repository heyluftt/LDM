#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app_state;
mod commands;
mod error;
mod ipc;
mod job_events;
mod logging;
mod native_host;
mod windows_registry;

use app_state::AppState;
use commands::startup::is_background_launch;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager,
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app_state = AppState::default();
    let exit_state = app_state.clone();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if !args.iter().any(|arg| arg == "--background") {
                show_main_window(app);
            }
        }))
        .manage(app_state.clone())
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir().map_err(|error| {
                std::io::Error::other(format!("could not resolve app data directory: {error}"))
            })?;
            match logging::init(&app_data_dir) {
                Ok(guard) => app.manage(guard),
                Err(error) => {
                    eprintln!("logging unavailable: {error}");
                    true
                }
            };
            let state = app.state::<AppState>().inner().clone();
            state
                .initialize_persistence(app_data_dir.join("download-state.json"))
                .map_err(std::io::Error::other)?;
            // A state file from before the history had a limit can hold far more entries.
            commands::downloads::prune_history(&state);
            state.start_persist_task();
            state.events.attach(app.handle().clone());
            register_native_host(&state);
            ipc::start_native_host_ipc(state, app.handle().clone());
            install_background_tray(app)?;
            keep_main_window_in_background(app);
            if is_background_launch() {
                hide_main_window(app);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::browser_integration::browser_integration_preferences,
            commands::browser_integration::set_browser_integration_preferences,
            commands::downloads::list_jobs,
            commands::downloads::list_history_jobs,
            commands::downloads::create_job,
            commands::downloads::pause_job,
            commands::downloads::resume_job,
            commands::downloads::retry_job,
            commands::downloads::cancel_job,
            commands::downloads::remove_history_job,
            commands::downloads::clear_history,
            commands::downloads::open_job_file,
            commands::downloads::show_job_in_folder,
            commands::legal::legal_status,
            commands::native_host::native_host_registration,
            commands::native_host::extension_contact,
            commands::legal::accept_legal_terms,
            commands::settings::target_folders,
            commands::settings::set_target_folder,
            commands::settings::pick_target_folder,
            commands::settings::concurrency_settings,
            commands::settings::set_max_concurrent_downloads,
            commands::startup::set_startup_enabled,
            commands::startup::startup_status,
            commands::tools::tool_status,
            commands::updates::check_for_update,
            commands::updates::open_release_page,
            commands::window::quit_app,
            commands::ytdlp::ytdlp_version,
            commands::ytdlp::latest_ytdlp_version
        ])
        .build(tauri::generate_context!())
        .expect("failed to build desktop app");

    app.run(move |_app, event| {
        // Pending throttled progress writes must not be lost on quit.
        if let tauri::RunEvent::Exit = event {
            if let Err(error) = exit_state.persist_now() {
                tracing::warn!("could not persist download state on exit: {error}");
            }
        }
    });
}

fn main() {
    run();
}

/// Writes the Native Messaging manifest and the browser registry keys for the LDM extension
/// (and the additional id from the settings), so a fresh install needs no setup. Failures are
/// logged, never fatal.
fn register_native_host(state: &AppState) {
    let extension_id = state.browser_integration_preferences().extension_id;
    let registration = native_host::register(extension_id.as_deref());
    match (&registration.error, registration.registered) {
        (Some(error), _) => tracing::warn!("native host registration failed: {error}"),
        (None, true) => tracing::info!(
            "native host registered for {} ({})",
            registration.browsers.join(", "),
            registration.host_path.as_deref().unwrap_or("?")
        ),
        (None, false) => tracing::info!("native host not registered"),
    }
}

fn install_background_tray(app: &mut tauri::App) -> tauri::Result<()> {
    let show_item = MenuItem::with_id(app, "show", "Show LDM", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit LDM", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_item, &quit_item])?;
    let Some(icon) = app.default_window_icon().cloned() else {
        return Ok(());
    };

    TrayIconBuilder::with_id("ldm-background")
        .tooltip("LDM")
        .icon(icon)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_main_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

fn keep_main_window_in_background(app: &tauri::App) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let background_window = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = background_window.hide();
        }
    });
}

fn hide_main_window(app: &tauri::App) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
