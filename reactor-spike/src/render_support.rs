//! Pure projections used by the native Reactor renderer.
//!
//! Keeping these calculations free of WinUI types makes the fixed-size
//! virtualization and compact monitor geometry contracts cheap to test on
//! every host.

use std::fmt::Write as _;

pub const PROCESS_REPEATER_SLOTS: usize = 100;

pub fn fixed_process_slots<T>(rows: &[T]) -> impl ExactSizeIterator<Item = (usize, Option<&T>)> {
    (0..PROCESS_REPEATER_SLOTS).map(|index| (index, rows.get(index)))
}

pub const MONITOR_GRAPH_WIDTH: f64 = 300.0;
pub const MONITOR_GRAPH_HEIGHT: f64 = 72.0;
pub const MONITOR_GRAPH_PATH_COUNT: usize = 2;

const MONITOR_GRAPH_BASELINE: f64 = 68.0;
const MONITOR_GRAPH_LINE_HALF_WIDTH: f64 = 0.8;
const MONITOR_GRAPH_SAMPLES: usize = 60;

#[derive(Debug, PartialEq)]
pub struct MonitorGraphGeometry {
    pub area: String,
    pub ribbon: String,
}

fn monitor_graph_points(series: &[f64], max: f64) -> Vec<(f64, f64)> {
    let max = if max.is_finite() && max > 0.0 {
        max
    } else {
        1.0
    };
    let graph_y = |value: f64| {
        let value = if value.is_finite() { value } else { 0.0 };
        MONITOR_GRAPH_BASELINE - (value / max).clamp(0.0, 1.0) * (MONITOR_GRAPH_HEIGHT - 12.0)
    };
    let start = series.len().saturating_sub(MONITOR_GRAPH_SAMPLES);
    let series = &series[start..];
    if series.len() <= 1 {
        return vec![
            (0.0, MONITOR_GRAPH_BASELINE),
            (MONITOR_GRAPH_WIDTH, MONITOR_GRAPH_BASELINE),
        ];
    }

    let step = MONITOR_GRAPH_WIDTH / (series.len() - 1) as f64;
    series
        .iter()
        .enumerate()
        .map(|(index, value)| (index as f64 * step, graph_y(*value)))
        .collect()
}

fn push_line_to(path: &mut String, x: f64, y: f64) {
    // Three decimal places are well below a physical pixel after the Viewbox
    // transform while keeping the per-sample geometry compact. One `write!`
    // per point remains (the buffer below is pre-sized so this no longer
    // reallocates); that per-point cost is inherent float-to-text formatting
    // for the geometry the OS actually needs, not wasted work to remove.
    write!(path, " L{x:.3} {y:.3}").expect("writing to a String cannot fail");
}

/// Build two filled XAML geometries: the translucent area and a thin closed
/// ribbon that visually matches the former stroked polyline.
pub fn monitor_graph_geometry(series: &[f64], max: f64) -> MonitorGraphGeometry {
    let points = monitor_graph_points(series, max);

    // Reserve up front: this geometry is rebuilt on every telemetry tick, so
    // skip the repeated growth reallocations (~16 bytes per path segment).
    let mut area = String::with_capacity(points.len() * 16 + 32);
    area.push_str("F1 M0.000 ");
    write!(
        area,
        "{MONITOR_GRAPH_HEIGHT:.3}",
    )
    .expect("writing to a String cannot fail");
    for &(x, y) in &points {
        push_line_to(&mut area, x, y);
    }
    push_line_to(&mut area, MONITOR_GRAPH_WIDTH, MONITOR_GRAPH_HEIGHT);
    area.push_str(" Z");

    let mut ribbon = String::with_capacity(points.len() * 32 + 16);
    let (first_x, first_y) = points[0];
    write!(
        ribbon,
        "{first_x:.3} {:.3}",
        (first_y - MONITOR_GRAPH_LINE_HALF_WIDTH).max(0.0)
    )
    .expect("writing to a String cannot fail");
    for &(x, y) in points.iter().skip(1) {
        push_line_to(&mut ribbon, x, (y - MONITOR_GRAPH_LINE_HALF_WIDTH).max(0.0));
    }
    for &(x, y) in points.iter().rev() {
        push_line_to(
            &mut ribbon,
            x,
            (y + MONITOR_GRAPH_LINE_HALF_WIDTH).min(MONITOR_GRAPH_HEIGHT),
        );
    }
    ribbon.push_str(" Z");

    MonitorGraphGeometry { area, ribbon }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_slots_keep_the_same_hundred_positional_keys() {
        let short = ["a", "b"];
        let reordered = ["b", "a", "c"];
        let short_slots = fixed_process_slots(&short).collect::<Vec<_>>();
        let reordered_slots = fixed_process_slots(&reordered).collect::<Vec<_>>();

        assert_eq!(short_slots.len(), PROCESS_REPEATER_SLOTS);
        assert_eq!(reordered_slots.len(), PROCESS_REPEATER_SLOTS);
        assert_eq!(
            short_slots.iter().map(|(key, _)| *key).collect::<Vec<_>>(),
            reordered_slots
                .iter()
                .map(|(key, _)| *key)
                .collect::<Vec<_>>()
        );
        assert_eq!(short_slots[0], (0, Some(&"a")));
        assert_eq!(reordered_slots[0], (0, Some(&"b")));
        assert_eq!(short_slots[2], (2, None));
        assert_eq!(reordered_slots[2], (2, Some(&"c")));
        assert_eq!(short_slots[99], (99, None));
    }

    #[test]
    fn monitor_graph_is_always_two_closed_filled_paths() {
        assert_eq!(MONITOR_GRAPH_PATH_COUNT, 2);
        let geometry = monitor_graph_geometry(&[0.0, 50.0, 100.0], 100.0);
        assert!(geometry.area.starts_with("F1 M0.000 72.000"));
        assert!(geometry.area.ends_with(" Z"));
        assert!(geometry.ribbon.starts_with("F1 M"));
        assert!(geometry.ribbon.ends_with(" Z"));
        assert!(geometry.area.contains("L150.000 38.000"));
    }

    #[test]
    fn monitor_graph_caps_input_to_the_sixty_sample_history() {
        let samples = (0..80).map(f64::from).collect::<Vec<_>>();
        let points = monitor_graph_points(&samples, 100.0);
        assert_eq!(points.len(), MONITOR_GRAPH_SAMPLES);
        assert_eq!(points.first().map(|point| point.0), Some(0.0));
        assert_eq!(
            points.last().map(|point| point.0),
            Some(MONITOR_GRAPH_WIDTH)
        );
        assert!(points.iter().all(|(x, y)| x.is_finite() && y.is_finite()));
    }

    #[test]
    fn monitor_graph_sanitizes_non_finite_samples_and_scale() {
        let geometry = monitor_graph_geometry(&[f64::NAN, f64::INFINITY], f64::NAN);
        assert!(!geometry.area.contains("NaN"));
        assert!(!geometry.area.contains("inf"));
        assert!(!geometry.ribbon.contains("NaN"));
        assert!(!geometry.ribbon.contains("inf"));
    }
}
