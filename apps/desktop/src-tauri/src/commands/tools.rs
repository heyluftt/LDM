use ldm_core::tool_discovery::{discover_tools, ToolStatusReport};

#[tauri::command]
pub fn tool_status() -> ToolStatusReport {
    discover_tools()
}
