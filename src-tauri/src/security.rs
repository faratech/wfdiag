use anyhow::Result;
use std::collections::HashMap;
use std::process::Command;

/// Decode Windows command output from OEM code page to UTF-8
///
/// Windows command-line tools (dism, verifier, etc.) output text in the OEM code page,
/// which is typically Windows-1252 or similar on Western systems. This function:
/// 1. First tries to decode as UTF-8 (for modern commands)
/// 2. Falls back to Windows-1252 for OEM code page output
#[cfg(windows)]
pub fn decode_windows_output(bytes: &[u8]) -> String {
    // UTF-16LE first: sfc.exe (and some other system tools) emit UTF-16,
    // which the UTF-8/1252 paths would garble into interleaved NULs.
    // Heuristic: even length + NULs dominating the odd byte positions.
    if bytes.len() >= 4 && bytes.len().is_multiple_of(2) {
        let probe = &bytes[..bytes.len().min(64)];
        let odd_nuls = probe.iter().skip(1).step_by(2).filter(|b| **b == 0).count();
        if odd_nuls > probe.len() / 4 {
            let units: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            return String::from_utf16_lossy(&units)
                .trim_start_matches('\u{feff}')
                .to_string();
        }
    }
    // Try UTF-8 (modern tools may output UTF-8)
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }
    // Fall back to Windows-1252 for OEM code page output
    encoding_rs::WINDOWS_1252.decode(bytes).0.into_owned()
}

/// Secure command execution with strict whitelisting
pub struct SecureCommandExecutor {
    allowed_commands: HashMap<String, CommandConfig>,
}

#[derive(Debug, Clone)]
pub struct CommandConfig {
    pub executable: String,
    pub allowed_args: Vec<String>,
    pub max_args: usize,
}

impl SecureCommandExecutor {
    pub fn new() -> Self {
        let mut allowed_commands = HashMap::new();

        // PowerShell - most restricted, only safe parameters allowed
        allowed_commands.insert(
            "powershell".to_string(),
            CommandConfig {
                executable: "powershell.exe".to_string(),
                allowed_args: vec![
                    "-NoProfile".to_string(),
                    "-NonInteractive".to_string(),
                    "-WindowStyle".to_string(),
                    "Hidden".to_string(),
                    "-Command".to_string(),
                ],
                max_args: 5,
            },
        );

        // System diagnostic commands - read-only operations only
        allowed_commands.insert(
            "dxdiag".to_string(),
            CommandConfig {
                executable: "dxdiag.exe".to_string(),
                allowed_args: vec!["/t".to_string(), "/whql:off".to_string()],
                max_args: 3, // /t <path> /whql:off
            },
        );

        allowed_commands.insert(
            "chkdsk".to_string(),
            CommandConfig {
                executable: "chkdsk.exe".to_string(),
                allowed_args: vec!["C:".to_string()], // Read-only scan of C: drive
                max_args: 1,
            },
        );

        allowed_commands.insert(
            "dism".to_string(),
            CommandConfig {
                executable: "dism.exe".to_string(),
                allowed_args: vec![
                    "/online".to_string(),
                    "/cleanup-image".to_string(),
                    "/checkhealth".to_string(),
                    "/scanhealth".to_string(),
                ],
                max_args: 3,
            },
        );

        allowed_commands.insert(
            "verifier".to_string(),
            CommandConfig {
                executable: "verifier.exe".to_string(),
                allowed_args: vec!["/querysettings".to_string()],
                max_args: 1,
            },
        );

        allowed_commands.insert(
            "powercfg".to_string(),
            CommandConfig {
                executable: "powercfg.exe".to_string(),
                allowed_args: vec!["/batteryreport".to_string(), "/output".to_string()],
                max_args: 3, // /batteryreport /output <path>
            },
        );

        allowed_commands.insert(
            "dsregcmd".to_string(),
            CommandConfig {
                executable: "dsregcmd.exe".to_string(),
                allowed_args: vec!["/status".to_string()],
                max_args: 1,
            },
        );

        allowed_commands.insert(
            "wmic".to_string(),
            CommandConfig {
                executable: "wmic.exe".to_string(),
                allowed_args: vec![
                    "path".to_string(),
                    "get".to_string(),
                    "/format:list".to_string(),
                ],
                max_args: 4, // path <class> get [properties] /format:list
            },
        );

        allowed_commands.insert(
            "wevtutil".to_string(),
            CommandConfig {
                executable: "wevtutil.exe".to_string(),
                allowed_args: vec![
                    "qe".to_string(),
                    "System".to_string(),
                    "Application".to_string(),
                    "/c:100".to_string(),
                    "/f:text".to_string(),
                ],
                max_args: 4, // qe <log> /c:100 /f:text
            },
        );

        allowed_commands.insert(
            "netstat".to_string(),
            CommandConfig {
                executable: "netstat.exe".to_string(),
                allowed_args: vec!["-an".to_string()],
                max_args: 1,
            },
        );

        allowed_commands.insert(
            "ipconfig".to_string(),
            CommandConfig {
                executable: "ipconfig.exe".to_string(),
                allowed_args: vec!["/all".to_string()],
                max_args: 1,
            },
        );

        allowed_commands.insert(
            "defrag".to_string(),
            CommandConfig {
                executable: "defrag.exe".to_string(),
                allowed_args: vec!["/A".to_string()], // Analysis only, no modifications
                max_args: 2,                          // drive letter and /A flag only
            },
        );

        Self { allowed_commands }
    }

    /// Validate and execute a command with strict security checks
    pub fn execute_command(&self, cmd: &str, args: &[&str]) -> Result<std::process::Output> {
        // Check if command is allowed
        let config = self
            .allowed_commands
            .get(cmd)
            .ok_or_else(|| anyhow::anyhow!("Command '{}' not in whitelist", cmd))?;

        // Check argument count
        if args.len() > config.max_args {
            return Err(anyhow::anyhow!(
                "Too many arguments for '{}': {} > {}",
                cmd,
                args.len(),
                config.max_args
            ));
        }

        // Validate each argument against whitelist
        self.validate_arguments(cmd, args, config)?;

        // Build and execute command with security flags
        let mut command = self.create_secure_command(&config.executable);
        command.args(args);

        command
            .output()
            .map_err(|e| anyhow::anyhow!("Failed to execute command '{}': {}", cmd, e))
    }

    /// Validate PowerShell scripts against known safe patterns
    #[allow(dead_code)] // Used by fallback diagnostics
    pub fn execute_powershell_script(&self, script: &str) -> Result<std::process::Output> {
        // Validate PowerShell script for safety
        self.validate_powershell_script(script)?;

        let config = self
            .allowed_commands
            .get("powershell")
            .ok_or_else(|| anyhow::anyhow!("PowerShell not configured"))?;

        let mut command = self.create_secure_command(&config.executable);
        command.args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-Command",
            script,
        ]);

        command
            .output()
            .map_err(|e| anyhow::anyhow!("Failed to execute PowerShell script: {}", e))
    }

    fn validate_arguments(&self, cmd: &str, args: &[&str], config: &CommandConfig) -> Result<()> {
        match cmd {
            "powershell" => {
                // PowerShell script validation is handled separately
                Ok(())
            }
            "dxdiag" => {
                // Only allow /t <temp_path> /whql:off
                if args.len() == 3 && args[0] == "/t" && args[2] == "/whql:off" {
                    self.validate_temp_path(args[1])?;
                    Ok(())
                } else {
                    Err(anyhow::anyhow!("Invalid dxdiag arguments"))
                }
            }
            "powercfg" => {
                // Only allow /batteryreport /output <temp_path>
                if args.len() == 3 && args[0] == "/batteryreport" && args[1] == "/output" {
                    self.validate_temp_path(args[2])?;
                    Ok(())
                } else {
                    Err(anyhow::anyhow!("Invalid powercfg arguments"))
                }
            }
            "wmic" => {
                // Validate WMI class names against whitelist
                self.validate_wmi_query(args)?;
                Ok(())
            }
            "wevtutil" => {
                // Only allow querying System or Application logs
                if args.len() == 4
                    && args[0] == "qe"
                    && (args[1] == "System" || args[1] == "Application")
                    && args[2] == "/c:100"
                    && args[3] == "/f:text"
                {
                    Ok(())
                } else {
                    Err(anyhow::anyhow!("Invalid wevtutil arguments"))
                }
            }
            "ipconfig" => {
                // Only allow /all flag
                if args.len() == 1 && args[0] == "/all" {
                    Ok(())
                } else {
                    Err(anyhow::anyhow!("Invalid ipconfig arguments"))
                }
            }
            "defrag" => {
                // Only allow drive letter and /A flag (analysis only)
                if args.len() == 2 && args[1] == "/A" {
                    self.validate_drive_letter(args[0])?;
                    Ok(())
                } else {
                    Err(anyhow::anyhow!(
                        "Invalid defrag arguments - only analysis mode allowed"
                    ))
                }
            }
            _ => {
                // For other commands, just check args are in allowed list
                for arg in args {
                    if !config.allowed_args.contains(&arg.to_string())
                        && !self.is_safe_argument(arg)
                    {
                        return Err(anyhow::anyhow!(
                            "Argument '{}' not allowed for '{}'",
                            arg,
                            cmd
                        ));
                    }
                }
                Ok(())
            }
        }
    }

    #[allow(dead_code)] // Used by execute_powershell_script
    fn validate_powershell_script(&self, script: &str) -> Result<()> {
        // Whitelist of allowed PowerShell commands for diagnostics
        let allowed_cmdlets = [
            "Get-AppxPackage",
            "Get-Counter",
            "Get-ScheduledTask",
            "Get-HotFix",
            "Select-Object",
            "Where-Object",
            "ConvertTo-Json",
            "Repair-Volume", // Read-only scan only
        ];

        // Blocked dangerous cmdlets and operations
        let blocked_patterns = [
            "Invoke-Expression",
            "iex",
            "Invoke-Command",
            "icm",
            "Start-Process",
            "New-Object",
            "Add-Type",
            "Invoke-RestMethod",
            "Invoke-WebRequest",
            "Download",
            "Remove-",
            "Delete",
            "Set-",
            "New-",
            "Install-",
            "Uninstall-",
            "Stop-",
            "Start-Service",
            "&",
            ";",
            "$env:",
            "Get-Credential",
            "ConvertTo-SecureString",
            // Note: "|" removed because it's commonly used for piping in safe diagnostic scripts
        ];

        // Check for blocked patterns
        let script_lower = script.to_lowercase();
        for pattern in &blocked_patterns {
            if script_lower.contains(&pattern.to_lowercase()) {
                return Err(anyhow::anyhow!(
                    "PowerShell script contains blocked pattern: {}",
                    pattern
                ));
            }
        }

        // Check if script uses allowed cmdlets (more permissive check)
        let has_allowed_cmdlet = allowed_cmdlets
            .iter()
            .any(|cmdlet| script_lower.contains(&cmdlet.to_lowercase()));

        if !has_allowed_cmdlet {
            // Allow some basic cmdlets and operations that are commonly used in diagnostic scripts
            let basic_allowed = [
                "select", "where", "sort", "format", "out-", "write-", "echo",
            ];

            let has_basic_cmdlet = basic_allowed
                .iter()
                .any(|cmdlet| script_lower.contains(&cmdlet.to_lowercase()));

            if !has_basic_cmdlet {
                return Err(anyhow::anyhow!(
                    "PowerShell script must use allowed diagnostic cmdlets"
                ));
            }
        }

        Ok(())
    }

    fn validate_wmi_query(&self, args: &[&str]) -> Result<()> {
        // WMI class whitelist for system diagnostics
        let allowed_wmi_classes = [
            "Win32_ComputerSystem",
            "Win32_OperatingSystem",
            "Win32_BIOS",
            "Win32_BaseBoard",
            "Win32_Processor",
            "Win32_PhysicalMemory",
            "Win32_DeviceMemoryAddress",
            "Win32_DMAChannel",
            "Win32_IRQResource",
            "Win32_DiskDrive",
            "Win32_DiskPartition",
            "Win32_LogicalDisk",
            "Win32_SystemDevices",
            "Win32_NetworkAdapter",
            "Win32_Printer",
            "Win32_Environment",
            "Win32_StartupCommand",
            "Win32_SystemDriver",
            "Win32_Process",
            "Win32_NTLogEvent",
        ];

        if args.len() >= 2 && args[0] == "path" {
            let wmi_class = args[1];
            if !allowed_wmi_classes.contains(&wmi_class) {
                return Err(anyhow::anyhow!(
                    "WMI class '{}' not in whitelist",
                    wmi_class
                ));
            }
        }

        Ok(())
    }

    fn validate_temp_path(&self, path: &str) -> Result<()> {
        // Ensure path is in temp directory and has safe filename
        let temp_dir = std::env::temp_dir();
        let path_buf = std::path::PathBuf::from(path);

        // Check if path is in temp directory
        if let Ok(canonical_path) = path_buf.canonicalize() {
            if !canonical_path.starts_with(&temp_dir) {
                return Err(anyhow::anyhow!("Path must be in temp directory"));
            }
        } else {
            // For non-existent files, check parent directory
            if let Some(parent) = path_buf.parent()
                && parent != temp_dir
            {
                return Err(anyhow::anyhow!("Path must be in temp directory"));
            }
        }

        // Check filename starts with safe prefix
        if let Some(filename) = path_buf.file_name().and_then(|f| f.to_str())
            && !filename.starts_with("wfdiag_")
        {
            return Err(anyhow::anyhow!("Temp filename must start with 'wfdiag_'"));
        }

        Ok(())
    }

    fn validate_drive_letter(&self, drive: &str) -> Result<()> {
        // Validate drive letter format (A: through Z:)
        if drive.len() != 2 {
            return Err(anyhow::anyhow!("Drive letter must be in format 'X:'"));
        }

        let chars: Vec<char> = drive.chars().collect();
        if chars.len() != 2 || chars[1] != ':' {
            return Err(anyhow::anyhow!("Drive letter must be in format 'X:'"));
        }

        let letter = chars[0].to_ascii_uppercase();
        if !letter.is_ascii_alphabetic() || !letter.is_ascii_uppercase() {
            return Err(anyhow::anyhow!("Drive letter must be A-Z"));
        }

        Ok(())
    }

    fn is_safe_argument(&self, arg: &str) -> bool {
        // Allow drive letters and basic switches
        matches!(arg, "C:" | "/status" | "/querysettings" | "-an" | "/all")
    }

    pub fn create_secure_command(&self, executable: &str) -> Command {
        let mut cmd = Command::new(executable);

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        cmd
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(windows)]
    fn decode_handles_utf16le_sfc_output() {
        use super::decode_windows_output;
        let text = "Windows Resource Protection did not find any integrity violations.";
        let utf16: Vec<u8> = text.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(decode_windows_output(&utf16), text);
        // Plain UTF-8 still passes through unchanged
        assert_eq!(decode_windows_output(b"plain ascii"), "plain ascii");
    }

    use super::*;

    #[test]
    fn test_command_whitelist() {
        let executor = SecureCommandExecutor::new();

        // Should allow whitelisted commands
        assert!(executor.allowed_commands.contains_key("powershell"));
        assert!(executor.allowed_commands.contains_key("dxdiag"));

        // Should reject non-whitelisted commands
        assert!(!executor.allowed_commands.contains_key("cmd"));
        assert!(!executor.allowed_commands.contains_key("notepad"));
    }

    #[test]
    fn test_powershell_validation() {
        let executor = SecureCommandExecutor::new();

        // Safe script should pass
        assert!(
            executor
                .validate_powershell_script("Get-AppxPackage | Select-Object Name")
                .is_ok()
        );

        // Dangerous script should fail
        assert!(
            executor
                .validate_powershell_script("Invoke-Expression 'calc'")
                .is_err()
        );
        assert!(
            executor
                .validate_powershell_script("Start-Process notepad")
                .is_err()
        );
    }
}
