use crate::native_diagnostics::NativeDiagnostics;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticTask {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: String,
    pub admin_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    pub success: bool,
    pub output: String,
    pub error: Option<String>,
    pub duration_ms: u64,
}

pub fn get_all_tasks() -> Vec<DiagnosticTask> {
    vec![
        // System Information
        DiagnosticTask {
            id: "comp_system".to_string(),
            name: "Computer System".to_string(),
            description: "Computer system information".to_string(),
            category: "System".to_string(),
            admin_required: false,
        },
        DiagnosticTask {
            id: "os_info".to_string(),
            name: "Operating System".to_string(),
            description: "Operating system details".to_string(),
            category: "System".to_string(),
            admin_required: false,
        },
        DiagnosticTask {
            id: "bios".to_string(),
            name: "BIOS".to_string(),
            description: "BIOS information".to_string(),
            category: "System".to_string(),
            admin_required: false,
        },
        DiagnosticTask {
            id: "baseboard".to_string(),
            name: "Motherboard".to_string(),
            description: "Motherboard details".to_string(),
            category: "Hardware".to_string(),
            admin_required: false,
        },
        DiagnosticTask {
            id: "processor".to_string(),
            name: "Processor".to_string(),
            description: "CPU information".to_string(),
            category: "Hardware".to_string(),
            admin_required: false,
        },
        DiagnosticTask {
            id: "physical_memory".to_string(),
            name: "Physical Memory".to_string(),
            description: "RAM modules information".to_string(),
            category: "Hardware".to_string(),
            admin_required: false,
        },
        DiagnosticTask {
            id: "device_memory".to_string(),
            name: "Device Memory Address".to_string(),
            description: "Memory address ranges".to_string(),
            category: "Hardware".to_string(),
            admin_required: false,
        },
        DiagnosticTask {
            id: "dma_channel".to_string(),
            name: "DMA Channels".to_string(),
            description: "Direct Memory Access channels".to_string(),
            category: "Hardware".to_string(),
            admin_required: false,
        },
        DiagnosticTask {
            id: "irq_resource".to_string(),
            name: "IRQ Resources".to_string(),
            description: "Interrupt request resources".to_string(),
            category: "Hardware".to_string(),
            admin_required: false,
        },
        DiagnosticTask {
            id: "npu_info".to_string(),
            name: "NPU (Neural Processing Unit)".to_string(),
            description: "AI accelerator / Neural Processing Unit detection".to_string(),
            category: "Hardware".to_string(),
            admin_required: false,
        },
        DiagnosticTask {
            id: "disk_drive".to_string(),
            name: "Disk Drives".to_string(),
            description: "Physical disk information".to_string(),
            category: "Storage".to_string(),
            admin_required: false,
        },
        DiagnosticTask {
            id: "disk_partition".to_string(),
            name: "Disk Partitions".to_string(),
            description: "Partition information".to_string(),
            category: "Storage".to_string(),
            admin_required: false,
        },
        DiagnosticTask {
            id: "logical_disk".to_string(),
            name: "Logical Disks".to_string(),
            description: "Logical disk drives with free space".to_string(),
            category: "Storage".to_string(),
            admin_required: false,
        },
        DiagnosticTask {
            id: "disk_fragmentation".to_string(),
            name: "Disk Fragmentation".to_string(),
            description: "Analyze disk fragmentation".to_string(),
            category: "Storage".to_string(),
            admin_required: true,
        },
        DiagnosticTask {
            id: "system_devices".to_string(),
            name: "System Devices".to_string(),
            description: "All system devices".to_string(),
            category: "Hardware".to_string(),
            admin_required: false,
        },
        DiagnosticTask {
            id: "network_adapter".to_string(),
            name: "Network Adapters".to_string(),
            description: "Network interface cards".to_string(),
            category: "Network".to_string(),
            admin_required: false,
        },
        DiagnosticTask {
            id: "printer".to_string(),
            name: "Printers".to_string(),
            description: "Installed printers".to_string(),
            category: "Hardware".to_string(),
            admin_required: false,
        },
        DiagnosticTask {
            id: "environment".to_string(),
            name: "Environment Variables".to_string(),
            description: "System environment variables".to_string(),
            category: "System".to_string(),
            admin_required: false,
        },
        DiagnosticTask {
            id: "startup_command".to_string(),
            name: "Startup Commands".to_string(),
            description: "Programs that run at startup".to_string(),
            category: "System".to_string(),
            admin_required: false,
        },
        DiagnosticTask {
            id: "system_driver".to_string(),
            name: "System Drivers".to_string(),
            description: "Installed system drivers".to_string(),
            category: "Drivers".to_string(),
            admin_required: false,
        },
        DiagnosticTask {
            id: "dxdiag".to_string(),
            name: "DirectX Diagnostics".to_string(),
            description: "DirectX and graphics information".to_string(),
            category: "Graphics".to_string(),
            admin_required: false,
        },
        DiagnosticTask {
            id: "systeminfo".to_string(),
            name: "System Information".to_string(),
            description: "Comprehensive system information".to_string(),
            category: "System".to_string(),
            admin_required: false,
        },
        DiagnosticTask {
            id: "drivers_list".to_string(),
            name: "Driver List".to_string(),
            description: "All installed drivers with versions".to_string(),
            category: "Drivers".to_string(),
            admin_required: false,
        },
        DiagnosticTask {
            id: "event_logs".to_string(),
            name: "Event Logs".to_string(),
            description: "System and Application event logs".to_string(),
            category: "Logs".to_string(),
            admin_required: false,
        },
        DiagnosticTask {
            id: "ipconfig".to_string(),
            name: "Network Configuration".to_string(),
            description: "IP configuration details".to_string(),
            category: "Network".to_string(),
            admin_required: false,
        },
        DiagnosticTask {
            id: "installed_programs".to_string(),
            name: "Installed Programs".to_string(),
            description: "List of installed software".to_string(),
            category: "Software".to_string(),
            admin_required: false,
        },
        DiagnosticTask {
            id: "store_apps".to_string(),
            name: "Windows Store Apps".to_string(),
            description: "Microsoft Store applications (requires Administrator)".to_string(),
            category: "Software".to_string(),
            admin_required: true,
        },
        DiagnosticTask {
            id: "services".to_string(),
            name: "System Services".to_string(),
            description: "Windows services status".to_string(),
            category: "System".to_string(),
            admin_required: false,
        },
        DiagnosticTask {
            id: "processes".to_string(),
            name: "Running Processes".to_string(),
            description: "Currently running processes".to_string(),
            category: "System".to_string(),
            admin_required: false,
        },
        DiagnosticTask {
            id: "performance".to_string(),
            name: "Performance Data".to_string(),
            description: "System performance counters".to_string(),
            category: "Performance".to_string(),
            admin_required: false,
        },
        DiagnosticTask {
            id: "hosts_file".to_string(),
            name: "HOSTS File".to_string(),
            description: "DNS hosts file contents".to_string(),
            category: "Network".to_string(),
            admin_required: false,
        },
        DiagnosticTask {
            id: "dsregcmd".to_string(),
            name: "Domain Registration".to_string(),
            description: "Domain join status".to_string(),
            category: "System".to_string(),
            admin_required: false,
        },
        DiagnosticTask {
            id: "scheduled_tasks".to_string(),
            name: "Scheduled Tasks".to_string(),
            description: "Windows Task Scheduler tasks".to_string(),
            category: "System".to_string(),
            admin_required: false,
        },
        DiagnosticTask {
            id: "windows_update".to_string(),
            name: "Windows Update Log".to_string(),
            description: "Windows Update history".to_string(),
            category: "Logs".to_string(),
            admin_required: false,
        },
        DiagnosticTask {
            id: "firewall_status".to_string(),
            name: "Firewall Status".to_string(),
            description: "Check firewall status".to_string(),
            category: "Security".to_string(),
            admin_required: false,
        },
        // Admin-only tasks
        DiagnosticTask {
            id: "chkdsk".to_string(),
            name: "Disk Check".to_string(),
            description: "Check disk for errors (read-only)".to_string(),
            category: "Storage".to_string(),
            admin_required: true,
        },
        DiagnosticTask {
            id: "dism_health".to_string(),
            name: "DISM Health Check".to_string(),
            description: "Windows image health status".to_string(),
            category: "System".to_string(),
            admin_required: true,
        },
        DiagnosticTask {
            id: "battery_report".to_string(),
            name: "Battery Report".to_string(),
            description: "Battery usage statistics".to_string(),
            category: "Hardware".to_string(),
            admin_required: true,
        },
        DiagnosticTask {
            id: "driver_verifier".to_string(),
            name: "Driver Verifier".to_string(),
            description: "Driver verifier settings".to_string(),
            category: "Drivers".to_string(),
            admin_required: true,
        },
        DiagnosticTask {
            id: "minidump".to_string(),
            name: "BSOD Minidumps".to_string(),
            description: "Blue screen crash dumps".to_string(),
            category: "Debug".to_string(),
            admin_required: true,
        },
    ]
}

// Synchronous wrapper for running diagnostic tasks (used by OpenAI integration)
pub fn run_diagnostic_task_sync(task_id: &str) -> Result<TaskResult, String> {
    // Use block_in_place to run the async function in the current runtime
    tokio::task::block_in_place(|| {
        // Get the current runtime handle
        let handle = tokio::runtime::Handle::current();

        // Block on the async function using the current runtime
        handle.block_on(async { Ok(run_diagnostic_task(task_id).await) })
    })
}

pub async fn run_diagnostic_task(task_id: &str) -> TaskResult {
    let start = std::time::Instant::now();

    // Run native diagnostics in a blocking task to avoid blocking the async runtime
    let task_id_owned = task_id.to_string();
    let native_result = tokio::task::spawn_blocking(move || {
        let diagnostics = match NativeDiagnostics::new() {
            Ok(d) => d,
            Err(e) => return Err(e),
        };

        match task_id_owned.as_str() {
            "comp_system" => diagnostics.run_wmi_query("Win32_ComputerSystem", None),
            "os_info" => diagnostics.run_wmi_query("Win32_OperatingSystem", None),
            "bios" => diagnostics.run_wmi_query("Win32_BIOS", None),
            "baseboard" => diagnostics.run_wmi_query("Win32_BaseBoard", None),
            "processor" => diagnostics.run_wmi_query("Win32_Processor", None),
            "physical_memory" => diagnostics.run_wmi_query("Win32_PhysicalMemory", None),
            "device_memory" => diagnostics.run_wmi_query("Win32_DeviceMemoryAddress", None),
            "dma_channel" => diagnostics.run_wmi_query("Win32_DMAChannel", None),
            "irq_resource" => diagnostics.run_wmi_query("Win32_IRQResource", None),
            "npu_info" => Ok(crate::native_monitor::get_npu_diagnostic_info()),
            "disk_drive" => diagnostics.run_wmi_query("Win32_DiskDrive", None),
            "disk_partition" => diagnostics.run_wmi_query("Win32_DiskPartition", None),
            "logical_disk" => diagnostics.get_native_disk_space(),
            "disk_fragmentation" => diagnostics.get_disk_fragmentation(),
            "system_devices" => diagnostics.run_wmi_query("Win32_SystemDevices", None),
            "network_adapter" => diagnostics.get_native_network_adapters(),
            "printer" => diagnostics.run_wmi_query("Win32_Printer", None),
            "environment" => diagnostics.run_wmi_query("Win32_Environment", None),
            "startup_command" => diagnostics.run_wmi_query("Win32_StartupCommand", None),
            "system_driver" => diagnostics.run_wmi_query("Win32_SystemDriver", None),
            "systeminfo" => diagnostics.get_native_system_info(),
            "drivers_list" => diagnostics.get_drivers(),
            "event_logs" => diagnostics.get_event_logs(),
            "installed_programs" => diagnostics.get_installed_programs(),
            "services" => diagnostics.get_native_services(),
            "processes" => diagnostics.run_wmi_query("Win32_Process", None),
            "dxdiag" => diagnostics.run_dxdiag(),
            "battery_report" => diagnostics.get_battery_report(),
            "minidump" => diagnostics.get_minidumps(),
            "chkdsk" => diagnostics.get_disk_health(),
            "dism_health" => diagnostics.run_dism_health(),
            "ipconfig" => diagnostics.run_ipconfig(),
            "hosts_file" => diagnostics.read_hosts_file(),
            "dsregcmd" => diagnostics.run_dsregcmd(),
            "windows_update" => diagnostics.get_windows_update_history(),
            "firewall_status" => {
                diagnostics.run_wmi_query("FirewallProduct", Some(r"root\SecurityCenter2"))
            }
            "store_apps" => diagnostics.get_store_apps(),
            "performance" => diagnostics.get_performance_data(),
            "scheduled_tasks" => diagnostics.get_scheduled_tasks(),
            "disk_health" => diagnostics.get_disk_health(),
            "driver_verifier" => diagnostics.get_driver_verifier(),
            _ => Err(anyhow::anyhow!("Not implemented in native diagnostics")),
        }
    })
    .await
    .unwrap_or_else(|_| Err(anyhow::anyhow!("Task panicked")));

    let result = match native_result {
        Ok(json_value) => TaskResult {
            success: true,
            output: serde_json::to_string_pretty(&json_value)
                .unwrap_or_else(|_| json_value.to_string()),
            error: None,
            duration_ms: 0,
        },
        Err(e) => {
            // Log the native diagnostic error for debugging
            let error_msg = format!("{:?}", e);
            eprintln!("Native diagnostic failed for {}: {}", task_id, error_msg);

            // Fallback to command-based diagnostics (no PowerShell)
            match task_id {
                "ipconfig" => run_command("ipconfig", &["/all"]),
                "hosts_file" => read_hosts_file(),
                "dsregcmd" => run_command("dsregcmd", &["/status"]),
                "dism_health" => {
                    run_command("dism", &["/online", "/cleanup-image", "/checkhealth"])
                }
                "driver_verifier" => run_command("verifier", &["/querysettings"]),
                // These now have native implementations, return detailed error
                "store_apps" | "performance" | "scheduled_tasks" | "chkdsk" | "windows_update" => {
                    TaskResult {
                        success: false,
                        output: String::new(),
                        error: Some(format!("Native diagnostic failed: {}", error_msg)),
                        duration_ms: 0,
                    }
                }
                _ => TaskResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("Unknown task: {}", task_id)),
                    duration_ms: 0,
                },
            }
        }
    };

    let mut result = result;
    result.duration_ms = start.elapsed().as_millis() as u64;
    result
}

fn run_command(cmd: &str, args: &[&str]) -> TaskResult {
    // Use secure command execution
    let executor = crate::security::SecureCommandExecutor::new();
    match executor.execute_command(cmd, args) {
        Ok(output) => {
            let output_str = String::from_utf8_lossy(&output.stdout).to_string();
            let error_str = String::from_utf8_lossy(&output.stderr).to_string();

            TaskResult {
                success: output.status.success(),
                output: output_str,
                error: if !error_str.is_empty() {
                    Some(error_str)
                } else {
                    None
                },
                duration_ms: 0,
            }
        }
        Err(e) => TaskResult {
            success: false,
            output: String::new(),
            error: Some(e.to_string()),
            duration_ms: 0,
        },
    }
}

fn read_hosts_file() -> TaskResult {
    let hosts_path = Path::new("C:\\Windows\\System32\\drivers\\etc\\hosts");

    match fs::read_to_string(hosts_path) {
        Ok(content) => TaskResult {
            success: true,
            output: content,
            error: None,
            duration_ms: 0,
        },
        Err(e) => TaskResult {
            success: false,
            output: String::new(),
            error: Some(format!("Failed to read hosts file: {}", e)),
            duration_ms: 0,
        },
    }
}
