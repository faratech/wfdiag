use crate::{AppState, file_ops};
use anyhow::{Result, Context};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::process::Command;
use std::fs;
use sysinfo::System;
use serde::Deserialize;
use std::collections::HashMap;

pub struct DiagnosticTask {
    pub name: &'static str,
    pub admin_required: bool,
}

pub const DIAGNOSTIC_TASKS: &[DiagnosticTask] = &[
    DiagnosticTask { name: "Computer System", admin_required: false },
    DiagnosticTask { name: "Operating System", admin_required: false },
    DiagnosticTask { name: "BIOS", admin_required: false },
    DiagnosticTask { name: "BaseBoard", admin_required: false },
    DiagnosticTask { name: "Processor", admin_required: false },
    DiagnosticTask { name: "Physical Memory", admin_required: false },
    DiagnosticTask { name: "Device Memory Address", admin_required: false },
    DiagnosticTask { name: "DMA Channel", admin_required: false },
    DiagnosticTask { name: "IRQ Resource", admin_required: false },
    DiagnosticTask { name: "Disk Drive", admin_required: false },
    DiagnosticTask { name: "Disk Partition", admin_required: false },
    DiagnosticTask { name: "System Devices", admin_required: false },
    DiagnosticTask { name: "Network Adapter", admin_required: false },
    DiagnosticTask { name: "Printer", admin_required: false },
    DiagnosticTask { name: "Environment", admin_required: false },
    DiagnosticTask { name: "Startup Command", admin_required: false },
    DiagnosticTask { name: "System Driver", admin_required: false },
    DiagnosticTask { name: "DXDiag", admin_required: false },
    DiagnosticTask { name: "SystemInfo", admin_required: false },
    DiagnosticTask { name: "Drivers", admin_required: false },
    DiagnosticTask { name: "Event Logs", admin_required: false },
    DiagnosticTask { name: "IPConfig", admin_required: false },
    DiagnosticTask { name: "Installed Programs", admin_required: false },
    DiagnosticTask { name: "Windows Store Apps", admin_required: false },
    DiagnosticTask { name: "System Services", admin_required: false },
    DiagnosticTask { name: "Processes", admin_required: false },
    DiagnosticTask { name: "Performance Data", admin_required: false },
    DiagnosticTask { name: "HOSTS File", admin_required: false },
    DiagnosticTask { name: "Dsregcmd", admin_required: false },
    DiagnosticTask { name: "Scheduled Tasks", admin_required: false },
    DiagnosticTask { name: "Windows Update Log", admin_required: false },
    // Admin-only tasks
    DiagnosticTask { name: "Chkdsk", admin_required: true },
    DiagnosticTask { name: "DISM CheckHealth", admin_required: true },
    DiagnosticTask { name: "Battery Report", admin_required: true },
    DiagnosticTask { name: "Driver Verifier", admin_required: true },
    DiagnosticTask { name: "BSOD Minidump", admin_required: true },
];

pub async fn run_all_diagnostics(
    state: Arc<Mutex<AppState>>,
    output_dir: PathBuf,
    zip_path: PathBuf,
) -> Result<()> {
    let is_admin = {
        let app_state = state.lock().unwrap();
        app_state.is_admin
    };

    // Filter tasks based on admin privileges
    let tasks: Vec<_> = DIAGNOSTIC_TASKS.iter()
        .filter(|task| !task.admin_required || is_admin)
        .collect();

    let total_tasks = tasks.len();
    
    // Update state with total tasks
    {
        let mut app_state = state.lock().unwrap();
        app_state.total_tasks = total_tasks;
        app_state.tasks_completed = 0;
    }

    // Run each diagnostic task
    for (i, task) in tasks.iter().enumerate() {
        // Update current task
        {
            let mut app_state = state.lock().unwrap();
            app_state.current_task = task.name.to_string();
            app_state.status_text = format!("Running {}...", task.name);
        }

        // Execute the task
        let result = match task.name {
            "Computer System" => run_wmi_query("Win32_ComputerSystem", &output_dir, "CompSystem").await,
            "Operating System" => run_wmi_query("Win32_OperatingSystem", &output_dir, "OS").await,
            "BIOS" => run_wmi_query("Win32_BIOS", &output_dir, "BIOS").await,
            "BaseBoard" => run_wmi_query("Win32_BaseBoard", &output_dir, "BaseBoard").await,
            "Processor" => run_wmi_query("Win32_Processor", &output_dir, "Processor").await,
            "Physical Memory" => run_wmi_query("Win32_PhysicalMemory", &output_dir, "PhysicalMemory").await,
            "Device Memory Address" => run_wmi_query("Win32_DeviceMemoryAddress", &output_dir, "DevMemAddr").await,
            "DMA Channel" => run_wmi_query("Win32_DMAChannel", &output_dir, "DMAChannel").await,
            "IRQ Resource" => run_wmi_query("Win32_IRQResource", &output_dir, "IRQResource").await,
            "Disk Drive" => run_wmi_query("Win32_DiskDrive", &output_dir, "DiskDrive").await,
            "Disk Partition" => run_wmi_query("Win32_DiskPartition", &output_dir, "DiskPartition").await,
            "System Devices" => run_wmi_query("Win32_SystemDevices", &output_dir, "SysDevices").await,
            "Network Adapter" => run_wmi_query("Win32_NetworkAdapter", &output_dir, "NetAdapter").await,
            "Printer" => run_wmi_query("Win32_Printer", &output_dir, "Printer").await,
            "Environment" => run_wmi_query("Win32_Environment", &output_dir, "Environment").await,
            "Startup Command" => run_wmi_query("Win32_StartupCommand", &output_dir, "StartupCmd").await,
            "System Driver" => run_wmi_query("Win32_SystemDriver", &output_dir, "SysDriver").await,
            "DXDiag" => run_dxdiag(&output_dir).await,
            "SystemInfo" => run_systeminfo(&output_dir).await,
            "Drivers" => run_wmi_query("Win32_PnPSignedDriver", &output_dir, "DriversList").await,
            "Event Logs" => run_event_logs(&output_dir).await,
            "IPConfig" => run_ipconfig(&output_dir).await,
            "Installed Programs" => collect_installed_programs(&output_dir).await,
            "Windows Store Apps" => collect_store_apps(&output_dir).await,
            "System Services" => collect_services(&output_dir).await,
            "Processes" => collect_processes(&output_dir).await,
            "Performance Data" => collect_performance_data(&output_dir).await,
            "HOSTS File" => copy_hosts_file(&output_dir).await,
            "Dsregcmd" => run_dsregcmd(&output_dir).await,
            "Scheduled Tasks" => collect_scheduled_tasks(&output_dir).await,
            "Windows Update Log" => collect_windows_update_log(&output_dir).await,
            "Chkdsk" => run_chkdsk(&output_dir).await,
            "DISM CheckHealth" => run_dism_checkhealth(&output_dir).await,
            "Battery Report" => run_battery_report(&output_dir).await,
            "Driver Verifier" => run_driver_verifier(&output_dir).await,
            "BSOD Minidump" => collect_minidumps(&output_dir).await,
            _ => Ok(()),
        };

        // Log any errors but continue
        if let Err(e) = result {
            eprintln!("Error in task {}: {}", task.name, e);
        }

        // Update progress
        {
            let mut app_state = state.lock().unwrap();
            app_state.tasks_completed = i + 1;
            app_state.progress = (i + 1) as f32 / total_tasks as f32;
        }

        // Small delay to allow GUI updates
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    // Create zip file
    {
        let mut app_state = state.lock().unwrap();
        app_state.status_text = "Creating zip file...".to_string();
        app_state.current_task = "Compression".to_string();
    }

    file_ops::create_zip(&output_dir, &zip_path)?;

    // Final status
    {
        let mut app_state = state.lock().unwrap();
        app_state.status_text = format!("Complete! Results saved to {}", zip_path.display());
        app_state.current_task = "Finished".to_string();
        app_state.progress = 1.0;
        app_state.is_running = false;
    }

    // Open the zip file
    #[cfg(windows)]
    {
        Command::new("explorer")
            .arg(&zip_path)
            .spawn()
            .context("Failed to open zip file")?;
    }

    Ok(())
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct WmiObject {
    #[serde(flatten)]
    properties: HashMap<String, serde_json::Value>,
}

async fn run_wmi_query(
    class: &str,
    output_dir: &PathBuf,
    filename: &str,
) -> Result<()> {
    // Use native WMI calls through wmi crate with proper error handling
    let query = format!("SELECT * FROM {}", class);
    
    // Use blocking task to avoid Send/Sync issues with WMI
    let query_result = tokio::task::spawn_blocking({
        let query = query.clone();
        move || -> Result<String> {
            use wmi::{COMLibrary, WMIConnection};
            
            let com_con = COMLibrary::new()?;
            let wmi_con = WMIConnection::new(com_con.into())?;
            
            // Query with proper generic type
            let results: Vec<HashMap<String, wmi::Variant>> = wmi_con.raw_query(&query)?;
            
            let mut content = String::new();
            content.push_str(&format!("WMI Query: {}\n", query));
            content.push_str(&format!("Results Count: {}\n\n", results.len()));
            
            for (i, result) in results.iter().enumerate() {
                content.push_str(&format!("=== Object {} ===\n", i + 1));
                for (key, value) in result {
                    content.push_str(&format!("{}: {:?}\n", key, value));
                }
                content.push_str("\n");
            }
            
            Ok(content)
        }
    }).await??;
    
    let output_path = output_dir.join(format!("WindowsForum-{}.txt", filename));
    fs::write(output_path, query_result)?;
    Ok(())
}

async fn run_dxdiag(output_dir: &PathBuf) -> Result<()> {
    let output_path = output_dir.join("WindowsForum-DxDiag.txt");
    
    // Try to run DXDiag, but handle failures gracefully
    match Command::new("dxdiag")
        .args(&["/t", output_path.to_str().unwrap(), "/whql:off"])
        .spawn() {
        Ok(mut child) => {
            // Wait for the process to complete with timeout
            let timeout = tokio::time::Duration::from_secs(60); // 60 second timeout
            
            match tokio::time::timeout(timeout, async {
                child.wait()
            }).await {
                Ok(Ok(status)) => {
                    if !status.success() {
                        // Write error message to file instead of failing
                        let error_msg = format!("DXDiag failed with exit code: {:?}\nThis may indicate DirectX is not properly installed or accessible.", status.code());
                        fs::write(&output_path, error_msg)?;
                    }
                },
                Ok(Err(e)) => {
                    let error_msg = format!("DXDiag process error: {}\nThis may indicate DirectX is not properly installed.", e);
                    fs::write(&output_path, error_msg)?;
                },
                Err(_) => {
                    // Timeout - kill the process
                    let _ = child.kill();
                    let error_msg = "DXDiag timed out after 60 seconds.\nThis may indicate DirectX diagnostic issues.";
                    fs::write(&output_path, error_msg)?;
                }
            }
        },
        Err(e) => {
            // DXDiag not found or can't execute
            let error_msg = format!("DXDiag could not be executed: {}\nThis usually means DirectX diagnostic tools are not installed or not in PATH.\nThis is not critical for system diagnosis.", e);
            fs::write(&output_path, error_msg)?;
        }
    }
    
    Ok(())
}

async fn run_systeminfo(output_dir: &PathBuf) -> Result<()> {
    let output = Command::new("systeminfo").output()?;
    let output_path = output_dir.join("WindowsForum-SystemInfo.txt");
    fs::write(output_path, output.stdout)?;
    Ok(())
}

async fn run_event_logs(output_dir: &PathBuf) -> Result<()> {
    let logs = ["System", "Application"];
    for log in &logs {
        let output_path = output_dir.join(format!("WindowsForum-{}.evtx", log));
        Command::new("wevtutil")
            .args(&["epl", log, output_path.to_str().unwrap()])
            .output()?;
    }
    Ok(())
}

async fn run_ipconfig(output_dir: &PathBuf) -> Result<()> {
    let output = Command::new("ipconfig").arg("/all").output()?;
    let output_path = output_dir.join("WindowsForum-NetworkConfig.txt");
    fs::write(output_path, output.stdout)?;
    Ok(())
}

async fn collect_installed_programs(output_dir: &PathBuf) -> Result<()> {
    // Use registry to get installed programs on Windows
    #[cfg(windows)]
    {
        let query_result = tokio::task::spawn_blocking(move || -> Result<String> {
            use windows::Win32::System::Registry::*;
            use windows::core::PCSTR;
            
            let mut content = String::new();
            content.push_str("Installed Programs from Registry:\n\n");
            
            // Query both 32-bit and 64-bit program lists
            let keys = [
                r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
                r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
            ];
            
            for key_path in &keys {
                content.push_str(&format!("=== {} ===\n", key_path));
                
                unsafe {
                    let mut key = HKEY::default();
                    let result = RegOpenKeyExA(
                        HKEY_LOCAL_MACHINE,
                        PCSTR(key_path.as_ptr()),
                        Some(0),
                        KEY_READ,
                        &mut key,
                    );
                    
                    if result.is_ok() {
                        // Enumerate subkeys (would need more complex implementation)
                        content.push_str("Registry access successful (enumeration not fully implemented)\n");
                        let _ = RegCloseKey(key);
                    } else {
                        content.push_str("Registry access failed\n");
                    }
                }
                content.push_str("\n");
            }
            
            Ok(content)
        }).await??;
        
        let output_path = output_dir.join("WindowsForum-InstalledPrograms.txt");
        fs::write(output_path, query_result)?;
    }
    
    #[cfg(not(windows))]
    {
        let output_path = output_dir.join("WindowsForum-InstalledPrograms.txt");
        fs::write(output_path, "Installed programs collection only available on Windows")?;
    }
    
    Ok(())
}

async fn collect_store_apps(output_dir: &PathBuf) -> Result<()> {
    let output = Command::new("powershell")
        .args(&["-Command", "Get-AppxPackage | Select-Object Name, Version | Out-String"])
        .output()?;
    let output_path = output_dir.join("WindowsForum-StoreApps.txt");
    fs::write(output_path, output.stdout)?;
    Ok(())
}

async fn collect_services(output_dir: &PathBuf) -> Result<()> {
    let output = Command::new("sc").arg("query").output()?;
    let output_path = output_dir.join("WindowsForum-SystemServices.txt");
    fs::write(output_path, output.stdout)?;
    Ok(())
}

async fn collect_processes(output_dir: &PathBuf) -> Result<()> {
    let mut sys = System::new_all();
    sys.refresh_all();
    
    let mut content = String::new();
    for (pid, process) in sys.processes() {
        content.push_str(&format!("PID: {}, Name: {:?}, CPU: {:.2}%, Memory: {} KB\n", 
            pid, process.name(), process.cpu_usage(), process.memory()));
    }
    
    let output_path = output_dir.join("WindowsForum-RunningProcesses.txt");
    fs::write(output_path, content)?;
    Ok(())
}

async fn collect_performance_data(output_dir: &PathBuf) -> Result<()> {
    let output = Command::new("typeperf")
        .args(&["-qx"])
        .output()?;
    let output_path = output_dir.join("WindowsForum-PerformanceData.txt");
    fs::write(output_path, output.stdout)?;
    Ok(())
}

async fn copy_hosts_file(output_dir: &PathBuf) -> Result<()> {
    let hosts_path = PathBuf::from(r"C:\Windows\System32\drivers\etc\hosts");
    let output_path = output_dir.join("WindowsForum-HostsFile.txt");
    
    if hosts_path.exists() {
        fs::copy(hosts_path, output_path)?;
    }
    Ok(())
}

async fn run_dsregcmd(output_dir: &PathBuf) -> Result<()> {
    let output_path = output_dir.join("WindowsForum-DsRegCmd.txt");
    
    match Command::new("dsregcmd").arg("/status").output() {
        Ok(output) => {
            if output.status.success() {
                fs::write(output_path, output.stdout)?;
            } else {
                let error_msg = format!("dsregcmd failed with exit code: {:?}\nStderr: {}", 
                    output.status.code(), 
                    String::from_utf8_lossy(&output.stderr));
                fs::write(output_path, error_msg)?;
            }
        },
        Err(e) => {
            let error_msg = format!("dsregcmd could not be executed: {}\nThis command may not be available on this system.", e);
            fs::write(output_path, error_msg)?;
        }
    }
    
    Ok(())
}

async fn collect_scheduled_tasks(output_dir: &PathBuf) -> Result<()> {
    let output = Command::new("schtasks").arg("/query").output()?;
    let output_path = output_dir.join("WindowsForum-ScheduledTasks.txt");
    fs::write(output_path, output.stdout)?;
    Ok(())
}

async fn collect_windows_update_log(output_dir: &PathBuf) -> Result<()> {
    let output = Command::new("wevtutil")
        .args(&["qe", "Microsoft-Windows-WindowsUpdateClient/Operational", "/f:text"])
        .output()?;
    let output_path = output_dir.join("WindowsForum-WindowsUpdate.txt");
    fs::write(output_path, output.stdout)?;
    Ok(())
}

// Admin-only functions
async fn run_chkdsk(output_dir: &PathBuf) -> Result<()> {
    let output = Command::new("chkdsk")
        .args(&["C:", "/scan"])
        .output()?;
    let output_path = output_dir.join("WindowsForum-Chkdsk.txt");
    fs::write(output_path, output.stdout)?;
    Ok(())
}

async fn run_dism_checkhealth(output_dir: &PathBuf) -> Result<()> {
    let output = Command::new("dism")
        .args(&["/online", "/cleanup-image", "/checkhealth"])
        .output()?;
    let output_path = output_dir.join("WindowsForum-DISMCheckHealth.txt");
    fs::write(output_path, output.stdout)?;
    Ok(())
}

async fn run_battery_report(output_dir: &PathBuf) -> Result<()> {
    let output_path = output_dir.join("WindowsForum-BatteryReport.html");
    Command::new("powercfg")
        .args(&["/batteryreport", "/output", output_path.to_str().unwrap()])
        .output()?;
    Ok(())
}

async fn run_driver_verifier(output_dir: &PathBuf) -> Result<()> {
    let output = Command::new("verifier")
        .arg("/querysettings")
        .output()?;
    let output_path = output_dir.join("WindowsForum-DriverVerifierSettings.txt");
    fs::write(output_path, output.stdout)?;
    Ok(())
}

async fn collect_minidumps(output_dir: &PathBuf) -> Result<()> {
    let minidump_source = PathBuf::from(r"C:\Windows\Minidump");
    let minidump_dest = output_dir.join("Minidump");
    
    if minidump_source.exists() {
        // Copy the 3 most recent minidump files
        let mut entries: Vec<_> = fs::read_dir(&minidump_source)?
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry.path().extension()
                    .map_or(false, |ext| ext == "dmp")
            })
            .collect();
        
        entries.sort_by_key(|entry| {
            entry.metadata()
                .and_then(|meta| meta.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        });
        
        entries.reverse(); // Most recent first
        
        for entry in entries.into_iter().take(3) {
            let dest_path = minidump_dest.join(entry.file_name());
            fs::copy(entry.path(), dest_path)?;
        }
    }
    Ok(())
}