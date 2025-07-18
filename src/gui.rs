use crate::{AppState, diagnostics};
use anyhow::Result;
use eframe::egui;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::task::JoinHandle;

pub struct DiagnosticApp {
    state: Arc<Mutex<AppState>>,
    output_dir: PathBuf,
    zip_path: PathBuf,
    task_handle: Option<JoinHandle<Result<()>>>,
}

impl DiagnosticApp {
    pub fn new(state: Arc<Mutex<AppState>>, output_dir: PathBuf, zip_path: PathBuf) -> Self {
        Self {
            state,
            output_dir,
            zip_path,
            task_handle: None,
        }
    }

    fn start_diagnostics(&mut self) {
        let state = Arc::clone(&self.state);
        let output_dir = self.output_dir.clone();
        let zip_path = self.zip_path.clone();

        // Update state to indicate start
        {
            let mut app_state = self.state.lock().unwrap();
            app_state.is_running = true;
            app_state.status_text = "Initializing...".to_string();
            app_state.progress = 0.0;
        }

        // Start background task
        self.task_handle = Some(tokio::spawn(async move {
            diagnostics::run_all_diagnostics(state, output_dir, zip_path).await
        }));
    }
}

impl eframe::App for DiagnosticApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let app_state = self.state.lock().unwrap();
        let is_complete = app_state.progress >= 1.0 && !app_state.is_running;
        let task_finished = self.task_handle.as_ref().map_or(true, |h| h.is_finished());
        let should_show_close = task_finished && is_complete;
        
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(10.0);
                
                // Progress bar
                ui.add(egui::ProgressBar::new(app_state.progress).show_percentage());
                
                ui.add_space(10.0);
                
                // Status text
                ui.label(&app_state.status_text);
                
                ui.add_space(10.0);
                
                // Current task
                if !app_state.current_task.is_empty() {
                    ui.label(format!("Current: {}", app_state.current_task));
                }
                
                // Task counter
                if app_state.total_tasks > 0 {
                    ui.label(format!("Completed {} of {} tasks", app_state.tasks_completed, app_state.total_tasks));
                }
                
                // Show close button when complete
                if should_show_close {
                    ui.add_space(20.0);
                    if ui.button("Close").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
            });
        });

        // Auto-start diagnostics when app opens (only once)
        if !app_state.is_running && self.task_handle.is_none() && !app_state.diagnostics_started {
            drop(app_state);
            
            // Mark that we've started diagnostics
            {
                let mut app_state = self.state.lock().unwrap();
                app_state.diagnostics_started = true;
            }
            
            self.start_diagnostics();
            return;
        }

        // Clean up finished task handles
        if let Some(handle) = &self.task_handle {
            if handle.is_finished() {
                self.task_handle = None;
            }
        }

        // Only request repaint while tasks are running or GUI elements need updates
        if !task_finished || !is_complete {
            ctx.request_repaint();
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Clean up any running tasks
        if let Some(handle) = &self.task_handle {
            handle.abort();
        }
    }
}