use crate::SystemError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[cfg(windows)]
use windows::Win32::System::SystemInformation::{GetSystemInfo, IMAGE_FILE_MACHINE, SYSTEM_INFO};
#[cfg(windows)]
use windows::Win32::System::Threading::{GetCurrentProcess, IsWow64Process2};

/// Processor architecture constants exposed by the shipping contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u16)]
pub enum ProcessorArchitecture {
    Intel = 0,
    Arm = 5,
    Amd64 = 9,
    Arm64 = 12,
    Unknown = 0xFFFF,
}

impl ProcessorArchitecture {
    #[must_use]
    pub const fn from_u16(value: u16) -> Self {
        match value {
            0 => Self::Intel,
            5 => Self::Arm,
            9 => Self::Amd64,
            12 => Self::Arm64,
            _ => Self::Unknown,
        }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Intel => "x86",
            Self::Arm => "ARM",
            Self::Amd64 => "x64",
            Self::Arm64 => "ARM64",
            Self::Unknown => "Unknown",
        }
    }

    #[must_use]
    pub const fn to_u16(self) -> u16 {
        self as u16
    }
}

/// Canonical architecture data before its shipping command projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchitectureInfo {
    pub process_arch: ProcessorArchitecture,
    pub native_arch: ProcessorArchitecture,
    pub is_emulated: bool,
    pub process_arch_name: String,
    pub native_arch_name: String,
    pub page_size: u32,
    pub processor_count: u32,
}

/// Exact serialized contract returned by `get_architecture_info`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchitectureSnapshot {
    pub process_architecture: u16,
    pub process_architecture_name: String,
    pub native_architecture: u16,
    pub native_architecture_name: String,
    pub is_emulated: bool,
    pub page_size: u32,
    pub processor_count: u32,
    pub emulation_status: String,
}

impl From<ArchitectureInfo> for ArchitectureSnapshot {
    fn from(info: ArchitectureInfo) -> Self {
        let emulation_status = if info.is_emulated {
            format!(
                "{} app running on {} hardware",
                info.process_arch_name, info.native_arch_name
            )
        } else {
            format!("Native {} execution", info.native_arch_name)
        };

        Self {
            process_architecture: info.process_arch.to_u16(),
            process_architecture_name: info.process_arch_name,
            native_architecture: info.native_arch.to_u16(),
            native_architecture_name: info.native_arch_name,
            is_emulated: info.is_emulated,
            page_size: info.page_size,
            processor_count: info.processor_count,
            emulation_status,
        }
    }
}

#[cfg(any(windows, test))]
const fn machine_to_architecture(machine: u16) -> ProcessorArchitecture {
    match machine {
        0x014c => ProcessorArchitecture::Intel,
        0x01c4 => ProcessorArchitecture::Arm,
        0x8664 => ProcessorArchitecture::Amd64,
        0xAA64 => ProcessorArchitecture::Arm64,
        _ => ProcessorArchitecture::Unknown,
    }
}

/// `IMAGE_FILE_MACHINE_UNKNOWN` and `IMAGE_FILE_MACHINE_TARGET_HOST` both
/// mean that the current process is native, rather than a WOW64 guest.
#[cfg(any(windows, test))]
const fn is_wow64_machine(process_machine: u16) -> bool {
    !matches!(process_machine, 0x0000 | 0x0001)
}

#[cfg(any(windows, test))]
fn resolve_machine_pair(
    process_machine: u16,
    native_machine: u16,
) -> (ProcessorArchitecture, ProcessorArchitecture, bool) {
    let native_arch = machine_to_architecture(native_machine);
    let is_emulated = is_wow64_machine(process_machine);
    let process_arch = if is_emulated {
        machine_to_architecture(process_machine)
    } else {
        native_arch
    };
    (process_arch, native_arch, is_emulated)
}

/// Collect detailed architecture information using the same Windows API and
/// fallback semantics as the shipping Tauri command.
///
/// # Errors
///
/// This interface remains fallible for compatibility and future Windows API
/// failures. The current implementation falls back to `GetSystemInfo` when
/// `IsWow64Process2` is unavailable.
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn get_architecture_info() -> Result<ArchitectureInfo, SystemError> {
    let mut process_machine = IMAGE_FILE_MACHINE(0);
    let mut native_machine = IMAGE_FILE_MACHINE(0);

    unsafe {
        let wow64_result = IsWow64Process2(
            GetCurrentProcess(),
            &raw mut process_machine,
            Some(&raw mut native_machine),
        );

        if wow64_result.is_err() {
            let mut system_info = SYSTEM_INFO::default();
            GetSystemInfo(&raw mut system_info);
            let architecture = ProcessorArchitecture::from_u16(
                system_info.Anonymous.Anonymous.wProcessorArchitecture.0,
            );
            return Ok(ArchitectureInfo {
                process_arch: architecture,
                native_arch: architecture,
                is_emulated: false,
                process_arch_name: architecture.name().to_string(),
                native_arch_name: architecture.name().to_string(),
                page_size: system_info.dwPageSize,
                processor_count: system_info.dwNumberOfProcessors,
            });
        }

        let (process_arch, native_arch, is_emulated) =
            resolve_machine_pair(process_machine.0, native_machine.0);
        let mut system_info = SYSTEM_INFO::default();
        GetSystemInfo(&raw mut system_info);

        Ok(ArchitectureInfo {
            process_arch,
            native_arch,
            is_emulated,
            process_arch_name: process_arch.name().to_string(),
            native_arch_name: native_arch.name().to_string(),
            page_size: system_info.dwPageSize,
            processor_count: system_info.dwNumberOfProcessors,
        })
    }
}

/// Portable fallback retained for development tooling on non-Windows hosts.
///
/// # Errors
///
/// The fallback currently cannot fail.
#[cfg(not(windows))]
pub fn get_architecture_info() -> Result<ArchitectureInfo, SystemError> {
    Ok(ArchitectureInfo {
        process_arch: ProcessorArchitecture::Unknown,
        native_arch: ProcessorArchitecture::Unknown,
        is_emulated: false,
        process_arch_name: "Unknown".to_string(),
        native_arch_name: "Unknown".to_string(),
        page_size: 4096,
        processor_count: 1,
    })
}

/// Collect the typed shipping command projection.
///
/// # Errors
///
/// Propagates architecture collection errors.
pub fn get_architecture_snapshot() -> Result<ArchitectureSnapshot, SystemError> {
    get_architecture_info().map(ArchitectureSnapshot::from)
}

/// Collect architecture data as the exact JSON value returned by Tauri.
///
/// # Errors
///
/// Returns [`SystemError::Serialization`] if projection serialization fails.
pub fn get_architecture_json() -> Result<Value, SystemError> {
    serde_json::to_value(get_architecture_snapshot()?)
        .map_err(|error| SystemError::Serialization(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(
        process: ProcessorArchitecture,
        native: ProcessorArchitecture,
        emulated: bool,
    ) -> ArchitectureInfo {
        ArchitectureInfo {
            process_arch: process,
            native_arch: native,
            is_emulated: emulated,
            process_arch_name: process.name().to_string(),
            native_arch_name: native.name().to_string(),
            page_size: 4096,
            processor_count: 16,
        }
    }

    #[test]
    fn architecture_names_and_numeric_contract_are_stable() {
        assert_eq!(ProcessorArchitecture::Intel.name(), "x86");
        assert_eq!(ProcessorArchitecture::Arm.name(), "ARM");
        assert_eq!(ProcessorArchitecture::Amd64.name(), "x64");
        assert_eq!(ProcessorArchitecture::Arm64.name(), "ARM64");
        assert_eq!(ProcessorArchitecture::Unknown.name(), "Unknown");
        assert_eq!(
            ProcessorArchitecture::from_u16(0),
            ProcessorArchitecture::Intel
        );
        assert_eq!(
            ProcessorArchitecture::from_u16(9),
            ProcessorArchitecture::Amd64
        );
        assert_eq!(
            ProcessorArchitecture::from_u16(12),
            ProcessorArchitecture::Arm64
        );
        assert_eq!(
            ProcessorArchitecture::from_u16(7),
            ProcessorArchitecture::Unknown
        );
        assert_eq!(ProcessorArchitecture::Unknown.to_u16(), u16::MAX);
    }

    #[test]
    fn native_and_emulated_machine_pairs_match_is_wow64_process2_semantics() {
        assert_eq!(
            resolve_machine_pair(0x0000, 0x8664),
            (
                ProcessorArchitecture::Amd64,
                ProcessorArchitecture::Amd64,
                false
            )
        );
        assert_eq!(
            resolve_machine_pair(0x0001, 0xAA64),
            (
                ProcessorArchitecture::Arm64,
                ProcessorArchitecture::Arm64,
                false
            )
        );
        assert_eq!(
            resolve_machine_pair(0x8664, 0xAA64),
            (
                ProcessorArchitecture::Amd64,
                ProcessorArchitecture::Arm64,
                true
            )
        );
        assert_eq!(
            resolve_machine_pair(0x014c, 0x8664),
            (
                ProcessorArchitecture::Intel,
                ProcessorArchitecture::Amd64,
                true
            )
        );
    }

    #[test]
    fn architecture_snapshot_json_matches_the_shipping_contract() {
        let snapshot = ArchitectureSnapshot::from(info(
            ProcessorArchitecture::Amd64,
            ProcessorArchitecture::Arm64,
            true,
        ));
        assert_eq!(
            serde_json::to_string(&snapshot).unwrap(),
            r#"{"process_architecture":9,"process_architecture_name":"x64","native_architecture":12,"native_architecture_name":"ARM64","is_emulated":true,"page_size":4096,"processor_count":16,"emulation_status":"x64 app running on ARM64 hardware"}"#
        );

        let native = ArchitectureSnapshot::from(info(
            ProcessorArchitecture::Arm64,
            ProcessorArchitecture::Arm64,
            false,
        ));
        assert_eq!(native.emulation_status, "Native ARM64 execution");
    }

    #[test]
    fn portable_collection_returns_a_self_consistent_projection() {
        let snapshot = get_architecture_snapshot().unwrap();
        assert!(!snapshot.process_architecture_name.is_empty());
        assert!(!snapshot.native_architecture_name.is_empty());
        assert!(snapshot.page_size > 0);
        assert!(snapshot.processor_count > 0);
    }
}
