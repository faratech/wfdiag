//! The Monitor page.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::message::Message;
use crate::app::screen::ShellEnv;
use crate::app::shell_msg::ShellMsg;
use crate::app::state::{MonitorHistory, MonitorMetric, Page};
use crate::screens::monitor::state::{MonitorMsg, MonitorScreen};
use crate::widgets::cards::metric_card;
use crate::widgets::chrome::{fa_icon_label, page_header};
use crate::widgets::icons;
use crate::widgets::icons::FaIcon;
use crate::widgets::palette_colors::Palette;
use wfdiag_app::ports::monitor::NetworkConnection;
use wfdiag_native_projection::render::{
    MONITOR_GRAPH_HEIGHT, MONITOR_GRAPH_PATH_COUNT, MONITOR_GRAPH_WIDTH, monitor_graph_geometry,
};
use wfdiag_ui_core::SystemStats;
use windows_reactor::*;

pub(crate) fn monitor_graph(palette: Palette, series: &[f64], max: f64) -> View {
    let geometry = monitor_graph_geometry(series, max);
    let area_opacity = if palette.accent.r == 77 {
        36.0 / 255.0
    } else {
        31.0 / 255.0
    };
    let graph_paths: [PathIcon; MONITOR_GRAPH_PATH_COUNT] = [
        PathIcon::new()
            .data(geometry.area)
            .width(MONITOR_GRAPH_WIDTH)
            .height(MONITOR_GRAPH_HEIGHT)
            .opacity(area_opacity),
        PathIcon::new()
            .data(geometry.ribbon)
            .width(MONITOR_GRAPH_WIDTH)
            .height(MONITOR_GRAPH_HEIGHT),
    ];

    Viewbox::new()
        .height(62.0)
        .margin(Thickness::new(0.0, 12.0, 0.0, 0.0))
        .stretch(Stretch::Fill)
        .slot(
            ViewboxSlot::Child,
            Button::new()
                .width(MONITOR_GRAPH_WIDTH)
                .height(MONITOR_GRAPH_HEIGHT)
                .is_enabled(false)
                .horizontal_content_alignment(HorizontalAlignment::Stretch)
                .vertical_content_alignment(VerticalAlignment::Stretch)
                .resource_overrides(
                    ResourceOverrides::new()
                        .set("ButtonBackground", Color::transparent())
                        .set("ButtonBackgroundDisabled", Color::transparent())
                        .set("ButtonForeground", palette.accent)
                        .set("ButtonForegroundDisabled", palette.accent)
                        .set("ButtonBorderThemeThickness", Thickness::uniform(0.0))
                        .set("ButtonPadding", Thickness::uniform(0.0))
                        .set("ControlCornerRadius", CornerRadius::uniform(0.0)),
                )
                .content(
                    Canvas::new()
                        .width(MONITOR_GRAPH_WIDTH)
                        .height(MONITOR_GRAPH_HEIGHT)
                        .children(graph_paths),
                ),
        )
}

pub(crate) fn monitor_axis_label(
    palette: Palette,
    label: &'static str,
    column: i32,
    alignment: HorizontalAlignment,
) -> View {
    TextBlock::new()
        .text(label)
        .grid_column(column)
        .font_size(9.5)
        .foreground(palette.muted)
        .opacity(0.7)
        .horizontal_alignment(alignment)
        .into()
}

pub(crate) fn monitor_axis(palette: Palette) -> View {
    Grid::new()
        .margin(Thickness::new(0.0, 5.0, 0.0, 0.0))
        .columns([
            GridLength::Auto,
            GridLength::Star(1.0),
            GridLength::Auto,
            GridLength::Star(1.0),
            GridLength::Auto,
            GridLength::Star(1.0),
            GridLength::Auto,
            GridLength::Star(1.0),
            GridLength::Auto,
        ])
        .children((
            monitor_axis_label(palette, "-60s", 0, HorizontalAlignment::Left),
            monitor_axis_label(palette, "-45", 2, HorizontalAlignment::Center),
            monitor_axis_label(palette, "-30", 4, HorizontalAlignment::Center),
            monitor_axis_label(palette, "-15", 6, HorizontalAlignment::Center),
            monitor_axis_label(palette, "now", 8, HorizontalAlignment::Right),
        ))
}

pub(crate) fn monitor_action_button(
    palette: Palette,
    icon: FaIcon,
    label: &'static str,
    width: f64,
    action: Callback<()>,
) -> View {
    Button::new()
        .width(width)
        .height(32.0)
        .resource_overrides(
            ResourceOverrides::new()
                .set("ButtonBackground", palette.card)
                .set("ButtonBackgroundPointerOver", palette.card)
                .set("ButtonBackgroundPressed", palette.active)
                .set("ButtonBorderBrush", palette.border)
                .set("ButtonBorderBrushPointerOver", palette.border)
                .set("ButtonBorderThemeThickness", Thickness::uniform(1.0))
                .set("ButtonPadding", Thickness::xy(15.0, 0.0))
                .set("ControlCornerRadius", CornerRadius::uniform(7.0)),
        )
        .on_click(action)
        .automation_name(label)
        .content(
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(8.0)
                .children((
                    icons::path(icon).width(12.0).height(12.0),
                    TextBlock::new()
                        .text(label)
                        .font_size(13.0)
                        .font_weight(FontWeight::SEMI_BOLD),
                )),
        )
}

pub(crate) fn monitor_status_pill(palette: Palette, paused: bool) -> View {
    let foreground = if paused { palette.warn } else { palette.ok };
    Border::new()
        .height(22.0)
        .background(if paused {
            palette.warn_bg
        } else {
            palette.ok_bg
        })
        .corner_radius(999.0)
        .padding(Thickness::new(12.0, 0.0, 8.0, 0.0))
        .vertical_alignment(VerticalAlignment::Center)
        .content(
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(7.0)
                .vertical_alignment(VerticalAlignment::Center)
                .children((
                    Ellipse::new()
                        .width(7.0)
                        .height(7.0)
                        .fill(foreground)
                        .vertical_alignment(VerticalAlignment::Center),
                    TextBlock::new()
                        .text(if paused { "Paused" } else { "Live · sampling" })
                        .foreground(foreground)
                        .font_size(11.5)
                        .font_weight(FontWeight::SEMI_BOLD)
                        .vertical_alignment(VerticalAlignment::Center),
                )),
        )
}

impl MonitorScreen {
    /// Paint the page from the screen's own state plus the chrome's env.
    pub(crate) fn view(&self, env: &ShellEnv<'_>, vc: &mut ViewContext<WfdiagShell>) -> View {
        monitor_page(
            env.palette,
            env.narrow,
            self.paused,
            self.error.as_deref(),
            self.stats.as_ref(),
            &self.history,
            vc.message(Message::Monitor(MonitorMsg::ToggleMonitoring)),
            vc.message(Message::Shell(ShellMsg::Refresh)),
            self.network_connections.as_deref(),
            self.network_loading,
            vc.message(Message::Monitor(MonitorMsg::RequestNetworkConnections)),
        )
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn monitor_page(
    palette: Palette,
    narrow: bool,
    paused: bool,
    error: Option<&str>,
    stats: Option<&SystemStats>,
    history: &MonitorHistory,
    toggle: Callback<()>,
    refresh: Callback<()>,
    connections: Option<&[NetworkConnection]>,
    connections_loading: bool,
    load_connections: Callback<()>,
) -> View {
    let actions = StackPanel::new()
        .orientation(Orientation::Horizontal)
        .spacing(14.0)
        .margin(Thickness::new(0.0, 1.0, 0.0, -1.0))
        .children((
            monitor_action_button(
                palette,
                if paused { FaIcon::Play } else { FaIcon::Pause },
                if paused { "Resume" } else { "Pause" },
                88.0,
                toggle,
            ),
            monitor_action_button(palette, FaIcon::Refresh, "Refresh", 96.0, refresh.clone()),
        ));
    let (
        cpu_hint,
        cpu_value,
        memory_hint,
        memory_value,
        storage_hint,
        storage_value,
        network_hint,
        network_value,
        gpu_hint,
        gpu_value,
        npu_hint,
        npu_value,
        hardware_summary,
        show_gpu,
        show_npu,
    ) = if let Some(stats) = stats {
        let network_mb = (stats.network_upload_kb + stats.network_download_kb) / 1024.0;
        let gpu_percent = f64::from(stats.gpu_utilization.unwrap_or_default());
        let npu_percent = f64::from(stats.npu_utilization.unwrap_or_default());
        let gpu_hint = if stats.gpu_available && stats.gpu_memory_total_mb > 0.0 {
            format!(
                "{:.2} GB / {:.2} GB",
                stats.gpu_memory_used_mb / 1024.0,
                stats.gpu_memory_total_mb / 1024.0
            )
        } else {
            stats
                .gpu_name
                .clone()
                .unwrap_or_else(|| "Graphics adapter".to_string())
        };
        let npu_hint = if stats.npu_available && stats.npu_memory_total_mb > 0.0 {
            format!(
                "{:.2} GB / {:.2} GB",
                stats.npu_memory_used_mb / 1024.0,
                stats.npu_memory_total_mb / 1024.0
            )
        } else {
            stats
                .npu_name
                .clone()
                .unwrap_or_else(|| "Neural processing unit".to_string())
        };
        let mut hardware_summary = format!(
            "{} threads · {:.1} GB RAM",
            stats.per_cpu_utilization.len(),
            stats.memory_total_gb
        );
        if stats.gpu_available {
            hardware_summary.push_str(" · GPU: ");
            hardware_summary.push_str(stats.gpu_name.as_deref().unwrap_or("present"));
        }
        if stats.npu_available {
            hardware_summary.push_str(" · NPU: ");
            hardware_summary.push_str(stats.npu_name.as_deref().unwrap_or("present"));
        }
        (
            format!("{:.2} GHz", stats.cpu_frequency as f64 / 1000.0),
            format!("{:.1}", stats.cpu_utilization),
            format!(
                "{:.1} / {:.1} GB used",
                stats.memory_used_gb, stats.memory_total_gb
            ),
            format!("{:.1}", stats.memory_utilization),
            "Provisioned storage capacity used".to_string(),
            format!("{:.1}", stats.storage_used_percent),
            "Up + down throughput".to_string(),
            format!("{network_mb:.2}"),
            gpu_hint,
            if stats.gpu_available {
                format!("{gpu_percent:.1}")
            } else {
                "—".to_string()
            },
            npu_hint,
            if stats.npu_available {
                format!("{npu_percent:.1}")
            } else {
                "—".to_string()
            },
            hardware_summary,
            stats.gpu_available,
            stats.npu_available,
        )
    } else {
        (
            "Waiting for sample".to_string(),
            "—".to_string(),
            "Waiting for sample".to_string(),
            "—".to_string(),
            "Provisioned storage capacity used".to_string(),
            "—".to_string(),
            "Up + down throughput".to_string(),
            "—".to_string(),
            "Waiting for sample".to_string(),
            "—".to_string(),
            "Waiting for sample".to_string(),
            "—".to_string(),
            "Waiting for the first native telemetry sample…".to_string(),
            true,
            true,
        )
    };

    let cpu_series = history.series(MonitorMetric::Cpu);
    let memory_series = history.series(MonitorMetric::Memory);
    let storage_series = history.series(MonitorMetric::Storage);
    let network_series = history.series(MonitorMetric::Network);
    let gpu_series = history.series(MonitorMetric::Gpu);
    let npu_series = history.series(MonitorMetric::Npu);
    let network_max = network_series
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .fold(2.0_f64, f64::max)
        * 1.2;

    let mut cards = vec![
        (
            "cpu",
            metric_card(
                palette,
                "CPU",
                &cpu_hint,
                &cpu_value,
                "%",
                &cpu_series,
                100.0,
            ),
        ),
        (
            "memory",
            metric_card(
                palette,
                "MEMORY",
                &memory_hint,
                &memory_value,
                "%",
                &memory_series,
                100.0,
            ),
        ),
        (
            "storage",
            metric_card(
                palette,
                "STORAGE",
                &storage_hint,
                &storage_value,
                "%",
                &storage_series,
                100.0,
            ),
        ),
        (
            "network",
            metric_card(
                palette,
                "NETWORK",
                &network_hint,
                &network_value,
                "MB/s",
                &network_series,
                network_max,
            ),
        ),
    ];
    if show_gpu {
        cards.push((
            "gpu",
            metric_card(
                palette,
                "GPU",
                &gpu_hint,
                &gpu_value,
                "%",
                &gpu_series,
                100.0,
            ),
        ));
    }
    if show_npu {
        cards.push((
            "npu",
            metric_card(
                palette,
                "NPU",
                &npu_hint,
                &npu_value,
                "%",
                &npu_series,
                100.0,
            ),
        ));
    }
    let metrics: View = if narrow {
        StackPanel::new().spacing(14.0).keyed_children(
            cards
                .into_iter()
                .map(|(key, card)| KeyedView::new(key, card)),
        )
    } else {
        Grid::new()
            .columns([
                GridLength::Star(1.0),
                GridLength::Star(1.0),
                GridLength::Star(1.0),
            ])
            .rows([GridLength::Auto, GridLength::Auto])
            .column_spacing(14.0)
            .row_spacing(14.0)
            .keyed_children(cards.into_iter().enumerate().map(|(index, (key, card))| {
                KeyedView::new(
                    key,
                    Border::new()
                        .grid_column((index % 3) as i32)
                        .grid_row((index / 3) as i32)
                        .content(card),
                )
            }))
    };
    let connections_card: View = {
        let header = Grid::new()
            .columns([GridLength::Auto, GridLength::Star(1.0)])
            .children((
                TextBlock::new()
                    .text("NETWORK CONNECTIONS")
                    .font_size(11.0)
                    .font_weight(FontWeight::BOLD)
                    .foreground(palette.muted),
                Button::new()
                    .grid_column(1)
                    .width(86.0)
                    .height(30.0)
                    .horizontal_alignment(HorizontalAlignment::Right)
                    .is_enabled(!connections_loading)
                    .on_click(load_connections)
                    .content(if connections_loading {
                        fa_icon_label(FaIcon::Refresh, "…")
                    } else {
                        fa_icon_label(FaIcon::Refresh, "Load")
                    }),
            ));
        let rows: View = match connections {
            None => View::from(
                TextBlock::new()
                    .text("Press Load to capture the current TCP connection table.")
                    .font_size(12.0)
                    .foreground(palette.muted),
            ),
            Some([]) => View::from(
                TextBlock::new()
                    .text("No TCP connections found.")
                    .font_size(12.0),
            ),
            Some(list) => {
                let items: Vec<KeyedView> = list
                    .iter()
                    .take(10)
                    .enumerate()
                    .map(|(index, connection)| {
                        KeyedView::new(
                            format!(
                                "{index}:{}:{}:{}:{}",
                                connection.protocol,
                                connection.local_addr,
                                connection.remote_addr,
                                connection.status
                            ),
                            Border::new()
                                .min_height(32.0)
                                .border_brush(palette.border)
                                .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
                                .content(
                                    Grid::new()
                                        .columns(network_connection_columns(narrow))
                                        .column_spacing(8.0)
                                        .children((
                                            TextBlock::new()
                                                .text(connection.protocol.clone())
                                                .font_size(11.5)
                                                .vertical_alignment(VerticalAlignment::Center),
                                            TextBlock::new()
                                                .grid_column(1)
                                                .text(connection.local_addr.clone())
                                                .font_size(11.5)
                                                .text_trimming(TextTrimming::CharacterEllipsis)
                                                .vertical_alignment(VerticalAlignment::Center),
                                            TextBlock::new()
                                                .grid_column(2)
                                                .text(connection.remote_addr.clone())
                                                .font_size(11.5)
                                                .text_trimming(TextTrimming::CharacterEllipsis)
                                                .vertical_alignment(VerticalAlignment::Center),
                                            TextBlock::new()
                                                .grid_column(3)
                                                .text(connection.status.clone())
                                                .font_size(11.5)
                                                .foreground(palette.muted)
                                                .text_trimming(TextTrimming::CharacterEllipsis)
                                                .vertical_alignment(VerticalAlignment::Center),
                                        )),
                                ),
                        )
                    })
                    .collect();
                StackPanel::new().spacing(2.0).keyed_children(items)
            }
        };
        let column_headers: View = if connections.is_some_and(|list| !list.is_empty()) {
            Grid::new()
                .min_height(28.0)
                .columns(network_connection_columns(narrow))
                .column_spacing(8.0)
                .background(palette.card_strong)
                .children((
                    network_connection_header("PROTOCOL", 0, palette),
                    network_connection_header("LOCAL ADDRESS", 1, palette),
                    network_connection_header("REMOTE ADDRESS", 2, palette),
                    network_connection_header("STATE", 3, palette),
                ))
        } else {
            View::empty()
        };
        Border::new()
            .background(palette.card)
            .border_brush(palette.border)
            .border_thickness(1.0)
            .corner_radius(9.0)
            .padding(Thickness::new(14.0, 12.0, 14.0, 12.0))
            .content(
                StackPanel::new()
                    .spacing(8.0)
                    .children((header, column_headers, rows)),
            )
    };
    let error_notice: View = error.map_or_else(View::empty, |error| {
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
                            .text(if stats.is_some() {
                                format!(
                                    "Live telemetry stopped · {error} · showing the last successful sample"
                                )
                            } else {
                                format!("Live telemetry unavailable · {error}")
                            })
                            .font_size(11.5)
                            .foreground(palette.err)
                            .text_wrapping(TextWrapping::Wrap)
                            .vertical_alignment(VerticalAlignment::Center),
                        Button::new()
                            .grid_column(1)
                            .height(30.0)
                            .on_click(refresh)
                            .automation_name("Retry live monitoring")
                            .content("Retry"),
                    )),
            )
    });
    StackPanel::new().spacing(16.0).children((
        page_header(palette, Page::Monitor, View::empty()),
        Border::new()
            .height(32.0)
            .margin(Thickness::new(0.0, 6.0, 0.0, 0.0))
            .content(
                Grid::new()
                    .columns([GridLength::Auto, GridLength::Star(1.0), GridLength::Auto])
                    .column_spacing(14.0)
                    .children((
                        monitor_status_pill(palette, paused),
                        TextBlock::new()
                            .text(hardware_summary)
                            .grid_column(1)
                            .vertical_alignment(VerticalAlignment::Center)
                            .font_size(12.0)
                            .foreground(palette.muted)
                            .text_wrapping(TextWrapping::NoWrap)
                            .text_trimming(TextTrimming::CharacterEllipsis),
                        Border::new().grid_column(2).content(actions),
                    )),
            ),
        error_notice,
        Border::new()
            .margin(Thickness::new(0.0, 1.0, 0.0, 0.0))
            .content(metrics),
        Border::new()
            .margin(Thickness::new(0.0, 1.0, 0.0, 0.0))
            .content(connections_card),
    ))
}

pub(crate) fn network_connection_columns(narrow: bool) -> [GridLength; 4] {
    if narrow {
        [
            GridLength::Pixel(48.0),
            GridLength::Star(1.0),
            GridLength::Star(1.0),
            GridLength::Pixel(82.0),
        ]
    } else {
        [
            GridLength::Pixel(60.0),
            GridLength::Star(1.0),
            GridLength::Star(1.0),
            GridLength::Pixel(104.0),
        ]
    }
}

pub(crate) fn network_connection_header(
    label: &'static str,
    column: i32,
    palette: Palette,
) -> TextBlock {
    TextBlock::new()
        .grid_column(column)
        .text(label)
        .font_size(9.5)
        .font_weight(FontWeight::SEMI_BOLD)
        .foreground(palette.muted)
        .vertical_alignment(VerticalAlignment::Center)
        .text_trimming(TextTrimming::CharacterEllipsis)
}
