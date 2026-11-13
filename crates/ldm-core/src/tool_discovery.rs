//! Finds the external tools (yt-dlp, FFmpeg, FFprobe, Deno).
//!
//! A tool is looked for in three places, in this order: the folders LDM ships tools in, the
//! tool's own `LDM_*_EXE` variable, and `PATH`. Every candidate is an absolute path that does
//! not depend on the directory LDM happened to be started from: LDM runs what it finds here,
//! so a `tools\yt-dlp.exe` lying in some working directory must never be picked up.
//!
//! `PATH` is the one this process was started with plus the one Windows has now
//! (`system_path`), so a tool installed while LDM runs is found without a restart.

use std::path::{Path, PathBuf};

use serde::Serialize;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolKind {
    YtDlp,
    Ffmpeg,
    Ffprobe,
    Deno,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolSource {
    Bundled,
    Env,
    Path,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStatus {
    pub kind: ToolKind,
    pub name: &'static str,
    pub env_var: &'static str,
    pub required: bool,
    pub found: bool,
    pub source: Option<ToolSource>,
    pub path: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStatusReport {
    pub tools: Vec<ToolStatus>,
}

pub fn discover_tools() -> ToolStatusReport {
    ToolStatusReport {
        tools: ToolKind::all().iter().copied().map(discover_tool).collect(),
    }
}

pub fn discover_tool(kind: ToolKind) -> ToolStatus {
    discover_tool_with(
        kind,
        bundled_tool_dirs(),
        std::env::var_os(kind.env_var()),
        std::env::join_paths(crate::system_path::current_path_entries()).ok(),
    )
}

fn discover_tool_with(
    kind: ToolKind,
    bundled_dirs: Vec<PathBuf>,
    env_value: Option<std::ffi::OsString>,
    path_var: Option<std::ffi::OsString>,
) -> ToolStatus {
    if let Some(path) = find_bundled_tool(kind, &bundled_dirs) {
        return kind.status_found(ToolSource::Bundled, path);
    }

    if let Some(path) = env_value
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
    {
        if !path.is_absolute() {
            return kind.status_missing(format!(
                "{} must be the full path of the executable.",
                kind.env_var()
            ));
        }
        if path.is_file() {
            return kind.status_found(ToolSource::Env, path);
        }

        return kind.status_missing(format!(
            "{} points to a file that does not exist.",
            kind.env_var()
        ));
    }

    if let Some(path) = find_path_tool(kind, path_var) {
        return kind.status_found(ToolSource::Path, path);
    }

    kind.status_missing(default_missing_message(kind))
}

fn find_bundled_tool(kind: ToolKind, bundled_dirs: &[PathBuf]) -> Option<PathBuf> {
    bundled_dirs
        .iter()
        .map(|dir| dir.join(kind.filename()))
        .find(|path| path.is_file())
}

/// The first `PATH` folder that holds the tool. Relative entries (`.`, `bin`) are skipped:
/// they mean a different folder for every working directory.
fn find_path_tool(kind: ToolKind, path_var: Option<std::ffi::OsString>) -> Option<PathBuf> {
    std::env::split_paths(&path_var?)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(kind.filename()))
        .find(|path| path.is_file())
}

/// The folders LDM ships tools in.
fn bundled_tool_dirs() -> Vec<PathBuf> {
    bundled_tool_dirs_for(
        std::env::var_os("LDM_RESOURCES_DIR").map(PathBuf::from),
        std::env::current_exe().ok().as_deref(),
    )
}

fn bundled_tool_dirs_for(
    resources_dir: Option<PathBuf>,
    current_exe: Option<&Path>,
) -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    // Set by whoever keeps the tools somewhere else.
    if let Some(root) = resources_dir.filter(|root| root.is_absolute()) {
        dirs.push(root.join("tools"));
        dirs.push(root);
    }

    // Where the installer puts them: `resources\tools` next to the app.
    if let Some(exe_dir) = current_exe.and_then(Path::parent) {
        dirs.push(exe_dir.join("resources").join("tools"));
    }

    // A development build uses the tools of its checkout. A release build must not: the path
    // would be compiled into the executable and looked up on other people's machines.
    #[cfg(debug_assertions)]
    if let Some(repo_root) = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2) {
        dirs.push(repo_root.join("apps/desktop/src-tauri/resources/tools"));
    }

    dirs
}

fn default_missing_message(kind: ToolKind) -> String {
    if kind.required() {
        format!(
            "{} is missing. Install it so that it is on PATH, or set {} to the full path of the executable.",
            kind.name(),
            kind.env_var()
        )
    } else {
        format!(
            "{} is optional. Install it so that it is on PATH, or set {} to the full path of the executable, when provider extraction needs it.",
            kind.name(),
            kind.env_var()
        )
    }
}

impl ToolKind {
    pub fn all() -> &'static [Self] {
        &[Self::YtDlp, Self::Ffmpeg, Self::Ffprobe, Self::Deno]
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::YtDlp => "yt-dlp",
            Self::Ffmpeg => "FFmpeg",
            Self::Ffprobe => "FFprobe",
            Self::Deno => "Deno",
        }
    }

    pub fn env_var(self) -> &'static str {
        match self {
            Self::YtDlp => "LDM_YTDLP_EXE",
            Self::Ffmpeg => "LDM_FFMPEG_EXE",
            Self::Ffprobe => "LDM_FFPROBE_EXE",
            Self::Deno => "LDM_DENO_EXE",
        }
    }

    /// The file name of the executable.
    pub fn filename(self) -> &'static str {
        match self {
            Self::YtDlp => "yt-dlp.exe",
            Self::Ffmpeg => "ffmpeg.exe",
            Self::Ffprobe => "ffprobe.exe",
            Self::Deno => "deno.exe",
        }
    }

    pub fn required(self) -> bool {
        !matches!(self, Self::Deno)
    }

    fn status_found(self, source: ToolSource, path: PathBuf) -> ToolStatus {
        ToolStatus {
            kind: self,
            name: self.name(),
            env_var: self.env_var(),
            required: self.required(),
            found: true,
            source: Some(source),
            path: Some(path.to_string_lossy().to_string()),
            message: match source {
                ToolSource::Bundled => format!("{} found in bundled tools.", self.name()),
                ToolSource::Env => format!("{} found via {}.", self.name(), self.env_var()),
                ToolSource::Path => format!("{} found on PATH.", self.name()),
            },
        }
    }

    fn status_missing(self, message: String) -> ToolStatus {
        ToolStatus {
            kind: self,
            name: self.name(),
            env_var: self.env_var(),
            required: self.required(),
            found: false,
            source: None,
            path: None,
            message,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use tempfile::tempdir;

    use super::*;

    /// A folder with an empty file named like the tool.
    fn dir_with(kind: ToolKind) -> (tempfile::TempDir, PathBuf) {
        let dir = tempdir().unwrap();
        let tool = dir.path().join(kind.filename());
        std::fs::write(&tool, b"").unwrap();
        (dir, tool)
    }

    fn os(path: &Path) -> Option<OsString> {
        Some(path.as_os_str().to_os_string())
    }

    #[test]
    fn bundled_tools_win_before_the_variable_and_path() {
        let (bundled, bundled_tool) = dir_with(ToolKind::YtDlp);
        let (_from_env, env_tool) = dir_with(ToolKind::YtDlp);
        let (on_path, _) = dir_with(ToolKind::YtDlp);

        let status = discover_tool_with(
            ToolKind::YtDlp,
            vec![bundled.path().to_path_buf()],
            os(&env_tool),
            os(on_path.path()),
        );

        assert!(status.found);
        assert_eq!(status.source, Some(ToolSource::Bundled));
        assert_eq!(status.path.as_deref(), Some(bundled_tool.to_str().unwrap()));
    }

    #[test]
    fn the_variable_wins_before_path() {
        let (_from_env, env_tool) = dir_with(ToolKind::Ffmpeg);
        let (on_path, _) = dir_with(ToolKind::Ffmpeg);

        let status = discover_tool_with(
            ToolKind::Ffmpeg,
            Vec::new(),
            os(&env_tool),
            os(on_path.path()),
        );

        assert_eq!(status.source, Some(ToolSource::Env));
        assert_eq!(status.path.as_deref(), Some(env_tool.to_str().unwrap()));
    }

    #[test]
    fn a_variable_that_names_no_file_is_reported_instead_of_falling_back() {
        let (on_path, _) = dir_with(ToolKind::Ffmpeg);
        let missing = on_path.path().join("elsewhere").join("ffmpeg.exe");

        let status = discover_tool_with(
            ToolKind::Ffmpeg,
            Vec::new(),
            os(&missing),
            os(on_path.path()),
        );

        assert!(!status.found);
        assert!(
            status.message.contains("does not exist"),
            "{}",
            status.message
        );
    }

    #[test]
    fn a_relative_path_in_the_variable_is_refused() {
        // Relative to whatever directory LDM was started in: not a place to run a program from.
        let status = discover_tool_with(
            ToolKind::YtDlp,
            Vec::new(),
            Some(OsString::from(r"tools\yt-dlp.exe")),
            None,
        );

        assert!(!status.found);
        assert!(status.message.contains("full path"), "{}", status.message);
    }

    #[test]
    fn path_is_the_last_place_and_empty_variables_do_not_count() {
        let (on_path, path_tool) = dir_with(ToolKind::Ffmpeg);

        let status = discover_tool_with(
            ToolKind::Ffmpeg,
            Vec::new(),
            Some(OsString::new()),
            os(on_path.path()),
        );

        assert!(status.found);
        assert_eq!(status.source, Some(ToolSource::Path));
        assert_eq!(status.path.as_deref(), Some(path_tool.to_str().unwrap()));
    }

    #[test]
    fn relative_path_entries_are_skipped() {
        let (on_path, path_tool) = dir_with(ToolKind::Deno);
        // `.` and `tools` would resolve against the working directory.
        let path_var = std::env::join_paths([
            PathBuf::from("."),
            PathBuf::from("tools"),
            on_path.path().to_path_buf(),
        ])
        .unwrap();

        let found = find_path_tool(ToolKind::Deno, Some(path_var));
        let only_relative = find_path_tool(ToolKind::Deno, Some(OsString::from(".")));

        assert_eq!(found, Some(path_tool));
        assert_eq!(only_relative, None);
    }

    #[test]
    fn a_missing_tool_says_how_to_provide_it() {
        let status = discover_tool_with(ToolKind::YtDlp, Vec::new(), None, None);

        assert!(!status.found);
        assert!(status.message.contains("PATH"), "{}", status.message);
        assert!(
            status.message.contains("LDM_YTDLP_EXE"),
            "{}",
            status.message
        );
    }

    #[test]
    fn tools_are_looked_for_next_to_the_app_and_never_in_the_working_directory() {
        let install_dir = tempdir().unwrap();
        let exe = install_dir.path().join("ldm-desktop.exe");

        let dirs = bundled_tool_dirs_for(None, Some(&exe));

        assert_eq!(dirs[0], install_dir.path().join("resources").join("tools"));
        assert!(dirs.iter().all(|dir| dir.is_absolute()), "{dirs:?}");
        let working_dir = std::env::current_dir().unwrap();
        assert!(
            !dirs.iter().any(|dir| *dir == working_dir.join("tools")
                || *dir == working_dir.join("resources").join("tools")),
            "{dirs:?}"
        );
    }

    #[test]
    fn the_resources_variable_comes_first_and_must_be_absolute() {
        let resources = tempdir().unwrap();
        let exe = resources.path().join("app").join("ldm-desktop.exe");

        let dirs = bundled_tool_dirs_for(Some(resources.path().to_path_buf()), Some(&exe));
        let relative = bundled_tool_dirs_for(Some(PathBuf::from("resources")), Some(&exe));

        assert_eq!(dirs[0], resources.path().join("tools"));
        assert_eq!(dirs[1], resources.path());
        assert!(relative.iter().all(|dir| dir.is_absolute()), "{relative:?}");
    }

    #[test]
    fn a_development_build_finds_the_tools_of_its_checkout() {
        let dirs = bundled_tool_dirs_for(None, None);

        // Only compiled into debug builds, which is what tests are.
        assert_eq!(dirs.len(), 1);
        assert!(dirs[0].ends_with("apps/desktop/src-tauri/resources/tools"));
        assert!(dirs[0]
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("tauri.conf.json")
            .is_file());
    }
}
