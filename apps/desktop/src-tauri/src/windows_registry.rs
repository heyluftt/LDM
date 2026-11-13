//! `HKEY_CURRENT_USER` access over the Win32 advapi32 API, without a shell or an extra crate.
//!
//! Used for the autostart `Run` value (`commands/startup.rs`) and the Native Messaging host
//! registration (`native_host.rs`). Only `REG_SZ` values are needed.

use std::{ffi::OsStr, io, os::windows::ffi::OsStrExt, ptr};

type HKey = isize;

const HKEY_CURRENT_USER: HKey = 0x8000_0001_u32 as HKey;
const KEY_QUERY_VALUE: u32 = 0x0001;
const KEY_SET_VALUE: u32 = 0x0002;
const REG_OPTION_NON_VOLATILE: u32 = 0x0000_0000;
const REG_SZ: u32 = 1;
const ERROR_SUCCESS: i32 = 0;
const ERROR_FILE_NOT_FOUND: i32 = 2;

extern "system" {
    fn RegOpenKeyExW(
        h_key: HKey,
        sub_key: *const u16,
        options: u32,
        desired_access: u32,
        result: *mut HKey,
    ) -> i32;
    fn RegCreateKeyExW(
        h_key: HKey,
        sub_key: *const u16,
        reserved: u32,
        class: *mut u16,
        options: u32,
        desired_access: u32,
        security_attributes: *mut std::ffi::c_void,
        result: *mut HKey,
        disposition: *mut u32,
    ) -> i32;
    fn RegQueryValueExW(
        h_key: HKey,
        value_name: *const u16,
        reserved: *mut u32,
        value_type: *mut u32,
        data: *mut u8,
        data_len: *mut u32,
    ) -> i32;
    fn RegSetValueExW(
        h_key: HKey,
        value_name: *const u16,
        reserved: u32,
        value_type: u32,
        data: *const u8,
        data_len: u32,
    ) -> i32;
    fn RegDeleteValueW(h_key: HKey, value_name: *const u16) -> i32;
    fn RegCloseKey(h_key: HKey) -> i32;
}

/// Reads the `REG_SZ` value `value_name` of `HKCU\subkey`. An empty `value_name` is the key's
/// default value. A missing key or value, or a value of another type, is `Ok(None)`.
pub fn read_string_value(subkey: &str, value_name: &str) -> io::Result<Option<String>> {
    let key = match open_key(subkey, KEY_QUERY_VALUE) {
        Ok(key) => key,
        Err(error) if is_not_found(&error) => return Ok(None),
        Err(error) => return Err(error),
    };
    let value_name = wide(value_name);
    let mut value_type = 0_u32;
    let mut data_len = 0_u32;
    let status = unsafe {
        RegQueryValueExW(
            key.0,
            value_name.as_ptr(),
            ptr::null_mut(),
            &mut value_type,
            ptr::null_mut(),
            &mut data_len,
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if status != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(status));
    }
    if value_type != REG_SZ || data_len == 0 {
        return Ok(None);
    }

    let mut data = vec![0_u8; data_len as usize];
    let status = unsafe {
        RegQueryValueExW(
            key.0,
            value_name.as_ptr(),
            ptr::null_mut(),
            &mut value_type,
            data.as_mut_ptr(),
            &mut data_len,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(status));
    }

    let (pairs, _) = data.as_chunks::<2>();
    let wide = pairs
        .iter()
        .map(|chunk| u16::from_le_bytes(*chunk))
        .take_while(|value| *value != 0)
        .collect::<Vec<_>>();
    Ok(Some(String::from_utf16_lossy(&wide)))
}

/// Creates `HKCU\subkey` (and its parents) when needed and sets the `REG_SZ` value
/// `value_name` to `data`. An empty `value_name` sets the key's default value.
pub fn write_string_value(subkey: &str, value_name: &str, data: &str) -> io::Result<()> {
    let key = create_key(subkey)?;
    let value_name = wide(value_name);
    let data = wide(data)
        .into_iter()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    let status = unsafe {
        RegSetValueExW(
            key.0,
            value_name.as_ptr(),
            0,
            REG_SZ,
            data.as_ptr(),
            data.len() as u32,
        )
    };
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(status))
    }
}

/// Removes `value_name` from `HKCU\subkey`; a missing key or value is not an error.
pub fn delete_value(subkey: &str, value_name: &str) -> io::Result<()> {
    let key = match open_key(subkey, KEY_SET_VALUE) {
        Ok(key) => key,
        Err(error) if is_not_found(&error) => return Ok(()),
        Err(error) => return Err(error),
    };
    let value_name = wide(value_name);
    let status = unsafe { RegDeleteValueW(key.0, value_name.as_ptr()) };
    if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(status))
    }
}

fn is_not_found(error: &io::Error) -> bool {
    error.raw_os_error() == Some(ERROR_FILE_NOT_FOUND)
}

fn open_key(subkey: &str, access: u32) -> io::Result<RegistryKey> {
    let mut key = 0;
    let subkey = wide(subkey);
    let status = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, access, &mut key) };
    if status == ERROR_SUCCESS {
        Ok(RegistryKey(key))
    } else {
        Err(io::Error::from_raw_os_error(status))
    }
}

fn create_key(subkey: &str) -> io::Result<RegistryKey> {
    let mut key = 0;
    let subkey = wide(subkey);
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            0,
            ptr::null_mut(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            ptr::null_mut(),
            &mut key,
            ptr::null_mut(),
        )
    };
    if status == ERROR_SUCCESS {
        Ok(RegistryKey(key))
    } else {
        Err(io::Error::from_raw_os_error(status))
    }
}

fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain([0]).collect()
}

struct RegistryKey(HKey);

impl Drop for RegistryKey {
    fn drop(&mut self) {
        unsafe {
            RegCloseKey(self.0);
        }
    }
}
