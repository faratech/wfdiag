//! Shared, portable decoders for raw evidence the Windows collectors gather
//! and the detectors read. Living here (the diagnostics crate depends on
//! this one) keeps one source of truth for what a code means and which
//! vetted remediation answers it.

pub mod windows_update {
    //! Windows Update client error codes, decoded into plain English and a
    //! remediation family.

    use serde::{Deserialize, Serialize};

    /// What a failure code points at, and therefore which catalog
    /// remediation is the right first move.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum WuFamily {
        /// The download cache or a partially installed package is damaged.
        CacheCorruption,
        /// The Windows component store needs repair before updates apply.
        ComponentStore,
        /// The Windows Update service itself is disabled.
        ServiceDisabled,
        /// Not enough free space to download or stage the update.
        DiskFull,
        /// A file or key is locked or access-denied (often security software).
        Permissions,
        /// The update servers could not be reached.
        Network,
        /// A previous update is waiting for a restart.
        RestartRequired,
        /// Not in the table; the generic reset is still the best first step.
        Unknown,
    }

    impl WuFamily {
        /// The catalog remediation to offer for this family.
        #[must_use]
        pub const fn remediation(self) -> &'static str {
            match self {
                Self::CacheCorruption | Self::Permissions | Self::Unknown => "windows_update_reset",
                Self::ComponentStore => "dism_restorehealth",
                Self::ServiceDisabled => "enable_windows_update_service",
                Self::DiskFull => "open_disk_cleanup",
                Self::Network => "open_network_settings",
                Self::RestartRequired => "restart_system",
            }
        }

        /// Short label for the issue description.
        #[must_use]
        pub const fn label(self) -> &'static str {
            match self {
                Self::CacheCorruption => "damaged update cache",
                Self::ComponentStore => "damaged Windows component store",
                Self::ServiceDisabled => "Windows Update service disabled",
                Self::DiskFull => "not enough free disk space",
                Self::Permissions => "a locked or access-denied file",
                Self::Network => "the update servers could not be reached",
                Self::RestartRequired => "a restart is still pending",
                Self::Unknown => "an unrecognised error",
            }
        }
    }

    /// One decoded HRESULT.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct HresultInfo {
        pub code: u32,
        /// The symbolic name Microsoft documents, or `HRESULT` when unknown.
        pub name: &'static str,
        /// One plain-English sentence.
        pub plain: &'static str,
        pub family: WuFamily,
    }

    /// Every code the table knows, with its plain-English reading.
    const TABLE: &[(u32, &str, &str, WuFamily)] = &[
        (
            0x8007_0002,
            "ERROR_FILE_NOT_FOUND",
            "A file the update needed is missing from the download cache.",
            WuFamily::CacheCorruption,
        ),
        (
            0x8007_0003,
            "ERROR_PATH_NOT_FOUND",
            "A folder the update needed is missing from the download cache.",
            WuFamily::CacheCorruption,
        ),
        (
            0x8024_0034,
            "WU_E_DOWNLOAD_FAILED",
            "The update download failed part way through.",
            WuFamily::CacheCorruption,
        ),
        (
            0x8024_0022,
            "WU_E_ALL_UPDATES_FAILED",
            "Every update in the batch failed to install.",
            WuFamily::CacheCorruption,
        ),
        (
            0x8024_200B,
            "WU_E_UH_INSTALLERFAILURE",
            "The update's installer stopped with an error.",
            WuFamily::CacheCorruption,
        ),
        (
            0x8007_0643,
            "ERROR_INSTALL_FAILURE",
            "The installer reported a fatal error.",
            WuFamily::CacheCorruption,
        ),
        (
            0x8024_2016,
            "WU_E_UH_POSTREBOOTUNEXPECTEDSTATE",
            "The last update expected a restart that has not happened yet.",
            WuFamily::RestartRequired,
        ),
        (
            0x8007_0BC2,
            "ERROR_SUCCESS_REBOOT_REQUIRED",
            "An earlier update is waiting for a restart before more can install.",
            WuFamily::RestartRequired,
        ),
        (
            0x800F_081F,
            "CBS_E_SOURCE_MISSING",
            "Windows could not find the component files it needs to repair itself.",
            WuFamily::ComponentStore,
        ),
        (
            0x800F_0831,
            "CBS_E_STORE_CORRUPTION",
            "The Windows component store is damaged.",
            WuFamily::ComponentStore,
        ),
        (
            0x800F_0982,
            "PSFX_E_MATCHING_COMPONENT_NOT_FOUND",
            "A component the update depends on is missing from the store.",
            WuFamily::ComponentStore,
        ),
        (
            0x800F_0922,
            "CBS_E_INSTALLERS_FAILED",
            "An update installer failed, often because the system reserved partition is full.",
            WuFamily::ComponentStore,
        ),
        (
            0x8007_3712,
            "ERROR_SXS_COMPONENT_STORE_CORRUPT",
            "The Windows component store is damaged.",
            WuFamily::ComponentStore,
        ),
        (
            0x8007_0490,
            "ERROR_NOT_FOUND",
            "A Windows component the update expected was not found.",
            WuFamily::ComponentStore,
        ),
        (
            0x8007_0422,
            "ERROR_SERVICE_DISABLED",
            "The Windows Update service is disabled, so nothing can install.",
            WuFamily::ServiceDisabled,
        ),
        (
            0x8007_0070,
            "ERROR_DISK_FULL",
            "The drive is full.",
            WuFamily::DiskFull,
        ),
        (
            0x8007_000E,
            "E_OUTOFMEMORY",
            "Windows ran out of memory or disk space while installing.",
            WuFamily::DiskFull,
        ),
        (
            0x8007_0005,
            "E_ACCESSDENIED",
            "Access was denied to a file or setting the update needed.",
            WuFamily::Permissions,
        ),
        (
            0x8007_0020,
            "ERROR_SHARING_VIOLATION",
            "A file the update needed was locked by another program.",
            WuFamily::Permissions,
        ),
        (
            0x8007_2EE2,
            "ERROR_INTERNET_TIMEOUT",
            "The connection to the update servers timed out.",
            WuFamily::Network,
        ),
        (
            0x8007_2EFD,
            "ERROR_INTERNET_CANNOT_CONNECT",
            "Windows could not connect to the update servers.",
            WuFamily::Network,
        ),
        (
            0x8007_2EFE,
            "ERROR_INTERNET_CONNECTION_ABORTED",
            "The connection to the update servers was cut off.",
            WuFamily::Network,
        ),
        (
            0x8007_2F8F,
            "ERROR_INTERNET_SECURE_FAILURE",
            "A secure connection to the update servers failed, often a wrong clock or a filtering proxy.",
            WuFamily::Network,
        ),
        (
            0x8024_4022,
            "WU_E_PT_HTTP_STATUS_SERVICE_UNAVAIL",
            "The update service was temporarily unavailable.",
            WuFamily::Network,
        ),
        (
            0x8024_4019,
            "WU_E_PT_HTTP_STATUS_NOT_FOUND",
            "The update server did not have the requested file.",
            WuFamily::Network,
        ),
        (
            0x8024_401C,
            "WU_E_PT_HTTP_STATUS_REQUEST_TIMEOUT",
            "The update server did not answer in time.",
            WuFamily::Network,
        ),
        (
            0x8024_0438,
            "WU_E_PT_ENDPOINT_UNREACHABLE",
            "The update endpoint could not be reached.",
            WuFamily::Network,
        ),
        (
            0x8024_402C,
            "WU_E_PT_WINHTTP_NAME_NOT_RESOLVED",
            "The update server's name could not be resolved (DNS).",
            WuFamily::Network,
        ),
    ];

    /// Parse an event's error code as the client writes it: `0x80070422`,
    /// a signed decimal (`-2147023838`) or an unsigned decimal.
    #[must_use]
    pub fn parse_error_code(text: &str) -> Option<u32> {
        let text = text.trim();
        if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
            return u32::from_str_radix(hex, 16).ok();
        }
        if let Ok(signed) = text.parse::<i64>() {
            return u32::try_from(signed)
                .ok()
                .or_else(|| i32::try_from(signed).ok().map(i32::cast_unsigned));
        }
        None
    }

    /// Decode a code; unknown codes keep their hex form and the generic
    /// reset family.
    #[must_use]
    pub fn decode_hresult(code: u32) -> HresultInfo {
        TABLE.iter().find(|(known, ..)| *known == code).map_or(
            HresultInfo {
                code,
                name: "HRESULT",
                plain: "Windows reported an error the app does not recognise.",
                family: WuFamily::Unknown,
            },
            |(_, name, plain, family)| HresultInfo {
                code,
                name,
                plain,
                family: *family,
            },
        )
    }

    /// `0x80070422` — the form users search for.
    #[must_use]
    pub fn format_code(code: u32) -> String {
        format!("0x{code:08X}")
    }

    /// Every family the table can produce, for exhaustive catalog checks.
    pub const ALL_FAMILIES: [WuFamily; 8] = [
        WuFamily::CacheCorruption,
        WuFamily::ComponentStore,
        WuFamily::ServiceDisabled,
        WuFamily::DiskFull,
        WuFamily::Permissions,
        WuFamily::Network,
        WuFamily::RestartRequired,
        WuFamily::Unknown,
    ];

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn parses_every_spelling_the_client_writes() {
            assert_eq!(parse_error_code("0x80070422"), Some(0x8007_0422));
            assert_eq!(parse_error_code(" 0X800F081F "), Some(0x800F_081F));
            assert_eq!(parse_error_code("-2147023838"), Some(0x8007_0422));
            assert_eq!(parse_error_code("2147943458"), Some(0x8007_0422));
            assert_eq!(parse_error_code(""), None);
            assert_eq!(parse_error_code("not a code"), None);
        }

        #[test]
        fn table_resolves_families_and_unknown_keeps_hex() {
            assert_eq!(
                decode_hresult(0x8007_0422).family,
                WuFamily::ServiceDisabled
            );
            assert_eq!(decode_hresult(0x800F_081F).family, WuFamily::ComponentStore);
            assert_eq!(decode_hresult(0x8007_0070).family, WuFamily::DiskFull);
            let unknown = decode_hresult(0xDEAD_BEEF);
            assert_eq!(unknown.family, WuFamily::Unknown);
            assert_eq!(unknown.name, "HRESULT");
            assert_eq!(format_code(unknown.code), "0xDEADBEEF");
            let mut codes: Vec<u32> = TABLE.iter().map(|(code, ..)| *code).collect();
            codes.sort_unstable();
            codes.dedup();
            assert_eq!(codes.len(), TABLE.len(), "duplicate code in the table");
        }

        #[test]
        fn every_family_maps_to_a_catalog_remediation() {
            for family in ALL_FAMILIES {
                assert!(
                    wfdiag_remediation_catalog::find(family.remediation()).is_some(),
                    "{family:?} -> {}",
                    family.remediation()
                );
            }
            assert_eq!(
                serde_json::to_value(WuFamily::ServiceDisabled).unwrap(),
                serde_json::json!("service_disabled")
            );
        }
    }
}
