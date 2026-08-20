//! Per-user Windows startup registration.

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows::core::{HSTRING, w};

const VALUE_NAME: &str = "MonitorDDC";

pub fn is_enabled() -> Result<bool, String> {
    let Some(command) = read_value()? else {
        return Ok(false);
    };
    Ok(command == startup_command()?)
}

pub fn set_enabled(enabled: bool) -> Result<(), String> {
    let key = if enabled {
        // RegCreateKeyExW also handles the unusual case where the per-user Run
        // key has not been created yet. No administrator privileges are needed.
        RegistryKey::create(KEY_SET_VALUE)?
    } else {
        let Some(key) = RegistryKey::open_optional(KEY_SET_VALUE)? else {
            return Ok(());
        };
        key
    };
    let value_name = HSTRING::from(VALUE_NAME);

    if enabled {
        let command = startup_command()?;
        let utf16: Vec<u16> = command.encode_utf16().chain([0]).collect();
        let bytes: Vec<u8> = utf16.iter().flat_map(|value| value.to_le_bytes()).collect();
        let result =
            unsafe { RegSetValueExW(key.0, &value_name, None, REG_SZ, Some(bytes.as_slice())) };
        check_result(result, "写入开机启动项")
    } else {
        let result = unsafe { RegDeleteValueW(key.0, &value_name) };
        if result == ERROR_SUCCESS || result == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(format!("删除开机启动项失败（Windows 错误 {}）", result.0))
        }
    }
}

fn read_value() -> Result<Option<String>, String> {
    let Some(key) = RegistryKey::open_optional(KEY_READ)? else {
        return Ok(None);
    };
    let value_name = HSTRING::from(VALUE_NAME);
    let mut value_type = Default::default();
    let mut byte_count = 0_u32;
    let result = unsafe {
        RegQueryValueExW(
            key.0,
            &value_name,
            None,
            Some(&mut value_type),
            None,
            Some(&mut byte_count),
        )
    };
    if result == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    check_result(result, "读取开机启动项")?;
    if value_type != REG_SZ || byte_count < 2 {
        return Ok(None);
    }

    let mut bytes = vec![0_u8; byte_count as usize];
    let result = unsafe {
        RegQueryValueExW(
            key.0,
            &value_name,
            None,
            Some(&mut value_type),
            Some(bytes.as_mut_ptr()),
            Some(&mut byte_count),
        )
    };
    check_result(result, "读取开机启动项")?;

    let utf16: Vec<u16> = bytes[..byte_count as usize]
        .chunks_exact(2)
        .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
        .take_while(|value| *value != 0)
        .collect();
    Ok(Some(String::from_utf16_lossy(&utf16)))
}

fn startup_command() -> Result<String, String> {
    let executable =
        std::env::current_exe().map_err(|error| format!("无法获取程序路径：{error}"))?;
    Ok(format!("\"{}\" --tray", executable.display()))
}

fn check_result(
    result: windows::Win32::Foundation::WIN32_ERROR,
    operation: &str,
) -> Result<(), String> {
    if result == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!("{operation}失败（Windows 错误 {}）", result.0))
    }
}

struct RegistryKey(HKEY);

impl RegistryKey {
    fn open_optional(
        access: windows::Win32::System::Registry::REG_SAM_FLAGS,
    ) -> Result<Option<Self>, String> {
        let mut key = HKEY::default();
        let result = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"),
                None,
                access,
                &mut key,
            )
        };
        if result == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        check_result(result, "打开开机启动配置")?;
        Ok(Some(Self(key)))
    }

    fn create(access: windows::Win32::System::Registry::REG_SAM_FLAGS) -> Result<Self, String> {
        let mut key = HKEY::default();
        let result = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"),
                None,
                w!(""),
                REG_OPTION_NON_VOLATILE,
                access,
                None,
                &mut key,
                None,
            )
        };
        check_result(result, "创建开机启动配置")?;
        Ok(Self(key))
    }
}

impl Drop for RegistryKey {
    fn drop(&mut self) {
        let _ = unsafe { RegCloseKey(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::startup_command;

    #[test]
    fn startup_command_quotes_the_executable_and_starts_hidden() {
        let command = startup_command().expect("current executable should be available");
        assert!(command.starts_with('"'));
        assert!(command.ends_with("\" --tray"));
    }
}
