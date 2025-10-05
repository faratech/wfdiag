use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[cfg(windows)]
use windows::Win32::System::Threading::{GetCurrentProcess, IsWow64Process2};
#[cfg(windows)]
use windows::Win32::System::SystemInformation::{GetSystemInfo, IMAGE_FILE_MACHINE, SYSTEM_INFO};

/// Processor architecture constants from Windows API
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u16)]
pub enum ProcessorArchitecture {
    Intel = 0,      // x86
    Arm = 5,        // ARM (32-bit)
    Amd64 = 9,      // x64
    Arm64 = 12,     // ARM64
    Unknown = 0xFFFF,
}

impl ProcessorArchitecture {
    pub fn from_u16(value: u16) -> Self {
        match value {
            0 => Self::Intel,
            5 => Self::Arm,
            9 => Self::Amd64,
            12 => Self::Arm64,
            _ => Self::Unknown,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Intel => "x86",
            Self::Arm => "ARM",
            Self::Amd64 => "x64",
            Self::Arm64 => "ARM64",
            Self::Unknown => "Unknown",
        }
    }

    pub fn to_u16(&self) -> u16 {
        *self as u16
    }
}

/// Architecture information for the system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchitectureInfo {
    /// Process architecture (what this app is compiled as)
    pub process_arch: ProcessorArchitecture,
    /// Native system architecture (actual hardware)
    pub native_arch: ProcessorArchitecture,
    /// Whether the process is running under emulation
    pub is_emulated: bool,
    /// Process architecture name
    pub process_arch_name: String,
    /// Native architecture name
    pub native_arch_name: String,
    /// Page size from SYSTEM_INFO
    pub page_size: u32,
    /// Number of processors
    pub processor_count: u32,
}

/// Get detailed architecture information using Windows APIs
#[cfg(windows)]
pub fn get_architecture_info() -> Result<ArchitectureInfo> {
    let mut process_machine: IMAGE_FILE_MACHINE = IMAGE_FILE_MACHINE(0);
    let mut native_machine: IMAGE_FILE_MACHINE = IMAGE_FILE_MACHINE(0);

    unsafe {
        let process_handle = GetCurrentProcess();

        // Try to use IsWow64Process2 for accurate detection
        let wow64_result = IsWow64Process2(
            process_handle,
            &mut process_machine as *mut _,
            Some(&mut native_machine as *mut _),
        );

        if wow64_result.is_err() {
            // Fallback to GetSystemInfo if IsWow64Process2 is not available (older Windows)
            let mut system_info = SYSTEM_INFO::default();
            GetSystemInfo(&mut system_info);

            let arch = system_info.Anonymous.Anonymous.wProcessorArchitecture.0;
            let arch_enum = ProcessorArchitecture::from_u16(arch);

            return Ok(ArchitectureInfo {
                process_arch: arch_enum,
                native_arch: arch_enum,
                is_emulated: false,
                process_arch_name: arch_enum.name().to_string(),
                native_arch_name: arch_enum.name().to_string(),
                page_size: system_info.dwPageSize,
                processor_count: system_info.dwNumberOfProcessors,
            });
        }

        // Convert IMAGE_FILE_MACHINE to ProcessorArchitecture
        let process_arch = machine_to_architecture(process_machine);
        let native_arch = machine_to_architecture(native_machine);

        // Get system info for page size and processor count
        let mut system_info = SYSTEM_INFO::default();
        GetSystemInfo(&mut system_info);

        Ok(ArchitectureInfo {
            process_arch,
            native_arch,
            is_emulated: process_arch != native_arch,
            process_arch_name: process_arch.name().to_string(),
            native_arch_name: native_arch.name().to_string(),
            page_size: system_info.dwPageSize,
            processor_count: system_info.dwNumberOfProcessors,
        })
    }
}

/// Fallback for non-Windows platforms
#[cfg(not(windows))]
pub fn get_architecture_info() -> Result<ArchitectureInfo> {
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

/// Convert IMAGE_FILE_MACHINE to ProcessorArchitecture
#[cfg(windows)]
fn machine_to_architecture(machine: windows::Win32::System::SystemInformation::IMAGE_FILE_MACHINE) -> ProcessorArchitecture {
    match machine.0 {
        0x014c => ProcessorArchitecture::Intel,     // IMAGE_FILE_MACHINE_I386
        0x01c4 => ProcessorArchitecture::Arm,       // IMAGE_FILE_MACHINE_ARMNT
        0x8664 => ProcessorArchitecture::Amd64,     // IMAGE_FILE_MACHINE_AMD64
        0xAA64 => ProcessorArchitecture::Arm64,     // IMAGE_FILE_MACHINE_ARM64
        _ => ProcessorArchitecture::Unknown,
    }
}

/// Get architecture info as JSON
pub fn get_architecture_json() -> Result<Value> {
    let arch_info = get_architecture_info()?;

    Ok(json!({
        "process_architecture": arch_info.process_arch.to_u16(),
        "process_architecture_name": arch_info.process_arch_name,
        "native_architecture": arch_info.native_arch.to_u16(),
        "native_architecture_name": arch_info.native_arch_name,
        "is_emulated": arch_info.is_emulated,
        "page_size": arch_info.page_size,
        "processor_count": arch_info.processor_count,
        "emulation_status": if arch_info.is_emulated {
            format!("{} app running on {} hardware",
                arch_info.process_arch_name,
                arch_info.native_arch_name)
        } else {
            format!("Native {} execution", arch_info.native_arch_name)
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_architecture_names() {
        assert_eq!(ProcessorArchitecture::Intel.name(), "x86");
        assert_eq!(ProcessorArchitecture::Amd64.name(), "x64");
        assert_eq!(ProcessorArchitecture::Arm64.name(), "ARM64");
    }

    #[test]
    fn test_architecture_from_u16() {
        assert_eq!(ProcessorArchitecture::from_u16(0), ProcessorArchitecture::Intel);
        assert_eq!(ProcessorArchitecture::from_u16(9), ProcessorArchitecture::Amd64);
        assert_eq!(ProcessorArchitecture::from_u16(12), ProcessorArchitecture::Arm64);
    }

    #[test]
    fn test_get_architecture_info() {
        let result = get_architecture_info();
        assert!(result.is_ok());
    }
}
