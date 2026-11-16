//! Current `PATH` including the user and machine values from the registry.

use std::{
    ffi::{OsStr, OsString},
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::PathBuf,
    ptr,
};

use windows_sys::Win32::{
    Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS},
    System::Registry::{
        RegGetValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_EXPAND_SZ,
        RRF_RT_REG_SZ,
    },
};

const USER_ENVIRONMENT_KEY: &str = "Environment";
const MACHINE_ENVIRONMENT_KEY: &str =
    r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";

/// Folders to search: this process's `PATH`, then the registry's user and machine `PATH`, without repeats.
pub fn current_path_entries() -> Vec<PathBuf> {
    let sources = [
        std::env::var_os("PATH"),
        registry_path(HKEY_CURRENT_USER, USER_ENVIRONMENT_KEY),
        registry_path(HKEY_LOCAL_MACHINE, MACHINE_ENVIRONMENT_KEY),
    ];
    merge_path_values(sources.iter().flatten().map(OsString::as_os_str))
}

/// Splits `PATH` values and drops repeated folders (case and trailing separator ignored).
fn merge_path_values<'a>(values: impl Iterator<Item = &'a OsStr>) -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    let mut entries = Vec::new();
    for value in values {
        for entry in std::env::split_paths(value) {
            if entry.as_os_str().is_empty() {
                continue;
            }
            let key = entry
                .to_string_lossy()
                .trim_end_matches(['\\', '/'])
                .to_lowercase();
            if seen.insert(key) {
                entries.push(entry);
            }
        }
    }
    entries
}

/// The `Path` value under `subkey`, with `%VARIABLES%` expanded; `None` when there is none.
fn registry_path(root: HKEY, subkey: &str) -> Option<OsString> {
    let subkey = wide(subkey);
    let value = wide("Path");
    // The value can grow between the size query and the read.
    for _ in 0..4 {
        let mut byte_len = 0_u32;
        // SAFETY: names are NUL-terminated; with no buffer the call only returns the size.
        let status = unsafe {
            RegGetValueW(
                root,
                subkey.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ,
                ptr::null_mut(),
                ptr::null_mut(),
                &mut byte_len,
            )
        };
        if status != ERROR_SUCCESS || byte_len == 0 {
            return None;
        }

        let mut buffer = vec![0_u16; (byte_len as usize).div_ceil(2)];
        let mut written = (buffer.len() * 2) as u32;
        // SAFETY: `buffer` is `written` bytes long.
        let status = unsafe {
            RegGetValueW(
                root,
                subkey.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ,
                ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut written,
            )
        };
        if status == ERROR_MORE_DATA {
            continue;
        }
        if status != ERROR_SUCCESS {
            return None;
        }
        buffer.truncate((written as usize) / 2);
        while buffer.last() == Some(&0) {
            buffer.pop();
        }
        return Some(OsString::from_wide(&buffer));
    }
    None
}

fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain([0]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn merged(values: &[&str]) -> Vec<String> {
        merge_path_values(values.iter().map(OsStr::new))
            .into_iter()
            .map(|entry| entry.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn folders_keep_their_order_and_appear_once() {
        assert_eq!(
            merged(&[
                r"C:\Tools;C:\Windows\System32",
                r"c:\tools\;D:\New;;C:\WINDOWS\system32",
            ]),
            [r"C:\Tools", r"C:\Windows\System32", r"D:\New"]
        );
    }

    #[test]
    fn the_machine_path_from_the_registry_is_part_of_the_search() {
        let machine = registry_path(HKEY_LOCAL_MACHINE, MACHINE_ENVIRONMENT_KEY)
            .expect("Windows always has a machine PATH");
        let machine = machine.to_string_lossy().to_lowercase();
        assert!(machine.contains(r"\system32"), "{machine}");
        assert!(!machine.contains('%'), "{machine}");

        let entries = current_path_entries();
        assert!(
            entries.iter().any(|entry| entry
                .to_string_lossy()
                .to_lowercase()
                .ends_with(r"\system32")),
            "{entries:?}"
        );
    }

    #[test]
    fn a_key_without_a_path_value_is_no_path() {
        assert_eq!(
            registry_path(HKEY_CURRENT_USER, r"Software\LDM\NoSuchKeyForTests"),
            None
        );
    }
}
