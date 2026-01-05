//! Process List UI Component - Full-featured htop-like process viewer
//!
//! Features:
//! - Sortable columns (click header to sort)
//! - Text filter/search
//! - Tree view with collapsible processes
//! - Color-coded CPU/Memory usage
//! - Scrollable virtualized list

use eframe::egui::{self, Color32, RichText, Sense, Vec2};
use std::collections::{HashMap, HashSet};

#[cfg(windows)]
use wfdiag_tauri::native_monitor::ProcessInfo;

/// Sort column options
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortColumn {
    Pid,
    PPid,
    Name,
    #[default]
    Cpu,
    Mem,
    Virt,
    Res,
    Shr,
    Threads,
    Handles,
    Priority,
    Time,
    Status,
    IoRead,
    IoWrite,
}

impl SortColumn {
    pub fn header(&self) -> &'static str {
        match self {
            SortColumn::Pid => "PID",
            SortColumn::PPid => "PPID",
            SortColumn::Name => "Name",
            SortColumn::Cpu => "CPU%",
            SortColumn::Mem => "MEM%",
            SortColumn::Virt => "VIRT",
            SortColumn::Res => "RES",
            SortColumn::Shr => "SHR",
            SortColumn::Threads => "THR",
            SortColumn::Handles => "HNDL",
            SortColumn::Priority => "PRI",
            SortColumn::Time => "TIME+",
            SortColumn::Status => "S",
            SortColumn::IoRead => "IO R",
            SortColumn::IoWrite => "IO W",
        }
    }

    pub fn width(&self) -> f32 {
        match self {
            SortColumn::Pid => 60.0,
            SortColumn::PPid => 60.0,
            SortColumn::Name => 180.0,
            SortColumn::Cpu => 55.0,
            SortColumn::Mem => 55.0,
            SortColumn::Virt => 70.0,
            SortColumn::Res => 70.0,
            SortColumn::Shr => 70.0,
            SortColumn::Threads => 45.0,
            SortColumn::Handles => 55.0,
            SortColumn::Priority => 40.0,
            SortColumn::Time => 80.0,
            SortColumn::Status => 30.0,
            SortColumn::IoRead => 70.0,
            SortColumn::IoWrite => 70.0,
        }
    }
}

/// Process list state
pub struct ProcessListState {
    /// Current sort column
    pub sort_column: SortColumn,
    /// Sort ascending (false = descending)
    pub sort_ascending: bool,
    /// Filter text
    pub filter_text: String,
    /// Show tree view
    pub tree_view: bool,
    /// Collapsed PIDs in tree view
    pub collapsed_pids: HashSet<u32>,
    /// Selected PID
    pub selected_pid: Option<u32>,
    /// Visible columns
    pub visible_columns: Vec<SortColumn>,
    /// Show kernel/system processes
    pub show_system_processes: bool,
    /// Scroll offset for virtualization
    pub scroll_offset: f32,
}

impl Default for ProcessListState {
    fn default() -> Self {
        Self {
            sort_column: SortColumn::Cpu,
            sort_ascending: false,
            filter_text: String::new(),
            tree_view: false,
            collapsed_pids: HashSet::new(),
            selected_pid: None,
            visible_columns: vec![
                SortColumn::Pid,
                SortColumn::Name,
                SortColumn::Cpu,
                SortColumn::Mem,
                SortColumn::Res,
                SortColumn::Threads,
                SortColumn::Time,
            ],
            show_system_processes: true,
            scroll_offset: 0.0,
        }
    }
}

#[cfg(windows)]
impl ProcessListState {
    /// Sort processes according to current settings
    pub fn sort_processes(&self, processes: &mut [ProcessInfo]) {
        let ascending = self.sort_ascending;

        processes.sort_by(|a, b| {
            let cmp = match self.sort_column {
                SortColumn::Pid => a.pid.cmp(&b.pid),
                SortColumn::PPid => a.parent_pid.cmp(&b.parent_pid),
                SortColumn::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                SortColumn::Cpu => a.cpu_percent.partial_cmp(&b.cpu_percent).unwrap_or(std::cmp::Ordering::Equal),
                SortColumn::Mem => a.memory_percent.partial_cmp(&b.memory_percent).unwrap_or(std::cmp::Ordering::Equal),
                SortColumn::Virt => a.virtual_memory_mb.partial_cmp(&b.virtual_memory_mb).unwrap_or(std::cmp::Ordering::Equal),
                SortColumn::Res => a.memory_mb.partial_cmp(&b.memory_mb).unwrap_or(std::cmp::Ordering::Equal),
                SortColumn::Shr => a.shared_memory_mb.partial_cmp(&b.shared_memory_mb).unwrap_or(std::cmp::Ordering::Equal),
                SortColumn::Threads => a.thread_count.cmp(&b.thread_count),
                SortColumn::Handles => a.handle_count.cmp(&b.handle_count),
                SortColumn::Priority => a.priority.cmp(&b.priority),
                SortColumn::Time => a.cpu_time_secs.cmp(&b.cpu_time_secs),
                SortColumn::Status => a.status.cmp(&b.status),
                SortColumn::IoRead => a.io_read_bytes.cmp(&b.io_read_bytes),
                SortColumn::IoWrite => a.io_write_bytes.cmp(&b.io_write_bytes),
            };

            if ascending { cmp } else { cmp.reverse() }
        });
    }

    /// Filter processes based on search text
    pub fn filter_processes<'a>(&self, processes: &'a [ProcessInfo]) -> Vec<&'a ProcessInfo> {
        let filter_lower = self.filter_text.to_lowercase();

        processes
            .iter()
            .filter(|p| {
                // System process filter
                if !self.show_system_processes {
                    if p.pid == 0 || p.pid == 4 || p.name.eq_ignore_ascii_case("System") {
                        return false;
                    }
                }

                // Text filter
                if !filter_lower.is_empty() {
                    let matches = p.name_lower.contains(&filter_lower)
                        || p.command_lower.contains(&filter_lower)
                        || p.pid.to_string().contains(&filter_lower);
                    if !matches {
                        return false;
                    }
                }

                true
            })
            .collect()
    }

    /// Build tree view from flat process list
    pub fn build_tree(&self, processes: &mut Vec<ProcessInfo>) {
        // Build parent-child map
        let pid_set: HashSet<u32> = processes.iter().map(|p| p.pid).collect();
        let mut children_map: HashMap<u32, Vec<usize>> = HashMap::new();

        for (idx, proc) in processes.iter().enumerate() {
            if proc.parent_pid != 0 && proc.parent_pid != proc.pid && pid_set.contains(&proc.parent_pid) {
                children_map.entry(proc.parent_pid).or_default().push(idx);
            }
        }

        // Mark which processes have children
        for proc in processes.iter_mut() {
            proc.has_children = children_map.contains_key(&proc.pid);
            proc.is_collapsed = self.collapsed_pids.contains(&proc.pid);
        }

        // Build ordered tree
        let mut result = Vec::with_capacity(processes.len());
        let mut visited = HashSet::new();

        // Find roots (processes with no parent in list)
        let mut roots: Vec<usize> = processes
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                p.parent_pid == 0 || p.parent_pid == p.pid || !pid_set.contains(&p.parent_pid)
            })
            .map(|(i, _)| i)
            .collect();

        // Sort roots by current sort column
        roots.sort_by(|&a, &b| {
            processes[a].pid.cmp(&processes[b].pid)
        });

        fn add_node(
            idx: usize,
            depth: usize,
            prefix: &str,
            is_last: bool,
            processes: &[ProcessInfo],
            children_map: &HashMap<u32, Vec<usize>>,
            collapsed_pids: &HashSet<u32>,
            result: &mut Vec<ProcessInfo>,
            visited: &mut HashSet<usize>,
        ) {
            if visited.contains(&idx) {
                return;
            }
            visited.insert(idx);

            let mut proc = processes[idx].clone();
            proc.tree_depth = depth;

            // Build tree prefix
            if depth > 0 {
                let connector = if is_last { "└─ " } else { "├─ " };
                proc.tree_prefix = format!("{}{}", prefix, connector);
            }

            let pid = proc.pid;
            let is_collapsed = collapsed_pids.contains(&pid);
            proc.is_collapsed = is_collapsed;

            result.push(proc);

            // Add children if not collapsed
            if !is_collapsed {
                if let Some(children) = children_map.get(&pid) {
                    let child_prefix = if depth > 0 {
                        format!("{}{}  ", prefix, if is_last { "   " } else { "│  " })
                    } else {
                        String::new()
                    };

                    let mut sorted_children = children.clone();
                    sorted_children.sort_by(|&a, &b| processes[a].pid.cmp(&processes[b].pid));

                    for (i, &child_idx) in sorted_children.iter().enumerate() {
                        let child_is_last = i == sorted_children.len() - 1;
                        add_node(
                            child_idx,
                            depth + 1,
                            &child_prefix,
                            child_is_last,
                            processes,
                            children_map,
                            collapsed_pids,
                            result,
                            visited,
                        );
                    }
                }
            }
        }

        for (i, &root_idx) in roots.iter().enumerate() {
            add_node(
                root_idx,
                0,
                "",
                i == roots.len() - 1,
                processes,
                &children_map,
                &self.collapsed_pids,
                &mut result,
                &mut visited,
            );
        }

        *processes = result;
    }

    /// Toggle collapse state for a PID
    pub fn toggle_collapse(&mut self, pid: u32) {
        if self.collapsed_pids.contains(&pid) {
            self.collapsed_pids.remove(&pid);
        } else {
            self.collapsed_pids.insert(pid);
        }
    }
}

/// Draw the process list UI
#[cfg(windows)]
pub fn draw_process_list(
    ui: &mut egui::Ui,
    state: &mut ProcessListState,
    processes: &[ProcessInfo],
) {
    // Toolbar
    ui.horizontal(|ui| {
        // Filter input
        ui.label("Filter:");
        let filter_response = ui.add(
            egui::TextEdit::singleline(&mut state.filter_text)
                .desired_width(150.0)
                .hint_text("Search...")
        );
        if filter_response.changed() {
            // Filter changed
        }

        ui.separator();

        // Tree view toggle
        if ui.selectable_label(state.tree_view, "🌲 Tree").clicked() {
            state.tree_view = !state.tree_view;
        }

        // System processes toggle
        if ui.selectable_label(state.show_system_processes, "⚙ System").clicked() {
            state.show_system_processes = !state.show_system_processes;
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(format!("{} processes", processes.len())).size(11.0).weak());
        });
    });

    ui.add_space(4.0);

    // Prepare processes
    let mut display_processes: Vec<ProcessInfo> = state.filter_processes(processes)
        .into_iter()
        .cloned()
        .collect();

    if state.tree_view {
        state.build_tree(&mut display_processes);
    } else {
        state.sort_processes(&mut display_processes);
    }

    // Table with headers
    let available_height = ui.available_height();
    let row_height = 20.0;

    egui::Frame::new()
        .fill(ui.visuals().extreme_bg_color)
        .corner_radius(4.0)
        .show(ui, |ui| {
            // Header row
            ui.horizontal(|ui| {
                ui.set_height(24.0);
                ui.spacing_mut().item_spacing.x = 0.0;

                for col in &state.visible_columns {
                    let width = col.width();
                    let header = col.header();
                    let is_sorted = state.sort_column == *col;

                    let sort_indicator = if is_sorted {
                        if state.sort_ascending { " ▲" } else { " ▼" }
                    } else {
                        ""
                    };

                    let text = format!("{}{}", header, sort_indicator);
                    let text_color = if is_sorted {
                        Color32::from_rgb(96, 165, 250)
                    } else {
                        ui.visuals().text_color()
                    };

                    let (rect, response) = ui.allocate_exact_size(
                        Vec2::new(width, 24.0),
                        Sense::click()
                    );

                    if response.clicked() {
                        if state.sort_column == *col {
                            state.sort_ascending = !state.sort_ascending;
                        } else {
                            state.sort_column = *col;
                            state.sort_ascending = false;
                        }
                    }

                    if response.hovered() {
                        ui.painter().rect_filled(rect, 0.0, Color32::from_white_alpha(10));
                    }

                    ui.painter().text(
                        rect.left_center() + Vec2::new(4.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        text,
                        egui::FontId::proportional(11.0),
                        text_color,
                    );
                }
            });

            ui.separator();

            // Clone visible columns to avoid borrow conflict
            let visible_columns = state.visible_columns.clone();
            let tree_view = state.tree_view;
            let is_dark = ui.visuals().dark_mode;

            // Track toggle action to apply after loop
            let mut toggle_pid: Option<u32> = None;

            // Process rows with scroll
            egui::ScrollArea::vertical()
                .max_height(available_height - 60.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for proc in &display_processes {
                        let is_selected = state.selected_pid == Some(proc.pid);
                        let row_bg = if is_selected {
                            Color32::from_rgb(59, 130, 246).linear_multiply(0.3)
                        } else {
                            Color32::TRANSPARENT
                        };

                        ui.horizontal(|ui| {
                            ui.set_height(row_height);
                            ui.spacing_mut().item_spacing.x = 0.0;

                            // Draw row background
                            let row_rect = ui.available_rect_before_wrap();
                            if row_bg != Color32::TRANSPARENT {
                                ui.painter().rect_filled(row_rect, 0.0, row_bg);
                            }

                            for col in &visible_columns {
                                let width = col.width();
                                let (rect, response) = ui.allocate_exact_size(
                                    Vec2::new(width, row_height),
                                    Sense::click()
                                );

                                if response.clicked() {
                                    state.selected_pid = Some(proc.pid);
                                    if tree_view && proc.has_children {
                                        toggle_pid = Some(proc.pid);
                                    }
                                }

                                if response.hovered() && !is_selected {
                                    ui.painter().rect_filled(rect, 0.0, Color32::from_white_alpha(5));
                                }

                                let (text, color) = format_cell(col, proc, tree_view, is_dark);

                                ui.painter().text(
                                    rect.left_center() + Vec2::new(4.0, 0.0),
                                    egui::Align2::LEFT_CENTER,
                                    text,
                                    egui::FontId::proportional(11.0),
                                    color,
                                );
                            }
                        });
                    }
                });

            // Apply deferred toggle action
            if let Some(pid) = toggle_pid {
                state.toggle_collapse(pid);
            }
        });
}

/// Format a cell value with appropriate coloring
#[cfg(windows)]
fn format_cell(col: &SortColumn, proc: &ProcessInfo, tree_view: bool, is_dark: bool) -> (String, Color32) {
    // For light mode, use dark text; for dark mode, use light text
    let text_color = if is_dark {
        Color32::from_rgb(229, 231, 235) // Light gray for dark mode
    } else {
        Color32::from_rgb(31, 41, 55) // Dark gray for light mode
    };
    let muted_color = if is_dark {
        Color32::from_rgb(156, 163, 175)
    } else {
        Color32::from_rgb(107, 114, 128)
    };
    let dim_color = if is_dark {
        Color32::from_rgb(107, 114, 128)
    } else {
        Color32::from_rgb(156, 163, 175)
    };

    match col {
        SortColumn::Pid => (proc.pid.to_string(), muted_color),

        SortColumn::PPid => (proc.parent_pid.to_string(), dim_color),

        SortColumn::Name => {
            let prefix = if tree_view {
                let collapse_indicator = if proc.has_children {
                    if proc.is_collapsed { "[+] " } else { "[-] " }
                } else {
                    "    "
                };
                format!("{}{}", proc.tree_prefix, collapse_indicator)
            } else {
                String::new()
            };
            (format!("{}{}", prefix, proc.name), text_color)
        }

        SortColumn::Cpu => {
            let color = if proc.cpu_percent >= 80.0 {
                Color32::from_rgb(239, 68, 68) // Red
            } else if proc.cpu_percent >= 50.0 {
                Color32::from_rgb(245, 158, 11) // Orange
            } else if proc.cpu_percent >= 10.0 {
                Color32::from_rgb(16, 185, 129) // Green
            } else {
                Color32::from_rgb(156, 163, 175) // Gray
            };
            (format!("{:.1}", proc.cpu_percent), color)
        }

        SortColumn::Mem => {
            let color = if proc.memory_percent >= 50.0 {
                Color32::from_rgb(239, 68, 68)
            } else if proc.memory_percent >= 20.0 {
                Color32::from_rgb(245, 158, 11)
            } else if proc.memory_percent >= 5.0 {
                Color32::from_rgb(16, 185, 129)
            } else {
                Color32::from_rgb(156, 163, 175)
            };
            (format!("{:.1}", proc.memory_percent), color)
        }

        SortColumn::Virt => (format_bytes(proc.virtual_memory_mb * 1024.0 * 1024.0), Color32::from_rgb(139, 92, 246)),

        SortColumn::Res => (format_bytes(proc.memory_mb * 1024.0 * 1024.0), Color32::from_rgb(59, 130, 246)),

        SortColumn::Shr => (format_bytes(proc.shared_memory_mb * 1024.0 * 1024.0), Color32::from_rgb(16, 185, 129)),

        SortColumn::Threads => {
            let color = if proc.thread_count == 1 {
                Color32::from_rgb(107, 114, 128)
            } else {
                Color32::from_rgb(156, 163, 175)
            };
            (proc.thread_count.to_string(), color)
        }

        SortColumn::Handles => (proc.handle_count.to_string(), Color32::from_rgb(156, 163, 175)),

        SortColumn::Priority => (proc.priority.to_string(), Color32::from_rgb(156, 163, 175)),

        SortColumn::Time => (format_cpu_time(proc.cpu_time_secs), Color32::from_rgb(245, 158, 11)),

        SortColumn::Status => (proc.status.chars().next().unwrap_or('?').to_string(), Color32::from_rgb(16, 185, 129)),

        SortColumn::IoRead => (format_bytes(proc.io_read_bytes as f64), Color32::from_rgb(96, 165, 250)),

        SortColumn::IoWrite => (format_bytes(proc.io_write_bytes as f64), Color32::from_rgb(251, 146, 60)),
    }
}

fn format_bytes(bytes: f64) -> String {
    if bytes >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1}G", bytes / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024.0 * 1024.0 {
        format!("{:.0}M", bytes / (1024.0 * 1024.0))
    } else if bytes >= 1024.0 {
        format!("{:.0}K", bytes / 1024.0)
    } else {
        format!("{:.0}", bytes)
    }
}

fn format_cpu_time(secs: u64) -> String {
    let hours = secs / 3600;
    let mins = (secs % 3600) / 60;
    let secs = secs % 60;

    if hours > 0 {
        format!("{:02}:{:02}:{:02}", hours, mins, secs)
    } else {
        format!("{:02}:{:02}", mins, secs)
    }
}
