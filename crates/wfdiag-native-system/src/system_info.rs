use crate::SystemError;
use serde::{Deserialize, Serialize};

/// Exact object returned by the shipping `get_system_info` command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemInfo {
    pub computer_name: String,
    pub os_version: String,
    pub is_admin: bool,
}

#[cfg(any(windows, test))]
fn format_windows_version(
    product_name: &str,
    display_version: Option<&str>,
    edition_id: Option<&str>,
    current_build: Option<&str>,
) -> String {
    let is_windows_11 = if let Some(build) = current_build {
        match build.parse::<u32>() {
            Ok(number) => number >= 22_000,
            Err(error) => {
                eprintln!("Warning: Failed to parse Windows build number '{build}': {error}");
                product_name.contains("Windows 11")
            }
        }
    } else {
        product_name.contains("Windows 11")
    };

    let mut version_parts = Vec::new();
    if is_windows_11 {
        version_parts.push("Windows 11".to_string());
    } else if product_name.contains("Windows 10") {
        version_parts.push("Windows 10".to_string());
    } else {
        version_parts.push(product_name.to_string());
    }
    if let Some(edition) = edition_id {
        version_parts.push(edition.to_string());
    }
    if let Some(display_version) = display_version {
        version_parts.push(format!("({display_version})"));
    }
    version_parts.join(" ")
}

#[cfg(windows)]
fn get_windows_version_info() -> String {
    use winreg::RegKey;
    use winreg::enums::HKEY_LOCAL_MACHINE;

    let local_machine = RegKey::predef(HKEY_LOCAL_MACHINE);
    if let Ok(current_version) =
        local_machine.open_subkey("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion")
    {
        let product_name = current_version
            .get_value::<String, _>("ProductName")
            .unwrap_or_else(|_| "Windows".to_string());
        let display_version = current_version
            .get_value::<String, _>("DisplayVersion")
            .ok();
        let edition_id = current_version.get_value::<String, _>("EditionID").ok();
        let current_build = current_version.get_value::<String, _>("CurrentBuild").ok();
        return format_windows_version(
            &product_name,
            display_version.as_deref(),
            edition_id.as_deref(),
            current_build.as_deref(),
        );
    }
    "Windows".to_string()
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn is_process_elevated() -> bool {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{
        GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token = HANDLE(std::ptr::null_mut());
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token).is_err() {
            return false;
        }

        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut returned_length = 0;
        let Ok(elevation_size) = u32::try_from(std::mem::size_of::<TOKEN_ELEVATION>()) else {
            let _ = CloseHandle(token);
            return false;
        };
        let elevated = GetTokenInformation(
            token,
            TokenElevation,
            Some(std::ptr::from_mut(&mut elevation).cast()),
            elevation_size,
            &raw mut returned_length,
        )
        .is_ok()
            && elevation.TokenIsElevated != 0;
        let _ = CloseHandle(token);
        elevated
    }
}

/// Collect the exact read-only system information surface used by the UI.
///
/// # Errors
///
/// The current collectors use shipping fallbacks and therefore do not fail.
pub fn get_system_info() -> Result<SystemInfo, SystemError> {
    let computer_name = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "Unknown".to_string());

    #[cfg(windows)]
    let os_version = get_windows_version_info();
    #[cfg(not(windows))]
    let os_version = std::env::var("OS").unwrap_or_else(|_| "Unknown".to_string());

    #[cfg(windows)]
    let is_admin = is_process_elevated();
    #[cfg(not(windows))]
    let is_admin = false;

    Ok(SystemInfo {
        computer_name,
        os_version,
        is_admin,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_info_json_matches_the_shipping_contract_exactly() {
        let info = SystemInfo {
            computer_name: "TEST-PC".to_string(),
            os_version: "Windows 11 Pro (25H2)".to_string(),
            is_admin: true,
        };
        assert_eq!(
            serde_json::to_string(&info).unwrap(),
            r#"{"computer_name":"TEST-PC","os_version":"Windows 11 Pro (25H2)","is_admin":true}"#
        );
        let value = serde_json::to_value(info).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 3);
        assert!(value.get("package_identity").is_none());
    }

    #[test]
    fn windows_version_projection_preserves_registry_semantics() {
        assert_eq!(
            format_windows_version(
                "Windows 10 Pro",
                Some("25H2"),
                Some("Professional"),
                Some("26200")
            ),
            "Windows 11 Professional (25H2)"
        );
        assert_eq!(
            format_windows_version(
                "Windows 10 Pro",
                Some("22H2"),
                Some("Professional"),
                Some("19045")
            ),
            "Windows 10 Professional (22H2)"
        );
        assert_eq!(
            format_windows_version("Windows 11 Enterprise", None, None, Some("bad")),
            "Windows 11"
        );
        assert_eq!(
            format_windows_version("Windows Server 2025", None, None, None),
            "Windows Server 2025"
        );
    }
}
