#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use actix_cors::Cors;
use actix_web::{middleware::Logger, web, App, HttpServer};
use clap::{Parser, Subcommand};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use log::info;

mod admin;
mod api;
mod diagnostics;
mod file_ops;
mod models;
mod service;
mod websocket;

use models::*;
use service::DiagnosticService;

// AppState for compatibility with diagnostics module
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
    pub selected_tasks: Vec<bool>,
    pub task_outputs: Vec<String>,
    pub current_output: String,
}

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run diagnostics and output JSON
    Run {
        /// Comma-separated list of task IDs to run
        #[arg(short, long)]
        tasks: Option<String>,
        
        /// Output format: json, zip, or both
        #[arg(short, long, default_value = "json")]
        format: String,
    },
    
    /// List available diagnostic tasks
    List,
    
    /// Start the web server
    Server {
        /// Port to listen on
        #[arg(short, long, default_value = "8080")]
        port: u16,
        
        /// Host to bind to
        #[arg(short = 'H', long, default_value = "127.0.0.1")]
        host: String,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));
    
    let cli = Cli::parse();
    
    match cli.command {
        Some(Commands::Run { tasks, format }) => {
            run_cli_mode(tasks, format).await?;
        }
        Some(Commands::List) => {
            list_tasks().await?;
        }
        Some(Commands::Server { port, host }) => {
            run_server(host, port).await?;
        }
        None => {
            // Default to server mode
            run_server("127.0.0.1".to_string(), 8080).await?;
        }
    }
    
    Ok(())
}

async fn run_cli_mode(tasks: Option<String>, format: String) -> Result<(), Box<dyn std::error::Error>> {
    let is_admin = admin::is_running_as_admin();
    
    // Parse selected tasks
    let selected_tasks: Vec<String> = if let Some(tasks) = tasks {
        tasks.split(',').map(|s| s.trim().to_string()).collect()
    } else {
        // Run all available tasks by default
        diagnostics::get_filtered_tasks(is_admin)
            .iter()
            .map(|t| t.name.to_lowercase().replace(" ", "_"))
            .collect()
    };
    
    let output_format = match format.as_str() {
        "json" => OutputFormat::Json,
        "zip" => OutputFormat::Zip,
        "both" => OutputFormat::Both,
        _ => {
            eprintln!("Invalid format. Use 'json', 'zip', or 'both'");
            std::process::exit(1);
        }
    };
    
    // Create progress channel
    let (tx, mut rx) = mpsc::channel::<ProgressUpdate>(100);
    
    // Create service
    let service = DiagnosticService::new(tx);
    
    // Start diagnostics
    let request = DiagnosticRequest {
        selected_tasks,
        output_format: Some(output_format),
    };
    
    let session = service.start_diagnostics(request).await?;
    
    // Print initial session info
    println!("{}", serde_json::to_string_pretty(&ApiResponse::success(&session))?);
    
    // Monitor progress
    while let Some(update) = rx.recv().await {
        // Print progress updates to stderr so they don't interfere with JSON output
        eprintln!("Progress: {:.1}% - {}", 
            update.progress * 100.0,
            update.message
        );
        
        if matches!(update.status, SessionStatus::Completed | SessionStatus::Failed | SessionStatus::Cancelled) {
            break;
        }
    }
    
    // Get final session state
    if let Some(final_session) = service.get_session(session.id).await {
        println!("{}", serde_json::to_string_pretty(&ApiResponse::success(final_session))?);
    }
    
    Ok(())
}

async fn list_tasks() -> Result<(), Box<dyn std::error::Error>> {
    let is_admin = admin::is_running_as_admin();
    let tasks = diagnostics::get_filtered_tasks(is_admin)
        .iter()
        .map(|task| DiagnosticTask {
            id: task.name.to_lowercase().replace(" ", "_"),
            name: task.name.to_string(),
            description: get_task_description(task.name),
            admin_required: task.admin_required,
            category: get_task_category(task.name),
        })
        .collect::<Vec<_>>();
    
    println!("{}", serde_json::to_string_pretty(&ApiResponse::success(tasks))?);
    Ok(())
}

async fn run_server(host: String, port: u16) -> Result<(), Box<dyn std::error::Error>> {
    info!("Starting WFDiag Backend Server on {}:{}", host, port);
    
    // Create progress channel for WebSocket
    let (progress_tx, _progress_rx) = mpsc::channel::<ProgressUpdate>(1000);
    
    // Create diagnostic service
    let service = web::Data::new(DiagnosticService::new(progress_tx));
    
    // Check admin status
    if !admin::is_running_as_admin() {
        info!("⚠️  Running without administrator privileges. Some diagnostics will be unavailable.");
    } else {
        info!("✅ Running with administrator privileges.");
    }
    
    // Start server
    HttpServer::new(move || {
        App::new()
            .app_data(service.clone())
            .wrap(Logger::default())
            .wrap(
                Cors::default()
                    .allow_any_origin()
                    .allow_any_method()
                    .allow_any_header()
                    .max_age(3600)
            )
            .configure(api::configure_routes)
            .configure(websocket::configure_websocket)
            .route("/health", web::get().to(|| async { "OK" }))
    })
    .bind((host.as_str(), port))?
    .run()
    .await?;
    
    Ok(())
}

fn get_task_description(task_name: &str) -> String {
    match task_name {
        "Computer System" => "Hardware and system information",
        "Operating System" => "Windows version and configuration",
        "BIOS" => "Firmware and boot settings",
        "BaseBoard" => "Motherboard specifications",
        "Processor" => "CPU details and capabilities",
        "Physical Memory" => "RAM configuration and usage",
        "Network Adapter" => "Network interfaces and settings",
        "Disk Drive" => "Storage devices and partitions",
        "DXDiag" => "DirectX and graphics diagnostics",
        "System Services" => "Windows services status",
        "Processes" => "Running applications and tasks",
        "Event Logs" => "System and application logs",
        _ => "System diagnostic information",
    }.to_string()
}

fn get_task_category(task_name: &str) -> String {
    match task_name {
        "Computer System" | "Operating System" | "BIOS" | "BaseBoard" => "System",
        "Processor" | "Physical Memory" => "Hardware",
        "Network Adapter" | "IPConfig" => "Network",
        "Disk Drive" | "Disk Partition" | "Chkdsk" => "Storage",
        "System Services" | "Processes" | "Scheduled Tasks" => "Services",
        "Event Logs" | "Windows Update Log" => "Logs",
        "DXDiag" | "Drivers" | "Driver Verifier" => "Drivers",
        _ => "Other",
    }.to_string()
}