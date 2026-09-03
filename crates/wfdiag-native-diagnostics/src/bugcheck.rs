//! Blue-screen decoding from a kernel minidump header, without a debugger.
//!
//! Portable: everything here is bounds-checked byte parsing plus a curated
//! table, so it is unit-tested on Linux against synthetic headers. The
//! Windows collector only reads the bytes. Every read is checked; malformed
//! input yields an error or `None`, never a panic.

use serde::Serialize;

/// Kernel dump header layout, from the SDK's `wdbgexts.h` (`DUMP_HEADER64` /
/// `DUMP_HEADER32`) and the public `_DMP_HEADER64` reconstructions used by
/// crash-dump tooling. Re-verified against a real dump in the live-system
/// suite; the tests below pin these offsets.
pub mod layout {
    /// `"PAGE"` as a little-endian u32.
    pub const SIGNATURE: u32 = 0x4547_4150;
    /// `"DU64"`: 64-bit header.
    pub const VALID_DUMP_64: u32 = 0x3436_5544;
    /// `"DUMP"`: 32-bit header.
    pub const VALID_DUMP_32: u32 = 0x504D_5544;
    /// Both headers are one page.
    pub const HEADER_LEN: usize = 0x2000;

    pub const MACHINE_IMAGE_TYPE_64: usize = 0x30;
    pub const BUGCHECK_CODE_64: usize = 0x38;
    pub const BUGCHECK_PARAMETERS_64: usize = 0x40;
    pub const DUMP_TYPE_64: usize = 0xF98;
    pub const SYSTEM_TIME_64: usize = 0xFA8;

    pub const MACHINE_IMAGE_TYPE_32: usize = 0x20;
    pub const BUGCHECK_CODE_32: usize = 0x28;
    pub const BUGCHECK_PARAMETERS_32: usize = 0x2C;
    pub const DUMP_TYPE_32: usize = 0xF88;
    pub const SYSTEM_TIME_32: usize = 0xFC0;

    /// `DUMP_TYPE_TRIAGE`: the kernel minidump written to `C:\Windows\Minidump`.
    pub const DUMP_TYPE_TRIAGE: u32 = 4;

    /// `TRIAGE_DUMP64` follows the header. Offsets are absolute file offsets.
    pub const TRIAGE_DRIVER_LIST_OFFSET_64: usize = 0x2030;
    pub const TRIAGE_DRIVER_COUNT_64: usize = 0x2034;
    pub const TRIAGE_BROKEN_DRIVER_OFFSET_64: usize = 0x2040;
    /// `DUMP_DRIVER_ENTRY64`: `u32 DriverNameOffset; u32 pad; KLDR_DATA_TABLE_ENTRY64`.
    pub const DRIVER_ENTRY_LEN_64: usize = 0x98;
    pub const DRIVER_ENTRY_NAME_OFFSET: usize = 0x0;
    pub const DRIVER_ENTRY_DLL_BASE_64: usize = 0x38;
    pub const DRIVER_ENTRY_SIZE_OF_IMAGE_64: usize = 0x48;
    /// Sanity cap so a corrupt count cannot spin the reader.
    pub const MAX_DRIVERS: usize = 4096;
    /// `DUMP_STRING`: `u32 Length` (in UTF-16 units) then the characters.
    pub const MAX_DRIVER_NAME_CHARS: u32 = 260;
}

/// Why a header could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DumpParseError {
    TooShort { needed: usize, actual: usize },
    BadSignature,
    UnknownValidDump(u32),
}

impl std::fmt::Display for DumpParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooShort { needed, actual } => {
                write!(f, "dump header too short ({actual} of {needed} bytes)")
            }
            Self::BadSignature => write!(f, "not a kernel dump (missing PAGE signature)"),
            Self::UnknownValidDump(value) => {
                write!(f, "unknown dump header variant 0x{value:08X}")
            }
        }
    }
}

/// The fixed fields every kernel dump carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DumpHeader {
    pub is_64: bool,
    pub bugcheck_code: u32,
    pub parameters: [u64; 4],
    pub dump_type: u32,
    /// `SystemTime` as a Windows FILETIME (100 ns since 1601), when present.
    pub system_time_filetime: Option<i64>,
    pub machine_image_type: u32,
}

impl DumpHeader {
    /// Crash time as Unix seconds, when the header carried one.
    #[must_use]
    pub fn crash_time_unix_secs(&self) -> Option<i64> {
        const EPOCH_DIFFERENCE_SECS: i64 = 11_644_473_600;
        let filetime = self.system_time_filetime?;
        (filetime > 0).then(|| filetime / 10_000_000 - EPOCH_DIFFERENCE_SECS)
    }
}

fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    bytes
        .get(offset..offset.checked_add(4)?)
        .map(|slice| u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn u64_at(bytes: &[u8], offset: usize) -> Option<u64> {
    bytes.get(offset..offset.checked_add(8)?).map(|slice| {
        let mut raw = [0_u8; 8];
        raw.copy_from_slice(slice);
        u64::from_le_bytes(raw)
    })
}

/// Parse the first page of a kernel dump.
///
/// # Errors
///
/// Too few bytes for the fixed fields, a missing `PAGE` signature, or a
/// `ValidDump` tag that is neither `DU64` nor `DUMP`.
pub fn parse_dump_header(bytes: &[u8]) -> Result<DumpHeader, DumpParseError> {
    if bytes.len() < 8 {
        return Err(DumpParseError::TooShort {
            needed: 8,
            actual: bytes.len(),
        });
    }
    let signature = u32_at(bytes, 0).ok_or(DumpParseError::TooShort {
        needed: 8,
        actual: bytes.len(),
    })?;
    if signature != layout::SIGNATURE {
        return Err(DumpParseError::BadSignature);
    }
    let valid = u32_at(bytes, 4).ok_or(DumpParseError::TooShort {
        needed: 8,
        actual: bytes.len(),
    })?;
    let is_64 = match valid {
        layout::VALID_DUMP_64 => true,
        layout::VALID_DUMP_32 => false,
        other => return Err(DumpParseError::UnknownValidDump(other)),
    };
    let (code_at, params_at, machine_at, dump_type_at, time_at) = if is_64 {
        (
            layout::BUGCHECK_CODE_64,
            layout::BUGCHECK_PARAMETERS_64,
            layout::MACHINE_IMAGE_TYPE_64,
            layout::DUMP_TYPE_64,
            layout::SYSTEM_TIME_64,
        )
    } else {
        (
            layout::BUGCHECK_CODE_32,
            layout::BUGCHECK_PARAMETERS_32,
            layout::MACHINE_IMAGE_TYPE_32,
            layout::DUMP_TYPE_32,
            layout::SYSTEM_TIME_32,
        )
    };
    let needed = params_at + if is_64 { 32 } else { 16 };
    let too_short = || DumpParseError::TooShort {
        needed,
        actual: bytes.len(),
    };
    let bugcheck_code = u32_at(bytes, code_at).ok_or_else(too_short)?;
    let mut parameters = [0_u64; 4];
    for (index, parameter) in parameters.iter_mut().enumerate() {
        *parameter = if is_64 {
            u64_at(bytes, params_at + index * 8).ok_or_else(too_short)?
        } else {
            u64::from(u32_at(bytes, params_at + index * 4).ok_or_else(too_short)?)
        };
    }
    Ok(DumpHeader {
        is_64,
        bugcheck_code,
        parameters,
        dump_type: u32_at(bytes, dump_type_at).unwrap_or(0),
        system_time_filetime: u64_at(bytes, time_at)
            .map(|raw| i64::from_le_bytes(raw.to_le_bytes()))
            .filter(|value| *value > 0),
        machine_image_type: u32_at(bytes, machine_at).unwrap_or(0),
    })
}

/// The broad reason a code points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CauseCategory {
    /// A driver misbehaved: update or roll it back.
    Driver,
    /// RAM or memory-controller faults: test the memory.
    MemoryHardware,
    /// The disk or its controller: back up, then check the drive.
    Disk,
    /// Power delivery, overheating or a machine-check exception.
    PowerOrThermal,
    /// Corrupt system files: repair the Windows image.
    Software,
    /// Firmware/BIOS or hardware compatibility.
    Firmware,
    Unknown,
}

impl CauseCategory {
    /// The catalog remediation that answers this cause, when one exists.
    #[must_use]
    pub const fn remediation(self) -> Option<&'static str> {
        match self {
            Self::Driver => Some("open_device_manager"),
            Self::MemoryHardware => Some("schedule_memory_diagnostic"),
            Self::Software => Some("sfc_scannow"),
            Self::Disk | Self::PowerOrThermal | Self::Firmware | Self::Unknown => None,
        }
    }

    /// One sentence a home user can act on without a debugger.
    #[must_use]
    pub const fn next_action(self) -> &'static str {
        match self {
            Self::Driver => {
                "Update or roll back the driver named below (or recently installed drivers) in Device Manager."
            }
            Self::MemoryHardware => {
                "Run Windows Memory Diagnostic; if it reports errors, reseat or replace the RAM."
            }
            Self::Disk => {
                "Back up your files now, then run a disk check and review the SMART health of the drive."
            }
            Self::PowerOrThermal => {
                "Check temperatures, clean dust and fans, and verify the power supply; note whether it happens under load."
            }
            Self::Software => {
                "Repair Windows system files (SFC, then DISM) and uninstall recently added low-level software."
            }
            Self::Firmware => {
                "Check the PC or motherboard vendor's site for a BIOS/UEFI update and reset firmware settings to defaults."
            }
            Self::Unknown => {
                "Note the code and the faulting module, then search WindowsForum for that exact combination."
            }
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Driver => "usually a faulty driver",
            Self::MemoryHardware => "usually faulty RAM",
            Self::Disk => "usually the disk or its controller",
            Self::PowerOrThermal => "usually power, heat or a hardware fault",
            Self::Software => "usually corrupt system files",
            Self::Firmware => "usually firmware or hardware compatibility",
            Self::Unknown => "cause not recognised",
        }
    }
}

/// One decoded stop code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BugcheckInfo {
    pub code: u32,
    pub name: &'static str,
    pub plain: &'static str,
    pub cause: CauseCategory,
    pub next_action: &'static str,
    pub remediation: Option<&'static str>,
}

const TABLE: &[(u32, &str, &str, CauseCategory)] = &[
    (
        0x0000_000A,
        "IRQL_NOT_LESS_OR_EQUAL",
        "A driver touched memory it was not allowed to at a high priority level.",
        CauseCategory::Driver,
    ),
    (
        0x0000_001A,
        "MEMORY_MANAGEMENT",
        "Windows found an inconsistency in its memory bookkeeping.",
        CauseCategory::MemoryHardware,
    ),
    (
        0x0000_001E,
        "KMODE_EXCEPTION_NOT_HANDLED",
        "A kernel-mode program raised an exception nothing handled.",
        CauseCategory::Driver,
    ),
    (
        0x0000_0024,
        "NTFS_FILE_SYSTEM",
        "The NTFS file-system driver hit a problem, often a failing disk.",
        CauseCategory::Disk,
    ),
    (
        0x0000_003B,
        "SYSTEM_SERVICE_EXCEPTION",
        "A system service crashed while running.",
        CauseCategory::Driver,
    ),
    (
        0x0000_003D,
        "INTERRUPT_EXCEPTION_NOT_HANDLED",
        "An interrupt handler raised an exception.",
        CauseCategory::Driver,
    ),
    (
        0x0000_0050,
        "PAGE_FAULT_IN_NONPAGED_AREA",
        "Windows referenced memory that was not present.",
        CauseCategory::MemoryHardware,
    ),
    (
        0x0000_007A,
        "KERNEL_DATA_INPAGE_ERROR",
        "Windows could not read a page of kernel data from the disk.",
        CauseCategory::Disk,
    ),
    (
        0x0000_007E,
        "SYSTEM_THREAD_EXCEPTION_NOT_HANDLED",
        "A system thread raised an exception nothing handled.",
        CauseCategory::Driver,
    ),
    (
        0x0000_007F,
        "UNEXPECTED_KERNEL_MODE_TRAP",
        "The processor raised a trap the kernel did not expect (double fault).",
        CauseCategory::MemoryHardware,
    ),
    (
        0x0000_009F,
        "DRIVER_POWER_STATE_FAILURE",
        "A driver did not complete a power transition (sleep/wake) in time.",
        CauseCategory::Driver,
    ),
    (
        0x0000_00A5,
        "ACPI_BIOS_ERROR",
        "The firmware's ACPI tables are inconsistent with the hardware.",
        CauseCategory::Firmware,
    ),
    (
        0x0000_00C2,
        "BAD_POOL_CALLER",
        "A driver made an invalid memory-pool request.",
        CauseCategory::Driver,
    ),
    (
        0x0000_00C4,
        "DRIVER_VERIFIER_DETECTED_VIOLATION",
        "Driver Verifier caught a driver breaking the rules.",
        CauseCategory::Driver,
    ),
    (
        0x0000_00C5,
        "DRIVER_CORRUPTED_EXPOOL",
        "A driver corrupted the kernel memory pool.",
        CauseCategory::Driver,
    ),
    (
        0x0000_00D1,
        "DRIVER_IRQL_NOT_LESS_OR_EQUAL",
        "A driver accessed pageable memory at too high a priority level.",
        CauseCategory::Driver,
    ),
    (
        0x0000_00EF,
        "CRITICAL_PROCESS_DIED",
        "A process Windows cannot run without stopped unexpectedly.",
        CauseCategory::Software,
    ),
    (
        0x0000_00F4,
        "CRITICAL_OBJECT_TERMINATION",
        "A critical system process or thread ended unexpectedly.",
        CauseCategory::Disk,
    ),
    (
        0x0000_00F7,
        "DRIVER_OVERRAN_STACK_BUFFER",
        "A driver overran its stack buffer.",
        CauseCategory::Driver,
    ),
    (
        0x0000_0101,
        "CLOCK_WATCHDOG_TIMEOUT",
        "A processor core stopped responding to clock interrupts.",
        CauseCategory::PowerOrThermal,
    ),
    (
        0x0000_0116,
        "VIDEO_TDR_FAILURE",
        "The display driver stopped responding and could not be reset.",
        CauseCategory::Driver,
    ),
    (
        0x0000_0117,
        "VIDEO_TDR_TIMEOUT_DETECTED",
        "The display driver stopped responding (timeout).",
        CauseCategory::Driver,
    ),
    (
        0x0000_0119,
        "VIDEO_SCHEDULER_INTERNAL_ERROR",
        "The GPU scheduler hit an internal error.",
        CauseCategory::Driver,
    ),
    (
        0x0000_0124,
        "WHEA_UNCORRECTABLE_ERROR",
        "The hardware reported a fatal machine-check error.",
        CauseCategory::PowerOrThermal,
    ),
    (
        0x0000_0133,
        "DPC_WATCHDOG_VIOLATION",
        "A driver kept the processor busy for too long.",
        CauseCategory::Driver,
    ),
    (
        0x0000_0139,
        "KERNEL_SECURITY_CHECK_FAILURE",
        "The kernel detected corruption of a critical data structure.",
        CauseCategory::Driver,
    ),
    (
        0x0000_013A,
        "KERNEL_MODE_HEAP_CORRUPTION",
        "The kernel heap was corrupted.",
        CauseCategory::Driver,
    ),
    (
        0x0000_0154,
        "UNEXPECTED_STORE_EXCEPTION",
        "The memory-compression store hit an unexpected exception.",
        CauseCategory::Disk,
    ),
    (
        0xC000_021A,
        "STATUS_SYSTEM_PROCESS_TERMINATED",
        "A critical user-mode system process (winlogon/csrss) terminated.",
        CauseCategory::Software,
    ),
    (
        0xC000_0221,
        "STATUS_IMAGE_CHECKSUM_MISMATCH",
        "A system file failed its checksum — it is corrupt or was replaced.",
        CauseCategory::Software,
    ),
];

/// Decode a stop code; unknown codes keep their hex form.
#[must_use]
pub fn decode_bugcheck(code: u32) -> BugcheckInfo {
    TABLE.iter().find(|(known, ..)| *known == code).map_or(
        BugcheckInfo {
            code,
            name: "UNKNOWN_BUGCHECK",
            plain: "Windows stopped with a code the app does not recognise.",
            cause: CauseCategory::Unknown,
            next_action: CauseCategory::Unknown.next_action(),
            remediation: None,
        },
        |(_, name, plain, cause)| BugcheckInfo {
            code,
            name,
            plain,
            cause: *cause,
            next_action: cause.next_action(),
            remediation: cause.remediation(),
        },
    )
}

/// `0x000000D1`, the form users search for.
#[must_use]
pub fn format_code(code: u32) -> String {
    format!("0x{code:08X}")
}

/// Which bugcheck parameter (0-based) carries the faulting address, for the
/// codes whose documentation says so.
const fn address_parameter(code: u32) -> Option<usize> {
    match code {
        // 0x50 (PAGE_FAULT_IN_NONPAGED_AREA): Arg4 is reserved; the referenced
        // address lives in Arg1 (2026-09-03 audit).
        0x0000_000A | 0x0000_00D1 => Some(3),
        0x0000_0050 => Some(0),
        0x0000_001E | 0x0000_003B | 0x0000_007E => Some(1),
        _ => None,
    }
}

fn read_dump_string(bytes: &[u8], offset: usize) -> Option<String> {
    let length = u32_at(bytes, offset)?;
    if length == 0 || length > layout::MAX_DRIVER_NAME_CHARS {
        return None;
    }
    let start = offset.checked_add(4)?;
    let end = start.checked_add(usize::try_from(length).ok()?.checked_mul(2)?)?;
    let raw = bytes.get(start..end)?;
    let units: Vec<u16> = raw
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    let name = String::from_utf16_lossy(&units);
    let name = name.trim_end_matches('\0').trim();
    (!name.is_empty() && name.chars().all(|c| !c.is_control())).then(|| name.to_string())
}

struct DriverEntry {
    name: Option<String>,
    base: u64,
    size: u64,
}

fn read_driver_entry(bytes: &[u8], offset: usize) -> Option<DriverEntry> {
    let name_offset = u32_at(bytes, offset.checked_add(layout::DRIVER_ENTRY_NAME_OFFSET)?)?;
    let base = u64_at(bytes, offset.checked_add(layout::DRIVER_ENTRY_DLL_BASE_64)?)?;
    let size = u64::from(u32_at(
        bytes,
        offset.checked_add(layout::DRIVER_ENTRY_SIZE_OF_IMAGE_64)?,
    )?);
    let name = usize::try_from(name_offset)
        .ok()
        .and_then(|name_offset| read_dump_string(bytes, name_offset));
    Some(DriverEntry { name, base, size })
}

/// The module the kernel blamed, or the one whose image range contains the
/// faulting address. 64-bit triage dumps only; `None` whenever anything is
/// missing or malformed.
#[must_use]
pub fn faulting_module(bytes: &[u8], header: &DumpHeader) -> Option<String> {
    if !header.is_64 || header.dump_type != layout::DUMP_TYPE_TRIAGE {
        return None;
    }
    let broken = u32_at(bytes, layout::TRIAGE_BROKEN_DRIVER_OFFSET_64)?;
    if broken != 0
        && let Some(entry) = usize::try_from(broken)
            .ok()
            .and_then(|offset| read_driver_entry(bytes, offset))
        && let Some(name) = entry.name
    {
        return Some(name);
    }
    let address = address_parameter(header.bugcheck_code)
        .map(|index| header.parameters[index])
        .filter(|address| *address != 0)?;
    let list_offset = usize::try_from(u32_at(bytes, layout::TRIAGE_DRIVER_LIST_OFFSET_64)?).ok()?;
    let count = usize::try_from(u32_at(bytes, layout::TRIAGE_DRIVER_COUNT_64)?).ok()?;
    if list_offset == 0 || count == 0 || count > layout::MAX_DRIVERS {
        return None;
    }
    (0..count)
        .filter_map(|index| {
            read_driver_entry(
                bytes,
                list_offset.checked_add(index.checked_mul(layout::DRIVER_ENTRY_LEN_64)?)?,
            )
        })
        .find(|entry| {
            entry.size > 0
                && address >= entry.base
                && address < entry.base.saturating_add(entry.size)
        })
        .and_then(|entry| entry.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header_64(code: u32, parameters: [u64; 4]) -> Vec<u8> {
        let mut bytes = vec![0_u8; layout::HEADER_LEN];
        bytes[0..4].copy_from_slice(&layout::SIGNATURE.to_le_bytes());
        bytes[4..8].copy_from_slice(&layout::VALID_DUMP_64.to_le_bytes());
        bytes[layout::MACHINE_IMAGE_TYPE_64..layout::MACHINE_IMAGE_TYPE_64 + 4]
            .copy_from_slice(&0x8664_u32.to_le_bytes());
        bytes[layout::BUGCHECK_CODE_64..layout::BUGCHECK_CODE_64 + 4]
            .copy_from_slice(&code.to_le_bytes());
        for (index, parameter) in parameters.iter().enumerate() {
            let at = layout::BUGCHECK_PARAMETERS_64 + index * 8;
            bytes[at..at + 8].copy_from_slice(&parameter.to_le_bytes());
        }
        bytes[layout::DUMP_TYPE_64..layout::DUMP_TYPE_64 + 4]
            .copy_from_slice(&layout::DUMP_TYPE_TRIAGE.to_le_bytes());
        // 2026-01-01T00:00:00Z as FILETIME.
        let filetime: i64 = (1_767_225_600 + 11_644_473_600) * 10_000_000;
        bytes[layout::SYSTEM_TIME_64..layout::SYSTEM_TIME_64 + 8]
            .copy_from_slice(&filetime.to_le_bytes());
        bytes
    }

    fn put_u32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u64(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn put_dump_string(bytes: &mut [u8], at: usize, text: &str) {
        let units: Vec<u16> = text.encode_utf16().collect();
        put_u32(bytes, at, u32::try_from(units.len()).unwrap());
        for (index, unit) in units.iter().enumerate() {
            let offset = at + 4 + index * 2;
            bytes[offset..offset + 2].copy_from_slice(&unit.to_le_bytes());
        }
    }

    /// A triage block with two drivers and the given broken-driver offset.
    fn with_driver_list(mut bytes: Vec<u8>, broken: bool) -> Vec<u8> {
        bytes.resize(0x4000, 0);
        let list = 0x2100;
        let strings = 0x3000;
        put_u32(
            &mut bytes,
            layout::TRIAGE_DRIVER_LIST_OFFSET_64,
            u32::try_from(list).unwrap(),
        );
        put_u32(&mut bytes, layout::TRIAGE_DRIVER_COUNT_64, 2);
        // ntoskrnl at 0xFFFF_F800_0000_0000, 1 MiB; nvlddmkm at 0xFFFF_F801_0000_0000, 64 KiB.
        for (index, (name, base, size)) in [
            ("ntoskrnl.exe", 0xFFFF_F800_0000_0000_u64, 0x10_0000_u32),
            ("nvlddmkm.sys", 0xFFFF_F801_0000_0000_u64, 0x1_0000_u32),
        ]
        .iter()
        .enumerate()
        {
            let entry = list + index * layout::DRIVER_ENTRY_LEN_64;
            let name_at = strings + index * 0x100;
            put_dump_string(&mut bytes, name_at, name);
            put_u32(
                &mut bytes,
                entry + layout::DRIVER_ENTRY_NAME_OFFSET,
                u32::try_from(name_at).unwrap(),
            );
            put_u64(&mut bytes, entry + layout::DRIVER_ENTRY_DLL_BASE_64, *base);
            put_u32(
                &mut bytes,
                entry + layout::DRIVER_ENTRY_SIZE_OF_IMAGE_64,
                *size,
            );
        }
        if broken {
            put_u32(
                &mut bytes,
                layout::TRIAGE_BROKEN_DRIVER_OFFSET_64,
                u32::try_from(list + layout::DRIVER_ENTRY_LEN_64).unwrap(),
            );
        }
        bytes
    }

    #[test]
    fn parses_pagedu64_fixed_fields() {
        let bytes = header_64(0xD1, [0x10, 0x2, 0x0, 0xFFFF_F801_0000_1234]);
        let header = parse_dump_header(&bytes).unwrap();
        assert!(header.is_64);
        assert_eq!(header.bugcheck_code, 0xD1);
        assert_eq!(header.parameters[3], 0xFFFF_F801_0000_1234);
        assert_eq!(header.dump_type, layout::DUMP_TYPE_TRIAGE);
        assert_eq!(header.machine_image_type, 0x8664);
        assert_eq!(header.crash_time_unix_secs(), Some(1_767_225_600));
    }

    #[test]
    fn parses_32_bit_headers_too() {
        let mut bytes = vec![0_u8; layout::HEADER_LEN];
        put_u32(&mut bytes, 0, layout::SIGNATURE);
        put_u32(&mut bytes, 4, layout::VALID_DUMP_32);
        put_u32(&mut bytes, layout::BUGCHECK_CODE_32, 0x7E);
        put_u32(&mut bytes, layout::BUGCHECK_PARAMETERS_32 + 4, 0x8052_1234);
        let header = parse_dump_header(&bytes).unwrap();
        assert!(!header.is_64);
        assert_eq!(header.bugcheck_code, 0x7E);
        assert_eq!(header.parameters[1], 0x8052_1234);
        assert_eq!(header.system_time_filetime, None);
        assert_eq!(header.crash_time_unix_secs(), None);
    }

    #[test]
    fn rejects_bad_signature_and_short_buffers() {
        assert_eq!(
            parse_dump_header(&[0_u8; 4]),
            Err(DumpParseError::TooShort {
                needed: 8,
                actual: 4
            })
        );
        assert_eq!(
            parse_dump_header(b"MDMP\0\0\0\0"),
            Err(DumpParseError::BadSignature)
        );
        let mut bytes = header_64(0xD1, [0; 4]);
        put_u32(&mut bytes, 4, 0x1234_5678);
        assert_eq!(
            parse_dump_header(&bytes),
            Err(DumpParseError::UnknownValidDump(0x1234_5678))
        );
        let truncated = header_64(0xD1, [0; 4])[..0x50].to_vec();
        assert!(matches!(
            parse_dump_header(&truncated),
            Err(DumpParseError::TooShort { .. })
        ));
        // Anything past the fixed fields is optional.
        let short = header_64(0xD1, [0; 4])[..0x60].to_vec();
        let header = parse_dump_header(&short).unwrap();
        assert_eq!(header.bugcheck_code, 0xD1);
        assert_eq!(header.dump_type, 0);
        assert!(faulting_module(&short, &header).is_none());
    }

    #[test]
    fn broken_driver_offset_wins_over_address_lookup() {
        let bytes = with_driver_list(
            header_64(0xD1, [0x10, 0x2, 0x0, 0xFFFF_F800_0000_0010]),
            true,
        );
        let header = parse_dump_header(&bytes).unwrap();
        assert_eq!(
            faulting_module(&bytes, &header).as_deref(),
            Some("nvlddmkm.sys")
        );
    }

    #[test]
    fn address_lookup_finds_containing_module() {
        let bytes = with_driver_list(
            header_64(0xD1, [0x10, 0x2, 0x0, 0xFFFF_F801_0000_1234]),
            false,
        );
        let header = parse_dump_header(&bytes).unwrap();
        assert_eq!(
            faulting_module(&bytes, &header).as_deref(),
            Some("nvlddmkm.sys")
        );
        let outside = with_driver_list(header_64(0xD1, [0, 0, 0, 0x1000]), false);
        let header = parse_dump_header(&outside).unwrap();
        assert!(faulting_module(&outside, &header).is_none());
        let no_address_code = with_driver_list(header_64(0x124, [0, 0, 0, 0]), false);
        let header = parse_dump_header(&no_address_code).unwrap();
        assert!(faulting_module(&no_address_code, &header).is_none());
    }

    #[test]
    fn corrupt_driver_lists_never_panic() {
        let mut bytes = header_64(0xD1, [0, 0, 0, 0xFFFF_F801_0000_1234]);
        bytes.resize(0x2100, 0);
        put_u32(
            &mut bytes,
            layout::TRIAGE_DRIVER_LIST_OFFSET_64,
            0xFFFF_FFF0,
        );
        put_u32(&mut bytes, layout::TRIAGE_DRIVER_COUNT_64, 0xFFFF_FFFF);
        put_u32(
            &mut bytes,
            layout::TRIAGE_BROKEN_DRIVER_OFFSET_64,
            0xFFFF_FFF0,
        );
        let header = parse_dump_header(&bytes).unwrap();
        assert!(faulting_module(&bytes, &header).is_none());

        // A broken-driver pointer at garbage: its name is unreadable, so the
        // address lookup decides instead.
        let mut garbage_broken =
            with_driver_list(header_64(0xD1, [0, 0, 0, 0xFFFF_F801_0000_1234]), false);
        put_u32(
            &mut garbage_broken,
            layout::TRIAGE_BROKEN_DRIVER_OFFSET_64,
            0x3FF0,
        );
        let header = parse_dump_header(&garbage_broken).unwrap();
        assert_eq!(
            faulting_module(&garbage_broken, &header).as_deref(),
            Some("nvlddmkm.sys")
        );

        // An absurd string length is refused rather than allocated.
        let mut huge_name =
            with_driver_list(header_64(0xD1, [0, 0, 0, 0xFFFF_F801_0000_1234]), true);
        put_u32(&mut huge_name, 0x3100, 0x7FFF_FFFF);
        let header = parse_dump_header(&huge_name).unwrap();
        assert!(faulting_module(&huge_name, &header).is_none());
    }

    #[test]
    fn table_covers_top_codes_and_unknown_formats_hex() {
        let d1 = decode_bugcheck(0xD1);
        assert_eq!(d1.name, "DRIVER_IRQL_NOT_LESS_OR_EQUAL");
        assert_eq!(d1.cause, CauseCategory::Driver);
        assert_eq!(d1.remediation, Some("open_device_manager"));
        assert_eq!(decode_bugcheck(0x124).cause, CauseCategory::PowerOrThermal);
        assert_eq!(
            decode_bugcheck(0x1A).remediation,
            Some("schedule_memory_diagnostic")
        );
        assert_eq!(
            decode_bugcheck(0xC000_021A).remediation,
            Some("sfc_scannow")
        );
        let unknown = decode_bugcheck(0x0BAD_F00D);
        assert_eq!(unknown.name, "UNKNOWN_BUGCHECK");
        assert_eq!(unknown.cause, CauseCategory::Unknown);
        assert_eq!(format_code(unknown.code), "0x0BADF00D");
        let mut codes: Vec<u32> = TABLE.iter().map(|(code, ..)| *code).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), TABLE.len(), "duplicate code in the table");
        assert!(TABLE.len() >= 28);
        for (_, _, _, cause) in TABLE {
            if let Some(remediation) = cause.remediation() {
                assert!(
                    wfdiag_native_issues::remediation_catalog()
                        .iter()
                        .any(|metadata| metadata.id == remediation),
                    "{cause:?} -> {remediation}"
                );
            }
        }
    }
}
