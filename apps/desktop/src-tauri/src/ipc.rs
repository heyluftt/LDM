use std::ffi::c_void;
use std::io;
use std::ptr;
use std::time::Duration;

use ldm_types::messaging::{DesktopToExtensionMessage, ExtensionToDesktopMessage, RejectionCode};
use ldm_types::SCHEMA_VERSION;
use tauri::{AppHandle, Manager};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, TokenUser, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use crate::app_state::AppState;
use crate::commands::downloads::start_job_from_extension;
use crate::error::ErrorCode;

const PIPE_NAME: &str = r"\\.\pipe\ldm-native-host";
const MAX_PIPE_MESSAGE_BYTES: usize = 1024 * 1024;
const PIPE_RETRY_DELAY: Duration = Duration::from_secs(2);

/// Listens on the named pipe the native host connects to. The pipe admits only the current
/// user (`current_user_pipe_sddl`), and the first instance refuses to open while another
/// process owns the name, so a pipe squatted before the app started shows up in the log instead
/// of silently sharing the name.
pub fn start_native_host_ipc(state: AppState, app_handle: AppHandle) {
    let sddl = match current_user_pipe_sddl() {
        Ok(sddl) => {
            tracing::debug!("native host pipe DACL: {sddl}");
            Some(sddl)
        }
        Err(error) => {
            tracing::warn!(
                "native host pipe keeps the default DACL; could not look up the current user: {error}"
            );
            None
        }
    };

    tauri::async_runtime::spawn(async move {
        let mut server = create_server_with_retries(sddl.as_deref(), true).await;
        loop {
            if let Err(error) = server.connect().await {
                tracing::warn!("native host pipe connect failed: {error}");
                tokio::time::sleep(PIPE_RETRY_DELAY).await;
                server = create_server_with_retries(sddl.as_deref(), false).await;
                continue;
            }

            // The next instance is opened before this connection is handled, so a host that
            // connects meanwhile finds the pipe instead of "file not found".
            let next = create_server_with_retries(sddl.as_deref(), false).await;
            let connected = std::mem::replace(&mut server, next);
            let state = state.clone();
            let app_handle = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                handle_connection(connected, state, app_handle).await;
            });
        }
    });
}

async fn create_server_with_retries(sddl: Option<&str>, first_instance: bool) -> NamedPipeServer {
    loop {
        match create_server(PIPE_NAME, sddl, first_instance) {
            Ok(server) => return server,
            Err(error) => {
                let hint = if first_instance {
                    " (does another process own the pipe?)"
                } else {
                    ""
                };
                tracing::warn!("failed to create native host pipe{hint}: {error}");
                tokio::time::sleep(PIPE_RETRY_DELAY).await;
            }
        }
    }
}

fn create_server(
    name: &str,
    sddl: Option<&str>,
    first_instance: bool,
) -> io::Result<NamedPipeServer> {
    let mut options = ServerOptions::new();
    options.first_pipe_instance(first_instance);
    let Some(sddl) = sddl else {
        return options.create(name);
    };

    let descriptor = SecurityDescriptor::from_sddl(sddl)?;
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    // SAFETY: `attributes` is a valid SECURITY_ATTRIBUTES whose descriptor stays allocated
    // until after the call; CreateNamedPipe copies what it needs into the pipe object.
    unsafe {
        options.create_with_security_attributes_raw(name, ptr::from_mut(&mut attributes).cast())
    }
}

/// SDDL for a DACL that grants full access to the current user and nothing to anyone else.
/// Without it a named pipe gets Windows' default DACL, which lets every account on the machine
/// open the pipe and send job requests or read the download URLs and forwarded headers the
/// native host relays.
fn current_user_pipe_sddl() -> io::Result<String> {
    Ok(format!("D:P(A;;GA;;;{})", current_user_sid()?))
}

fn current_user_sid() -> io::Result<String> {
    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: GetCurrentProcess returns a pseudo handle that needs no closing; `token`
    // receives a real handle, closed by `TokenHandle`.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = TokenHandle(token);

    let mut needed = 0_u32;
    // SAFETY: a zero-length query fails by design and only reports the needed size.
    unsafe { GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut needed) };
    if needed == 0 {
        return Err(io::Error::last_os_error());
    }

    // u64 slots keep the buffer aligned for the pointers inside TOKEN_USER.
    let mut buffer = vec![0_u64; (needed as usize).div_ceil(size_of::<u64>())];
    // SAFETY: the buffer holds at least `needed` bytes, the size the first call asked for.
    let ok = unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            needed,
            &mut needed,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: on success the buffer starts with a TOKEN_USER whose SID points into the same
    // buffer, which outlives every use below.
    let user = unsafe { ptr::read(buffer.as_ptr().cast::<TOKEN_USER>()) };
    let mut sid_string: *mut u16 = ptr::null_mut();
    // SAFETY: the SID is valid while `buffer` lives; `sid_string` receives a LocalAlloc string.
    if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut sid_string) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `sid_string` is NUL-terminated and is read in full before it is freed.
    let sid = unsafe {
        let len = (0..)
            .take_while(|&index| *sid_string.add(index) != 0)
            .count();
        let sid = String::from_utf16_lossy(std::slice::from_raw_parts(sid_string, len));
        LocalFree(sid_string as HLOCAL);
        sid
    };
    Ok(sid)
}

/// A security descriptor built from SDDL, allocated by Windows and freed on drop.
struct SecurityDescriptor(*mut c_void);

impl SecurityDescriptor {
    fn from_sddl(sddl: &str) -> io::Result<Self> {
        let wide: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
        let mut descriptor: *mut c_void = ptr::null_mut();
        // SAFETY: `wide` is NUL-terminated and outlives the call; `descriptor` receives a
        // LocalAlloc block that `Drop` frees.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                ptr::null_mut(),
            )
        };
        if ok == 0 || descriptor.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(descriptor))
    }
}

impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        // SAFETY: the block came from ConvertStringSecurityDescriptorToSecurityDescriptorW,
        // which allocates with LocalAlloc, and is freed exactly once.
        unsafe {
            LocalFree(self.0 as HLOCAL);
        }
    }
}

struct TokenHandle(HANDLE);

impl Drop for TokenHandle {
    fn drop(&mut self) {
        // SAFETY: the handle came from OpenProcessToken and is closed exactly once.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

async fn handle_connection(mut server: NamedPipeServer, state: AppState, app_handle: AppHandle) {
    let response = match read_request(&mut server).await {
        Ok(message) => handle_message(message, state, app_handle).await,
        Err(error) => {
            DesktopToExtensionMessage::rejected(format!("Invalid desktop IPC request: {error}"))
        }
    };

    if let Ok(mut payload) = serde_json::to_vec(&response) {
        payload.push(b'\n');
        let _ = server.write_all(&payload).await;
        let _ = server.flush().await;
    }
}

async fn read_request(server: &mut NamedPipeServer) -> Result<ExtensionToDesktopMessage, String> {
    let mut payload = Vec::new();

    loop {
        let mut byte = [0_u8; 1];
        let read = server
            .read(&mut byte)
            .await
            .map_err(|error| error.to_string())?;
        if read == 0 || byte[0] == b'\n' {
            break;
        }
        payload.push(byte[0]);
        if payload.len() > MAX_PIPE_MESSAGE_BYTES {
            return Err("desktop IPC request exceeded message size limit".to_string());
        }
    }

    serde_json::from_slice(&payload).map_err(|error| error.to_string())
}

async fn handle_message(
    message: ExtensionToDesktopMessage,
    state: AppState,
    app_handle: AppHandle,
) -> DesktopToExtensionMessage {
    if message.schema_version() != SCHEMA_VERSION {
        return DesktopToExtensionMessage::rejected(
            "Unsupported message schema version.".to_string(),
        );
    }

    match message {
        ExtensionToDesktopMessage::Ping { .. } => DesktopToExtensionMessage::Pong {
            schema_version: SCHEMA_VERSION,
        },
        ExtensionToDesktopMessage::GetBrowserIntegrationPreferences { .. } => {
            DesktopToExtensionMessage::BrowserIntegrationPreferences {
                schema_version: SCHEMA_VERSION,
                preferences: state.browser_integration_preferences(),
            }
        }
        ExtensionToDesktopMessage::DetectedCandidates { candidates, .. } => {
            DesktopToExtensionMessage::rejected(format!(
                "received {} candidate(s); no job was requested",
                candidates.len()
            ))
        }
        ExtensionToDesktopMessage::CreateJob { job, .. } => {
            match start_job_from_extension(&state, *job).await {
                Ok(download_job) => {
                    show_main_window(&app_handle);
                    DesktopToExtensionMessage::JobAccepted {
                        schema_version: SCHEMA_VERSION,
                        job_id: download_job.id,
                    }
                }
                // The browser carries on with the download; LDM has nothing to show for it.
                Err(error) if error.code == ErrorCode::SourceRefused => {
                    DesktopToExtensionMessage::rejected_because(
                        RejectionCode::SourceRefused,
                        error.message,
                    )
                }
                Err(error) => {
                    show_main_window(&app_handle);
                    match error.code {
                        ErrorCode::FirstRunSetupRequired => {
                            DesktopToExtensionMessage::rejected_because(
                                RejectionCode::FirstRunSetupRequired,
                                error.message,
                            )
                        }
                        _ => DesktopToExtensionMessage::rejected(error.message),
                    }
                }
            }
        }
    }
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[cfg(test)]
mod tests {
    use tokio::net::windows::named_pipe::ClientOptions;

    use super::*;

    #[test]
    fn builds_a_user_only_sddl() {
        let sddl = current_user_pipe_sddl().expect("sddl");
        assert!(sddl.starts_with("D:P(A;;GA;;;S-1-"), "{sddl}");
        SecurityDescriptor::from_sddl(&sddl).expect("descriptor");
    }

    #[tokio::test]
    async fn user_only_pipe_accepts_the_same_user() {
        let sddl = current_user_pipe_sddl().expect("sddl");
        let name = format!(r"\\.\pipe\ldm-test-acl-{}", std::process::id());
        let mut server = create_server(&name, Some(&sddl), true).expect("server");
        let mut client = ClientOptions::new().open(&name).expect("client");
        server.connect().await.expect("connect");

        client.write_all(b"ping\n").await.expect("write");
        let mut received = [0_u8; 5];
        server.read_exact(&mut received).await.expect("read");
        assert_eq!(&received, b"ping\n");
    }

    #[tokio::test]
    async fn first_instance_refuses_a_name_another_instance_holds() {
        let name = format!(r"\\.\pipe\ldm-test-first-{}", std::process::id());
        let _held = create_server(&name, None, true).expect("first instance");
        assert!(create_server(&name, None, true).is_err());
        assert!(create_server(&name, None, false).is_ok());
    }
}
