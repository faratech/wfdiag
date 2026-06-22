use anyhow::Result;
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::process::Stdio;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(300);

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
        let mut command = self.create_secure_command(&config.executable)?;
        command.args(args);

        self.output_with_timeout(command, DEFAULT_COMMAND_TIMEOUT, cmd)
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

        let mut command = self.create_secure_command(&config.executable)?;
        command.args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-Command",
            script,
        ]);

        self.output_with_timeout(command, DEFAULT_COMMAND_TIMEOUT, "powershell")
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

    pub fn create_secure_command(&self, executable: &str) -> Result<Command> {
        let program = trusted_system_program(executable)?;
        let mut cmd = Command::new(program);

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        Ok(cmd)
    }

    fn output_with_timeout(
        &self,
        mut command: Command,
        timeout: Duration,
        label: &str,
    ) -> Result<std::process::Output> {
        let stdout_path = unique_output_path(label, "stdout")?;
        let stderr_path = unique_output_path(label, "stderr")?;
        let stdout_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&stdout_path)?;
        let stderr_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&stderr_path)?;

        command.stdout(Stdio::from(stdout_file));
        command.stderr(Stdio::from(stderr_file));

        let mut child = command
            .spawn()
            .map_err(|e| anyhow::anyhow!("Failed to execute command '{}': {}", label, e))?;
        let deadline = Instant::now() + timeout;

        loop {
            if let Some(status) = child
                .try_wait()
                .map_err(|e| anyhow::anyhow!("Failed while waiting for '{}': {}", label, e))?
            {
                let stdout = fs::read(&stdout_path).unwrap_or_default();
                let stderr = fs::read(&stderr_path).unwrap_or_default();
                let _ = fs::remove_file(&stdout_path);
                let _ = fs::remove_file(&stderr_path);
                return Ok(std::process::Output {
                    status,
                    stdout,
                    stderr,
                });
            }

            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                let _ = fs::remove_file(&stdout_path);
                let _ = fs::remove_file(&stderr_path);
                return Err(anyhow::anyhow!(
                    "Command '{}' timed out after {} second(s)",
                    label,
                    timeout.as_secs()
                ));
            }

            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

fn unique_output_path(label: &str, stream: &str) -> Result<PathBuf> {
    let safe_label: String = label
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    Ok(std::env::temp_dir().join(format!(
        "wfdiag_{}_{}_{}_{}.tmp",
        std::process::id(),
        nonce,
        safe_label,
        stream
    )))
}

#[cfg(windows)]
fn windows_root() -> PathBuf {
    std::env::var_os("SystemRoot")
        .or_else(|| std::env::var_os("WINDIR"))
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
}

#[cfg(windows)]
fn with_exe_extension(program: &str) -> String {
    if program.to_ascii_lowercase().ends_with(".exe") {
        program.to_string()
    } else {
        format!("{}.exe", program)
    }
}

/// Resolve a fixed Windows executable to a trusted OS location. This avoids
/// launching elevated diagnostics or remediations through the current directory
/// or PATH search order.
pub(crate) fn trusted_system_program(program: &str) -> Result<PathBuf> {
    if program.contains('\\') || program.contains('/') || Path::new(program).is_absolute() {
        return Err(anyhow::anyhow!(
            "Trusted command names must not include paths: {}",
            program
        ));
    }

    #[cfg(windows)]
    {
        let root = windows_root();
        let lower = program.to_ascii_lowercase();
        let path = match lower.as_str() {
            "powershell" | "powershell.exe" => root
                .join("System32")
                .join("WindowsPowerShell")
                .join("v1.0")
                .join("powershell.exe"),
            "wmic" | "wmic.exe" => root.join("System32").join("wbem").join("wmic.exe"),
            "explorer" | "explorer.exe" => root.join("explorer.exe"),
            _ => root.join("System32").join(with_exe_extension(program)),
        };
        if path.exists() {
            Ok(path)
        } else {
            Err(anyhow::anyhow!(
                "Trusted Windows command not found: {}",
                path.display()
            ))
        }
    }

    #[cfg(not(windows))]
    {
        Ok(PathBuf::from(program))
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
