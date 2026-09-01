//! The Processes page.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::consts::PROCESS_PAGE_SIZE;
use crate::app::message::Message;
use crate::app::policy::process_layout_metrics;
use crate::app::screen::ShellEnv;
use crate::app::shell_msg::ShellMsg;
use crate::app::state::Page;
use crate::fixtures::visual::PROCESS_ROWS_258;
use crate::screens::monitor::state::MonitorMsg;
use crate::screens::processes::state::{ProcessesMsg, ProcessesScreen};
use crate::widgets::chrome::{fa_icon_label, page_header, placed};
use crate::widgets::icons;
use crate::widgets::icons::FaIcon;
use crate::widgets::palette_colors::{Palette, palette_track};
use std::sync::Arc;
use wfdiag_app::ports::monitor::{ProcessPage, ProcessRow, ProcessSortDirection, ProcessSortKey};
use wfdiag_native_projection::process_identity::ProcessIdentity;
use windows_reactor::*;

#[derive(Clone, PartialEq)]
pub(crate) struct ProcessViewRow {
    pub(crate) name: String,
    pub(crate) pid: u32,
    pub(crate) start_time: i64,
    pub(crate) cpu: f64,
    pub(crate) memory: String,
    pub(crate) memory_percent: f64,
    pub(crate) virtual_memory: String,
    pub(crate) status: String,
    pub(crate) threads: u32,
    pub(crate) handles: u32,
    pub(crate) cpu_time_secs: u64,
    pub(crate) read: String,
    pub(crate) written: String,
}

impl ProcessViewRow {
    pub(crate) fn identity(&self) -> ProcessIdentity {
        ProcessIdentity::new(self.pid, self.start_time)
    }

    /// The reconciliation key for one row.
    ///
    /// #194: rows used to be keyed by their **slot index**, so a CPU-sorted
    /// page that reorders every two seconds moved every row's contents to a
    /// different key and forced a full re-render. Keying by the process
    /// identity (PID plus start time) lets Reactor move the realized row
    /// instead, and lets an unchanged row be skipped entirely.
    pub(crate) fn row_key(&self) -> String {
        let identity = self.identity();
        format!("process:{}:{}", identity.pid, identity.start_time)
    }

    pub(crate) fn icon(&self) -> FaIcon {
        let name = self.name.to_ascii_lowercase();
        if name.contains("msmpeng") {
            FaIcon::ShieldHalved
        } else if name == "system" {
            FaIcon::Microchip
        } else if name.contains("dwm") {
            FaIcon::Desktop
        } else if name.contains("svchost") || name.contains("workload") {
            FaIcon::Gear
        } else if name.contains("terminal") {
            FaIcon::List
        } else {
            FaIcon::Windows
        }
    }
}

impl From<&ProcessRow> for ProcessViewRow {
    fn from(process: &ProcessRow) -> Self {
        Self {
            name: process.name.clone(),
            pid: process.pid,
            start_time: process.start_time,
            cpu: f64::from(process.cpu_percent),
            memory: format_megabytes(process.memory_mb),
            memory_percent: f64::from(process.memory_percent),
            virtual_memory: format_megabytes(process.virtual_memory_mb),
            status: process.status.clone(),
            threads: process.thread_count,
            handles: process.handle_count,
            cpu_time_secs: process.cpu_time_secs,
            read: format_bytes(process.io_read_bytes),
            written: format_bytes(process.io_write_bytes),
        }
    }
}

pub(crate) fn format_megabytes(value: f64) -> String {
    if !value.is_finite() || value < 0.0 {
        return "—".to_string();
    }
    if value >= 1024.0 {
        format!("{:.2} GB", value / 1024.0)
    } else {
        format!("{value:.1} MB")
    }
}

pub(crate) fn format_bytes(value: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = 1024.0 * KIB;
    const GIB: f64 = 1024.0 * MIB;
    let value = value as f64;
    if value >= GIB {
        format!("{:.2} GB", value / GIB)
    } else if value >= MIB {
        format!("{:.1} MB", value / MIB)
    } else if value >= KIB {
        format!("{:.1} KB", value / KIB)
    } else {
        format!("{} B", value as u64)
    }
}

#[derive(Clone)]
pub(crate) struct ProcessRowInput {
    pub(crate) palette: Palette,
    pub(crate) narrow: bool,
    pub(crate) row_width: f64,
    pub(crate) process: Arc<ProcessViewRow>,
    pub(crate) selected: bool,
    pub(crate) select_process: Callback<Option<ProcessIdentity>>,
}

/// #194: the live tick re-queries the same page twice a second, so most rows
/// arrive byte-identical. The screen keeps the previous `Arc` for those, which
/// makes the row comparison a pointer test instead of thirteen field tests —
/// and, more importantly, makes Reactor skip the realized row altogether.
impl PartialEq for ProcessRowInput {
    fn eq(&self, other: &Self) -> bool {
        let same_row = Arc::ptr_eq(&self.process, &other.process) || self.process == other.process;
        same_row
            && self.palette == other.palette
            && self.narrow == other.narrow
            && self.row_width == other.row_width
            && self.selected == other.selected
            && self.select_process == other.select_process
    }
}

pub(crate) struct ProcessRowComponent;

impl Component for ProcessRowComponent {
    type Input = ProcessRowInput;
    type Message = ();

    fn create(_input: &Self::Input, _context: &ComponentContext<Self>) -> Self {
        Self
    }

    fn view(&self, input: &Self::Input, _context: &mut ViewContext<Self>) -> View {
        process_row_258(
            input.palette,
            input.narrow,
            input.row_width,
            input.process.as_ref(),
            input.selected,
            input.select_process.clone(),
        )
    }
}

impl ProcessesScreen {
    /// Paint the page from the screen's own state plus the chrome's env.
    pub(crate) fn view(&self, env: &ShellEnv<'_>, vc: &mut ViewContext<WfdiagShell>) -> View {
        processes_page(
            env.palette,
            env.window_size.width,
            env.pane_expanded,
            &self.filter,
            self.page.as_ref(),
            &self.rows,
            self.loading,
            self.error.as_deref(),
            self.sort_key,
            self.sort_direction,
            env.deterministic_visual,
            self.selected,
            env.monitoring_paused,
            vc.callback(|value| Message::Processes(ProcessesMsg::FilterChanged(value))),
            vc.callback(|value| Message::Processes(ProcessesMsg::Sort(value))),
            vc.message(Message::Processes(ProcessesMsg::Previous)),
            vc.message(Message::Processes(ProcessesMsg::Next)),
            vc.callback(|value| Message::Processes(ProcessesMsg::Select(value))),
            vc.message(Message::Monitor(MonitorMsg::ToggleMonitoring)),
            vc.message(Message::Shell(ShellMsg::Refresh)),
        )
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn processes_page(
    palette: Palette,
    window_width: f64,
    pane_expanded: bool,
    filter: &str,
    process_page: Option<&ProcessPage>,
    live_rows: &[Arc<ProcessViewRow>],
    loading: bool,
    error: Option<&str>,
    sort_key: ProcessSortKey,
    sort_direction: ProcessSortDirection,
    deterministic_visual: bool,
    selected_identity: Option<ProcessIdentity>,
    paused: bool,
    filter_changed: Callback<String>,
    sort_processes: Callback<ProcessSortKey>,
    previous: Callback<()>,
    next: Callback<()>,
    select_process: Callback<Option<ProcessIdentity>>,
    toggle: Callback<()>,
    refresh: Callback<()>,
) -> View {
    // ItemsRepeater's default StackLayout measures each realized child at its
    // desired width instead of stretching it to the table. Keep every row on
    // the header's exact column boundaries and switch the details pane below
    // the table before that pane would squeeze the process-name column away.
    let (narrow, row_width) = process_layout_metrics(window_width, pane_expanded);
    let needle = filter.trim().to_ascii_lowercase();
    let (display_rows, total, offset, limit) = if deterministic_visual {
        let rows = PROCESS_ROWS_258
            .into_iter()
            .filter(|process| {
                needle.is_empty()
                    || process.name.to_ascii_lowercase().contains(&needle)
                    || process.pid.to_string().contains(&needle)
                    || process.status.to_ascii_lowercase().contains(&needle)
            })
            .map(|process| Arc::new(ProcessViewRow::from(process)))
            .collect::<Vec<_>>();
        let total = if needle.is_empty() { 450 } else { rows.len() };
        (rows, total, 0, PROCESS_PAGE_SIZE)
    } else if let Some(page) = process_page {
        (live_rows.to_vec(), page.total, page.offset, page.limit)
    } else {
        (Vec::new(), 0, 0, PROCESS_PAGE_SIZE)
    };
    let visible = display_rows.len().min(PROCESS_PAGE_SIZE);
    let selected = selected_identity.and_then(|identity| {
        display_rows
            .iter()
            .take(visible)
            .find(|process| identity.matches_observation(process.identity()))
            .map(Arc::clone)
    });
    // #194: rows are keyed by process identity, not by slot index. A
    // CPU-sorted page reorders on every live tick; with positional keys that
    // moved every row's contents into a different slot and re-rendered the
    // whole table twice a second. With identity keys Reactor moves the
    // realized row, and an unchanged row compares equal and is skipped.
    let rows = display_rows
        .iter()
        .take(visible)
        .map(|process| {
            KeyedView::new(
                process.row_key(),
                View::component::<ProcessRowComponent>(ProcessRowInput {
                    palette,
                    narrow,
                    row_width,
                    process: Arc::clone(process),
                    selected: selected_identity
                        .is_some_and(|identity| identity.matches_observation(process.identity())),
                    select_process: select_process.clone(),
                }),
            )
        })
        .collect::<Vec<_>>();

    let actions = StackPanel::new()
        .orientation(Orientation::Horizontal)
        .spacing(8.0)
        .children((
            Button::new()
                .on_click(refresh.clone())
                .automation_name("Refresh processes")
                .content(fa_icon_label(FaIcon::Refresh, "Refresh")),
            Button::new().on_click(toggle).content(fa_icon_label(
                if paused { FaIcon::Play } else { FaIcon::Pause },
                if paused { "Resume" } else { "Pause" },
            )),
        ));
    let start = if total == 0 { 0 } else { offset + 1 };
    let end = if deterministic_visual && needle.is_empty() {
        (offset + limit).min(total)
    } else {
        offset.saturating_add(visible).min(total)
    };
    let mut summary = format!("Showing {start}–{end} of {total} processes");
    if loading && process_page.is_some() && !deterministic_visual {
        summary.push_str(" · Refreshing…");
    }

    let toolbar: View = if narrow {
        StackPanel::new().spacing(8.0).children((
            TextBox::new()
                .height(32.0)
                .text(filter)
                .placeholder_text("Filter processes…")
                .on_text_changed(filter_changed),
            Grid::new()
                .columns([GridLength::Star(1.0), GridLength::Auto])
                .children((
                    TextBlock::new()
                        .text(summary)
                        .font_size(11.5)
                        .foreground(palette.muted)
                        .vertical_alignment(VerticalAlignment::Center),
                    Border::new().grid_column(1).content(actions),
                )),
        ))
    } else {
        Grid::new()
            .columns([GridLength::Star(1.0), GridLength::Auto])
            .children((
                StackPanel::new()
                    .orientation(Orientation::Horizontal)
                    .spacing(12.0)
                    .children((
                        TextBox::new()
                            .width(260.0)
                            .height(32.0)
                            .text(filter)
                            .placeholder_text("Filter processes…")
                            .on_text_changed(filter_changed),
                        TextBlock::new()
                            .text(summary)
                            .font_size(11.5)
                            .foreground(palette.muted)
                            .vertical_alignment(VerticalAlignment::Center),
                    )),
                Border::new().grid_column(1).content(actions),
            ))
    };

    let empty_message: View = if visible == 0 {
        let message = if let Some(error) = error {
            format!("Could not load processes: {error}")
        } else if loading && !deterministic_visual {
            "Loading processes…".to_string()
        } else if !filter.trim().is_empty() {
            format!("No processes match “{}”.", filter.trim())
        } else {
            "No running processes were returned.".to_string()
        };
        Border::new().height(180.0).content(
            TextBlock::new()
                .text(message)
                .font_size(12.0)
                .foreground(palette.muted)
                .horizontal_alignment(HorizontalAlignment::Center)
                .vertical_alignment(VerticalAlignment::Center),
        )
    } else {
        View::empty()
    };
    let rows_view = Grid::new().children((
        ItemsRepeater::new()
            .horizontal_alignment(HorizontalAlignment::Stretch)
            .items(rows),
        empty_message,
    ));

    let table = Border::new()
        .background(palette.card)
        .border_brush(palette.border)
        .border_thickness(1.0)
        .corner_radius(9.0)
        .content(StackPanel::new().children((
            process_header_258(palette, narrow, sort_key, sort_direction, sort_processes),
            rows_view,
            process_pagination_258(
                palette,
                start,
                end,
                total,
                offset > 0 && !loading,
                end < total && !loading,
                previous,
                next,
            ),
        )));
    let detail = selected
        .as_ref()
        .map(|process| process_details_258(palette, process.as_ref(), select_process))
        .unwrap_or_else(View::empty);
    let layout: View = if narrow {
        StackPanel::new().spacing(12.0).children((table, detail))
    } else {
        Grid::new()
            .columns([GridLength::Star(1.0), GridLength::Pixel(300.0)])
            .column_spacing(12.0)
            .children((table, placed(detail, 1, 0)))
    };
    let stale_error: View = if visible > 0 {
        error.map_or_else(View::empty, |error| {
            Border::new()
                .padding(Thickness::new(12.0, 9.0, 12.0, 9.0))
                .background(palette.err_bg)
                .border_brush(palette.err)
                .border_thickness(1.0)
                .corner_radius(7.0)
                .content(
                    Grid::new()
                        .columns([GridLength::Star(1.0), GridLength::Auto])
                        .column_spacing(10.0)
                        .children((
                            TextBlock::new()
                                .text(format!(
                                    "Process refresh failed · {error} · showing the last successful page"
                                ))
                                .font_size(11.5)
                                .foreground(palette.err)
                                .text_wrapping(TextWrapping::Wrap)
                                .vertical_alignment(VerticalAlignment::Center),
                            Button::new()
                                .grid_column(1)
                                .height(30.0)
                                .is_enabled(!loading)
                                .on_click(refresh)
                                .content("Retry"),
                        )),
                )
        })
    } else {
        View::empty()
    };

    StackPanel::new().spacing(16.0).children((
        page_header(palette, Page::Processes, View::empty()),
        Border::new()
            .margin(Thickness::new(0.0, 1.0, 0.0, 0.0))
            .content(toolbar),
        stale_error,
        Border::new()
            .margin(Thickness::new(0.0, -4.0, 0.0, 0.0))
            .content(layout),
    ))
}

pub(crate) fn process_columns_258(narrow: bool) -> [GridLength; 6] {
    if narrow {
        // Keep the compact fixed-width total at 434 DIP so the process-name
        // column and explicit row width are unchanged at the 720-DIP minimum.
        // The smaller meter frees enough room for complete PID, memory, and
        // thread values without sacrificing the status column.
        [
            GridLength::Star(1.0),
            GridLength::Pixel(60.0),
            GridLength::Pixel(88.0),
            GridLength::Pixel(142.0),
            GridLength::Pixel(78.0),
            GridLength::Pixel(66.0),
        ]
    } else {
        [
            GridLength::Star(1.0),
            GridLength::Pixel(70.0),
            GridLength::Pixel(140.0),
            GridLength::Pixel(150.0),
            GridLength::Pixel(84.0),
            GridLength::Pixel(110.0),
        ]
    }
}

pub(crate) fn process_header_horizontal_padding_258(narrow: bool) -> f64 {
    if narrow { 6.0 } else { 18.0 }
}

pub(crate) fn process_cell_horizontal_margin_258(narrow: bool) -> f64 {
    if narrow { 8.0 } else { 18.0 }
}

pub(crate) fn process_name_horizontal_margin_258(narrow: bool) -> f64 {
    if narrow { 12.0 } else { 18.0 }
}

pub(crate) fn process_meter_width_258(narrow: bool) -> f64 {
    if narrow { 44.0 } else { 56.0 }
}

pub(crate) fn process_header_258(
    palette: Palette,
    narrow: bool,
    sort_key: ProcessSortKey,
    sort_direction: ProcessSortDirection,
    sort_processes: Callback<ProcessSortKey>,
) -> View {
    Grid::new()
        .height(37.0)
        .columns(process_columns_258(narrow))
        .background(palette.card_strong)
        .children((
            process_header_cell_258(
                palette,
                narrow,
                "PROCESS",
                0,
                ProcessSortKey::Name,
                sort_key,
                sort_direction,
                sort_processes.clone(),
            ),
            process_header_cell_258(
                palette,
                narrow,
                "PID",
                1,
                ProcessSortKey::Pid,
                sort_key,
                sort_direction,
                sort_processes.clone(),
            ),
            process_header_cell_258(
                palette,
                narrow,
                "CPU",
                2,
                ProcessSortKey::CpuPercent,
                sort_key,
                sort_direction,
                sort_processes.clone(),
            ),
            process_header_cell_258(
                palette,
                narrow,
                "MEMORY",
                3,
                ProcessSortKey::MemoryMb,
                sort_key,
                sort_direction,
                sort_processes.clone(),
            ),
            process_header_cell_258(
                palette,
                narrow,
                "STATUS",
                4,
                ProcessSortKey::Status,
                sort_key,
                sort_direction,
                sort_processes.clone(),
            ),
            process_header_cell_258(
                palette,
                narrow,
                "THREADS",
                5,
                ProcessSortKey::ThreadCount,
                sort_key,
                sort_direction,
                sort_processes,
            ),
        ))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn process_header_cell_258(
    palette: Palette,
    narrow: bool,
    label: &'static str,
    column: i32,
    column_key: ProcessSortKey,
    active_key: ProcessSortKey,
    direction: ProcessSortDirection,
    sort_processes: Callback<ProcessSortKey>,
) -> View {
    let active = column_key == active_key;
    let arrow: View = if active {
        TextBlock::new()
            .text(match direction {
                ProcessSortDirection::Asc => "↑",
                ProcessSortDirection::Desc => "↓",
            })
            .font_size(10.5)
            .foreground(palette.accent)
            .into()
    } else {
        View::empty()
    };
    Button::new()
        .grid_column(column)
        .height(37.0)
        .horizontal_alignment(HorizontalAlignment::Stretch)
        .horizontal_content_alignment(HorizontalAlignment::Stretch)
        .resource_overrides(
            ResourceOverrides::new()
                .set("ButtonBackground", Color::transparent())
                .set("ButtonBackgroundPointerOver", palette.active)
                .set("ButtonBackgroundPressed", palette.active)
                .set("ButtonForeground", palette.muted)
                .set("ButtonBorderThemeThickness", Thickness::uniform(0.0))
                .set(
                    "ButtonPadding",
                    Thickness::xy(process_header_horizontal_padding_258(narrow), 0.0),
                )
                .set("ControlCornerRadius", CornerRadius::uniform(0.0)),
        )
        .automation_name(format!("Sort processes by {label}"))
        .on_click(move || {
            let _ = sort_processes.call(column_key);
        })
        .content(
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(4.0)
                .vertical_alignment(VerticalAlignment::Center)
                .children((
                    TextBlock::new()
                        .text(label)
                        .font_size(10.5)
                        .font_weight(FontWeight::SEMI_BOLD)
                        .foreground(palette.muted),
                    arrow,
                )),
        )
}

pub(crate) fn process_row_258(
    palette: Palette,
    narrow: bool,
    row_width: f64,
    process: &ProcessViewRow,
    selected: bool,
    select_process: Callback<Option<ProcessIdentity>>,
) -> View {
    let select = select_process.clone();
    let identity = process.identity();
    let row = Grid::new()
        .height(37.0)
        .horizontal_alignment(HorizontalAlignment::Stretch)
        .columns(process_columns_258(narrow))
        .children((
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(9.0)
                .margin(Thickness::xy(
                    process_name_horizontal_margin_258(narrow),
                    0.0,
                ))
                .vertical_alignment(VerticalAlignment::Center)
                .children((
                    icons::path(process.icon()),
                    TextBlock::new()
                        .text(process.name.clone())
                        .font_size(12.0)
                        .foreground(palette.text)
                        .text_trimming(TextTrimming::CharacterEllipsis),
                )),
            process_table_cell_258(palette, narrow, process.pid.to_string(), 1),
            process_cpu_cell_258(palette, narrow, process.cpu, 2),
            process_memory_cell_258(palette, narrow, process, 3),
            process_table_cell_258(palette, narrow, process.status.clone(), 4),
            process_table_cell_258(palette, narrow, process.threads.to_string(), 5),
        ));

    Border::new()
        .width(row_width)
        .height(37.0)
        .horizontal_alignment(HorizontalAlignment::Stretch)
        .background(if selected {
            palette.active
        } else {
            Color::transparent()
        })
        .border_brush(palette.border)
        .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
        .content(
            Button::new()
                .height(37.0)
                .horizontal_alignment(HorizontalAlignment::Stretch)
                .horizontal_content_alignment(HorizontalAlignment::Stretch)
                .resource_overrides(
                    ResourceOverrides::new()
                        .set("ButtonBackground", Color::transparent())
                        .set("ButtonBackgroundPointerOver", palette.active)
                        .set("ButtonBackgroundPressed", palette.active)
                        .set("ButtonForeground", palette.text)
                        .set("ButtonBorderThemeThickness", Thickness::uniform(0.0))
                        .set("ButtonPadding", Thickness::uniform(0.0))
                        .set("ControlCornerRadius", CornerRadius::uniform(0.0)),
                )
                .automation_name(format!("{} PID {}", process.name, process.pid))
                .on_click(move || {
                    let _ = select.call(Some(identity));
                })
                .content(row),
        )
}

pub(crate) fn process_table_cell_258(
    palette: Palette,
    narrow: bool,
    text: impl Into<String>,
    column: i32,
) -> TextBlock {
    TextBlock::new()
        .text(text)
        .grid_column(column)
        .margin(Thickness::xy(
            process_cell_horizontal_margin_258(narrow),
            0.0,
        ))
        .font_size(11.5)
        .foreground(palette.muted)
        .vertical_alignment(VerticalAlignment::Center)
}

pub(crate) fn process_percent_stack_258(palette: Palette, narrow: bool, percent: f64) -> View {
    let meter_width = process_meter_width_258(narrow);
    StackPanel::new().spacing(3.0).children((
        TextBlock::new()
            .text(format!("{percent:.1}%"))
            .font_size(11.5)
            .foreground(palette.muted)
            .horizontal_alignment(HorizontalAlignment::Right),
        Border::new()
            .width(meter_width)
            .height(4.0)
            .background(palette_track())
            .corner_radius(999.0)
            .horizontal_alignment(HorizontalAlignment::Left)
            .content(
                Border::new()
                    .width((percent.clamp(0.0, 100.0) * meter_width / 100.0).max(1.0))
                    .height(4.0)
                    .background(if percent > 80.0 {
                        palette.err
                    } else if percent > 50.0 {
                        palette.warn
                    } else {
                        palette.accent
                    })
                    .corner_radius(999.0)
                    .horizontal_alignment(HorizontalAlignment::Left),
            ),
    ))
}

pub(crate) fn process_cpu_cell_258(
    palette: Palette,
    narrow: bool,
    percent: f64,
    column: i32,
) -> View {
    Border::new()
        .grid_column(column)
        .margin(Thickness::xy(
            process_cell_horizontal_margin_258(narrow),
            3.0,
        ))
        .content(process_percent_stack_258(palette, narrow, percent))
}

pub(crate) fn process_memory_cell_258(
    palette: Palette,
    narrow: bool,
    process: &ProcessViewRow,
    column: i32,
) -> View {
    let meter_width = process_meter_width_258(narrow);
    Grid::new()
        .grid_column(column)
        .margin(Thickness::xy(
            process_cell_horizontal_margin_258(narrow),
            0.0,
        ))
        .columns([GridLength::Star(1.0), GridLength::Pixel(meter_width)])
        .column_spacing(8.0)
        .children((
            TextBlock::new()
                .text(process.memory.clone())
                .font_size(11.5)
                .foreground(palette.muted)
                .horizontal_alignment(HorizontalAlignment::Right)
                .vertical_alignment(VerticalAlignment::Center),
            Border::new()
                .grid_column(1)
                .margin(Thickness::new(0.0, 3.0, 0.0, 3.0))
                .content(process_percent_stack_258(
                    palette,
                    narrow,
                    process.memory_percent,
                )),
        ))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn process_pagination_258(
    palette: Palette,
    start: usize,
    end: usize,
    total: usize,
    can_previous: bool,
    can_next: bool,
    previous: Callback<()>,
    next: Callback<()>,
) -> View {
    let range = format!("{start}–{end} of {total}");
    Border::new().height(45.0).content(
        Grid::new()
            .columns([
                GridLength::Star(1.0),
                GridLength::Auto,
                GridLength::Pixel(94.0),
                GridLength::Auto,
            ])
            .column_spacing(9.0)
            .children((
                Button::new()
                    .grid_column(1)
                    .height(30.0)
                    .is_enabled(can_previous)
                    .on_click(previous)
                    .vertical_alignment(VerticalAlignment::Center)
                    .content("Previous"),
                TextBlock::new()
                    .text(range)
                    .grid_column(2)
                    .font_size(11.5)
                    .foreground(palette.muted)
                    .horizontal_alignment(HorizontalAlignment::Center)
                    .vertical_alignment(VerticalAlignment::Center),
                Button::new()
                    .grid_column(3)
                    .height(30.0)
                    .margin(Thickness::new(0.0, 0.0, 12.0, 0.0))
                    .is_enabled(can_next)
                    .on_click(next)
                    .vertical_alignment(VerticalAlignment::Center)
                    .content("Next"),
            )),
    )
}

pub(crate) fn process_details_258(
    palette: Palette,
    process: &ProcessViewRow,
    select_process: Callback<Option<ProcessIdentity>>,
) -> View {
    let close = select_process.clone();
    Border::new()
        .background(palette.card)
        .border_brush(palette.border)
        .border_thickness(1.0)
        .corner_radius(9.0)
        .content(StackPanel::new().children((
            Border::new()
                .padding(Thickness::new(15.0, 13.0, 10.0, 13.0))
                .border_brush(palette.border)
                .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
                .content(
                    Grid::new()
                        .columns([GridLength::Star(1.0), GridLength::Auto])
                        .children((
                            StackPanel::new().spacing(2.0).children((
                                TextBlock::new()
                                    .text(process.name.clone())
                                    .font_size(13.0)
                                    .font_weight(FontWeight::SEMI_BOLD)
                                    .text_trimming(TextTrimming::CharacterEllipsis),
                                TextBlock::new()
                                    .text(format!("PID {}", process.pid))
                                    .font_size(11.0)
                                    .foreground(palette.muted),
                            )),
                            Button::new()
                                .grid_column(1)
                                .width(28.0)
                                .height(28.0)
                                .resource_overrides(
                                    ResourceOverrides::new()
                                        .set("ButtonBackground", Color::transparent())
                                        .set("ButtonBackgroundPointerOver", palette.active)
                                        .set("ButtonBackgroundPressed", palette.active)
                                        .set("ButtonForeground", palette.muted)
                                        .set(
                                            "ButtonBorderThemeThickness",
                                            Thickness::uniform(0.0),
                                        )
                                        .set("ButtonPadding", Thickness::uniform(6.0))
                                        .set(
                                            "ControlCornerRadius",
                                            CornerRadius::uniform(5.0),
                                        ),
                                )
                                .automation_name("Close process details")
                                .on_click(move || {
                                    let _ = close.call(None);
                                })
                                .content(icons::path(FaIcon::Xmark)),
                        )),
                ),
            Border::new()
                .padding(Thickness::new(15.0, 7.0, 15.0, 7.0))
                .content(StackPanel::new().children((
                    process_detail_row_258(palette, "CPU", format!("{:.1}%", process.cpu)),
                    process_detail_row_258(
                        palette,
                        "Memory",
                        format!("{} ({:.1}%)", process.memory, process.memory_percent),
                    ),
                    process_detail_row_258(
                        palette,
                        "Virtual memory",
                        process.virtual_memory.clone(),
                    ),
                    process_detail_row_258(palette, "Threads", process.threads.to_string()),
                    process_detail_row_258(palette, "Handles", process.handles.to_string()),
                    process_detail_row_258(
                        palette,
                        "CPU time",
                        format!("{}s", process.cpu_time_secs),
                    ),
                    process_detail_row_258(palette, "Read", process.read.clone()),
                    process_detail_row_258(palette, "Written", process.written.clone()),
                ))),
            Border::new()
                .padding(Thickness::new(15.0, 10.0, 15.0, 14.0))
                .content(
                    TextBlock::new()
                        .text("Path, owner, architecture, and elevation are omitted when Windows does not expose them without an additional privileged query.")
                        .font_size(10.5)
                        .foreground(palette.muted)
                        .text_wrapping(TextWrapping::Wrap),
                ),
        )))
}

pub(crate) fn process_detail_row_258(
    palette: Palette,
    label: impl Into<String>,
    value: impl Into<String>,
) -> View {
    Border::new()
        .min_height(32.0)
        .border_brush(palette.border)
        .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
        .content(
            Grid::new()
                .columns([GridLength::Star(1.0), GridLength::Star(1.25)])
                .column_spacing(10.0)
                .children((
                    TextBlock::new()
                        .text(label)
                        .font_size(11.5)
                        .foreground(palette.muted)
                        .vertical_alignment(VerticalAlignment::Center),
                    TextBlock::new()
                        .text(value)
                        .grid_column(1)
                        .font_size(11.5)
                        .horizontal_alignment(HorizontalAlignment::Right)
                        .text_wrapping(TextWrapping::Wrap)
                        .vertical_alignment(VerticalAlignment::Center),
                )),
        )
}
