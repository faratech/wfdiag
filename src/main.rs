use std::sync::{Arc, Mutex};
use std::path::PathBuf;

mod admin;
mod diagnostics;
mod file_ops;
mod gui;

use gui::DiagnosticApp;

const VERSION: &str = "2.0.6";

#[derive(Default)]
pub struct AppState {
    pub progress: f32,
    pub status_text: String,
    pub current_task: String,
    pub is_running: bool,
    pub tasks_completed: usize,
    pub total_tasks: usize,
    pub is_admin: bool,
    pub diagnostics_started: bool,
}

#[tokio::main]
async fn main() {
    // Check admin privileges
    let is_admin = admin::is_running_as_admin();
    
    if !is_admin {
        show_admin_warning();
    }

    // Setup paths
    let desktop_path = dirs::desktop_dir().unwrap_or_else(|| PathBuf::from("."));
    let output_dir = desktop_path.join("WindowsForum");
    let zip_path = desktop_path.join("WF-Diag.zip");

    // Clean up existing files
    if output_dir.exists() {
        let _ = std::fs::remove_dir_all(&output_dir);
    }
    if zip_path.exists() {
        let _ = std::fs::remove_file(&zip_path);
    }

    // Create output directory
    let _ = std::fs::create_dir_all(&output_dir);
    let _ = std::fs::create_dir_all(output_dir.join("Minidump"));

    // Initialize app state
    let app_state = Arc::new(Mutex::new(AppState {
        is_admin,
        ..Default::default()
    }));

    // Run GUI
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([500.0, 200.0])
            .with_resizable(false),
        ..Default::default()
    };

    let _ = eframe::run_native(
        &format!("WindowsForum.com Diagnostic Tool {}", VERSION),
        options,
        Box::new(|_cc| Ok(Box::new(DiagnosticApp::new(app_state, output_dir, zip_path)))),
    );
}

fn show_admin_warning() {
    #[cfg(windows)]
    {
        use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_OK, MB_ICONINFORMATION, MB_TOPMOST};
        use windows::core::PCWSTR;
        
        let title = "Admin Rights Required\0".encode_utf16().collect::<Vec<u16>>();
        let message = "Admin rights are needed for some reports including BSOD Minidump. Running as a standard user may limit results.\0".encode_utf16().collect::<Vec<u16>>();
        
        unsafe {
            MessageBoxW(
                None,
                PCWSTR(message.as_ptr()),
                PCWSTR(title.as_ptr()),
                MB_OK | MB_ICONINFORMATION | MB_TOPMOST
            );
        }
    }
}