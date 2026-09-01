//! The command palette: its command catalogue, fuzzy match, and views.

#![deny(unsafe_code)]

use crate::widgets::icons::FaIcon;
use crate::widgets::palette_colors::Palette;
use std::borrow::Cow;
use windows_reactor::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PaletteCommandSpec {
    pub(crate) section: &'static str,
    pub(crate) label: Cow<'static, str>,
    pub(crate) tag: Cow<'static, str>,
    pub(crate) keywords: Cow<'static, str>,
    pub(crate) enabled: bool,
    pub(crate) icon: FaIcon,
    pub(crate) shortcut: Option<Cow<'static, str>>,
}

pub(crate) const PALETTE_MAX_RESULTS: usize = 14;

#[derive(Clone, Copy)]
pub(crate) struct PaletteCommandTemplate {
    pub(crate) section: &'static str,
    pub(crate) label: &'static str,
    pub(crate) tag: &'static str,
    pub(crate) keywords: &'static str,
    pub(crate) icon: FaIcon,
    pub(crate) shortcut: Option<&'static str>,
}

impl PaletteCommandTemplate {
    pub(crate) fn command(self, enabled: bool) -> PaletteCommandSpec {
        PaletteCommandSpec {
            section: self.section,
            label: Cow::Borrowed(self.label),
            tag: Cow::Borrowed(self.tag),
            keywords: Cow::Borrowed(self.keywords),
            enabled,
            icon: self.icon,
            shortcut: self.shortcut.map(Cow::Borrowed),
        }
    }
}

pub(crate) const PALETTE_NAVIGATION_TEMPLATES: [PaletteCommandTemplate; 6] = [
    PaletteCommandTemplate {
        section: "Navigate",
        label: "Go to Diagnostics",
        tag: "diagnostics",
        keywords: "page screen navigate",
        icon: FaIcon::Diagnostics,
        shortcut: Some("Ctrl+1"),
    },
    PaletteCommandTemplate {
        section: "Navigate",
        label: "Go to Live Monitor",
        tag: "monitor",
        keywords: "page screen navigate",
        icon: FaIcon::Monitor,
        shortcut: Some("Ctrl+2"),
    },
    PaletteCommandTemplate {
        section: "Navigate",
        label: "Go to Processes",
        tag: "processes",
        keywords: "page screen navigate",
        icon: FaIcon::Processes,
        shortcut: Some("Ctrl+3"),
    },
    PaletteCommandTemplate {
        section: "Navigate",
        label: "Go to AI Analysis",
        tag: "ai",
        keywords: "page screen navigate",
        icon: FaIcon::Ai,
        shortcut: Some("Ctrl+4"),
    },
    PaletteCommandTemplate {
        section: "Navigate",
        label: "Go to Issues",
        tag: "issues",
        keywords: "page screen navigate",
        icon: FaIcon::Issues,
        shortcut: Some("Ctrl+5"),
    },
    PaletteCommandTemplate {
        section: "Navigate",
        label: "Go to History",
        tag: "history",
        keywords: "page screen navigate",
        icon: FaIcon::History,
        shortcut: Some("Ctrl+6"),
    },
];

pub(crate) const PALETTE_SCAN_TEMPLATES: [PaletteCommandTemplate; 2] = [
    PaletteCommandTemplate {
        section: "Scan",
        label: "Run Quick Scan",
        tag: "quick-scan",
        keywords: "diagnostic fast ctrl shift q",
        icon: FaIcon::Bolt,
        shortcut: Some("Ctrl+Shift+Q"),
    },
    PaletteCommandTemplate {
        section: "Scan",
        label: "Run Full Scan",
        tag: "full-scan",
        keywords: "diagnostic complete ctrl shift f",
        icon: FaIcon::ListCheck,
        shortcut: Some("Ctrl+Shift+F"),
    },
];

pub(crate) const PALETTE_STOP_SCAN_TEMPLATE: PaletteCommandTemplate = PaletteCommandTemplate {
    section: "Scan",
    label: "Stop Scan",
    tag: "stop-scan",
    keywords: "cancel stop diagnostic",
    icon: FaIcon::Xmark,
    shortcut: None,
};

pub(crate) const PALETTE_REPORT_TEMPLATES: [PaletteCommandTemplate; 5] = [
    PaletteCommandTemplate {
        section: "Report",
        label: "Copy Report to Clipboard",
        tag: "copy-diagnostic-report",
        keywords: "clipboard forum copy",
        icon: FaIcon::Copy,
        shortcut: None,
    },
    PaletteCommandTemplate {
        section: "Report",
        label: "Export Report…",
        tag: "export",
        keywords: "save file json txt html",
        icon: FaIcon::FileExport,
        shortcut: None,
    },
    PaletteCommandTemplate {
        section: "Report",
        label: "Share to WindowsForum",
        tag: "share",
        keywords: "forum browser clipboard",
        icon: FaIcon::ShareNodes,
        shortcut: None,
    },
    PaletteCommandTemplate {
        section: "Report",
        label: "Email Report",
        tag: "email",
        keywords: "mail compose clipboard",
        icon: FaIcon::PaperPlane,
        shortcut: None,
    },
    PaletteCommandTemplate {
        section: "Report",
        label: "Generate Support Package",
        tag: "support-package",
        keywords: "support package json txt html bundle",
        icon: FaIcon::Download,
        shortcut: None,
    },
];

pub(crate) const PALETTE_APP_TEMPLATES: [PaletteCommandTemplate; 2] = [
    PaletteCommandTemplate {
        section: "App",
        label: "Open Settings",
        tag: "settings",
        keywords: "preferences configuration",
        icon: FaIcon::Gear,
        shortcut: None,
    },
    PaletteCommandTemplate {
        section: "App",
        label: "About WindowsForum Diagnostics",
        tag: "about",
        keywords: "version information",
        icon: FaIcon::CircleInfo,
        shortcut: None,
    },
];

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PaletteFuzzyResult {
    pub(crate) score: f64,
    pub(crate) indices: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PaletteMatch {
    pub(crate) command: PaletteCommandSpec,
    pub(crate) score: f64,
    pub(crate) indices: Vec<usize>,
}

/// Exact port of the shipping React palette matcher: case-insensitive
/// subsequence scoring with word-start, consecutive-run, early-hit, and tight
/// spread bonuses. Character indices are retained for title highlighting.
pub(crate) fn palette_fuzzy_score(query: &str, target: &str) -> Option<PaletteFuzzyResult> {
    let query = query.to_lowercase().chars().collect::<Vec<_>>();
    let target = target.to_lowercase().chars().collect::<Vec<_>>();
    if query.is_empty() {
        return Some(PaletteFuzzyResult {
            score: 0.0,
            indices: Vec::new(),
        });
    }
    if query.len() > target.len() {
        return None;
    }

    let mut indices = Vec::with_capacity(query.len());
    let mut score = 0.0_f64;
    let mut target_index = 0_usize;
    let mut previous_match = None;
    for needle in query {
        let relative = target[target_index..]
            .iter()
            .position(|character| *character == needle)?;
        let found = target_index + relative;
        let mut bonus = 1.0;
        if found == 0 {
            bonus += 8.0;
        } else if matches!(target[found - 1], ' ' | '-' | '_' | '/' | '.' | ':') {
            bonus += 6.0;
        }
        if previous_match.is_some_and(|previous| found == previous + 1) {
            bonus += 4.0;
        }
        bonus += (3.0 - found as f64 * 0.05).max(0.0);
        score += bonus;
        indices.push(found);
        previous_match = Some(found);
        target_index = found + 1;
    }

    let spread = indices.last()? - indices.first()? + 1;
    score += (indices.len() as f64 * 2.0 - (spread - indices.len()) as f64).max(0.0);
    Some(PaletteFuzzyResult { score, indices })
}

pub(crate) fn palette_section_order(section: &str) -> usize {
    match section {
        "Navigate" => 0,
        "Scan" => 1,
        "Report" => 2,
        "App" => 3,
        "Diagnostics" => 4,
        _ => 5,
    }
}

pub(crate) fn palette_visible_matches(
    commands: Vec<PaletteCommandSpec>,
    query: &str,
) -> Vec<PaletteMatch> {
    let query = query.trim();
    let mut matches = if query.is_empty() {
        commands
            .into_iter()
            .filter(|command| matches!(command.section, "Navigate" | "Scan"))
            .map(|command| PaletteMatch {
                command,
                score: 0.0,
                indices: Vec::new(),
            })
            .collect::<Vec<_>>()
    } else {
        commands
            .into_iter()
            .filter_map(|command| {
                if let Some(title_match) = palette_fuzzy_score(query, &command.label) {
                    return Some(PaletteMatch {
                        command,
                        score: title_match.score,
                        indices: title_match.indices,
                    });
                }
                palette_fuzzy_score(query, &command.keywords).map(|keyword_match| PaletteMatch {
                    command,
                    score: keyword_match.score * 0.6,
                    indices: Vec::new(),
                })
            })
            .collect::<Vec<_>>()
    };

    if !query.is_empty() {
        // Stable descending sort retains source command order for exact ties,
        // matching modern JavaScript Array.sort semantics.
        matches.sort_by(|left, right| right.score.total_cmp(&left.score));
        matches.truncate(PALETTE_MAX_RESULTS);
    }
    matches.sort_by_key(|matched| palette_section_order(matched.command.section));
    matches
}

pub(crate) fn diagnostic_palette_icon(category: &str) -> FaIcon {
    match category.trim().to_ascii_lowercase().as_str() {
        "system" => FaIcon::Desktop,
        "hardware" => FaIcon::Microchip,
        "storage" => FaIcon::HardDrive,
        "network" => FaIcon::Globe,
        "drivers" | "software" => FaIcon::Gear,
        "logs" => FaIcon::ClockRotateLeft,
        "performance" => FaIcon::ChartLine,
        "debug" => FaIcon::Stethoscope,
        _ => FaIcon::Diagnostics,
    }
}

pub(crate) fn command_palette_key_chip(palette: Palette, keys: impl Into<String>) -> View {
    Border::new()
        .min_height(20.0)
        .padding(Thickness::xy(6.0, 1.0))
        .background(palette.card)
        .border_brush(palette.border)
        .border_thickness(1.0)
        .corner_radius(4.0)
        .content(
            TextBlock::new()
                .text(keys)
                .font_size(10.0)
                .foreground(palette.muted)
                .vertical_alignment(VerticalAlignment::Center),
        )
}

pub(crate) fn command_palette_highlighted_label(
    palette: Palette,
    label: String,
    _indices: &[usize],
    enabled: bool,
) -> View {
    let normal = if enabled { palette.text } else { palette.muted };
    // Reactor does not yet expose TextBlock inline Runs. Multiple adjacent
    // TextBlocks cannot ellipsize as one label and can force the shortcut
    // column outside the popup viewport. Keep the fuzzy match in the search
    // model while rendering one bounded native text element.
    TextBlock::new()
        .grid_column(1)
        .text(label)
        .font_size(13.5)
        .foreground(normal)
        .text_trimming(TextTrimming::CharacterEllipsis)
        .vertical_alignment(VerticalAlignment::Center)
        .into()
}

pub(crate) fn command_palette_footer(palette: Palette, expanded: bool) -> View {
    if !expanded {
        return TextBlock::new()
            .text("↑ ↓ navigate   ·   Enter run   ·   Esc close")
            .font_size(11.0)
            .foreground(palette.muted)
            .vertical_alignment(VerticalAlignment::Center)
            .into();
    }

    let footer_label = |label: &'static str| {
        TextBlock::new()
            .text(label)
            .font_size(11.0)
            .foreground(palette.muted)
            .vertical_alignment(VerticalAlignment::Center)
    };
    StackPanel::new()
        .orientation(Orientation::Horizontal)
        .spacing(14.0)
        .vertical_alignment(VerticalAlignment::Center)
        .children((
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(4.0)
                .children((
                    command_palette_key_chip(palette, "↑"),
                    command_palette_key_chip(palette, "↓"),
                    footer_label("navigate"),
                )),
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(5.0)
                .children((
                    command_palette_key_chip(palette, "Enter"),
                    footer_label("run"),
                )),
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(5.0)
                .children((
                    command_palette_key_chip(palette, "Esc"),
                    footer_label("close"),
                )),
        ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette_spec(
        section: &'static str,
        label: impl Into<String>,
        tag: impl Into<String>,
    ) -> PaletteCommandSpec {
        PaletteCommandSpec {
            section,
            label: Cow::Owned(label.into()),
            tag: Cow::Owned(tag.into()),
            keywords: Cow::Borrowed("alpha command"),
            enabled: true,
            icon: FaIcon::Diagnostics,
            shortcut: None,
        }
    }

    #[test]
    fn command_palette_defaults_caps_search_and_preserves_section_order() {
        let defaults = palette_visible_matches(
            vec![
                palette_spec("App", "Open Settings", "settings"),
                palette_spec("Scan", "Run Quick Scan", "quick"),
                palette_spec("Navigate", "Go to Diagnostics", "diagnostics"),
                palette_spec("Report", "Export Report", "export"),
            ],
            "",
        );
        assert_eq!(
            defaults
                .iter()
                .map(|matched| matched.command.tag.as_ref())
                .collect::<Vec<_>>(),
            ["diagnostics", "quick"]
        );

        let matches = palette_visible_matches(
            (0..20)
                .map(|index| {
                    palette_spec(
                        "Diagnostics",
                        format!("Alpha command {index:02}"),
                        format!("run:{index:02}"),
                    )
                })
                .collect(),
            "alpha",
        );
        assert_eq!(matches.len(), PALETTE_MAX_RESULTS);
        assert_eq!(
            matches.first().map(|matched| matched.command.tag.as_ref()),
            Some("run:00")
        );
        assert_eq!(
            matches.last().map(|matched| matched.command.tag.as_ref()),
            Some("run:13")
        );

        let grouped = palette_visible_matches(
            vec![
                palette_spec("Diagnostics", "Alpha diagnostic", "diagnostic"),
                palette_spec("App", "Alpha app", "app"),
                palette_spec("Navigate", "Alpha page", "page"),
                palette_spec("Report", "Alpha report", "report"),
                palette_spec("Scan", "Alpha scan", "scan"),
            ],
            "alpha",
        );
        assert_eq!(
            grouped
                .iter()
                .map(|matched| matched.command.section)
                .collect::<Vec<_>>(),
            ["Navigate", "Scan", "Report", "App", "Diagnostics"]
        );
    }

    #[test]
    fn command_palette_matches_titles_before_keywords_and_never_section_names() {
        let matches = palette_visible_matches(
            vec![
                palette_spec("App", "Open Settings", "settings"),
                palette_spec("Navigate", "Open Processes", "process list"),
                PaletteCommandSpec {
                    keywords: Cow::Borrowed("open preferences"),
                    ..palette_spec("Diagnostics", "Configure providers", "providers")
                },
            ],
            "open",
        );
        assert_eq!(
            matches
                .iter()
                .map(|matched| matched.command.tag.as_ref())
                .collect::<Vec<_>>(),
            ["process list", "settings", "providers"]
        );
        assert!(!matches[0].indices.is_empty());
        assert!(!matches[1].indices.is_empty());
        assert!(matches[2].indices.is_empty());
        assert!(matches[1].score > matches[2].score);

        let section_only = palette_visible_matches(
            vec![palette_spec("App", "Toggle theme", "theme color")],
            "app",
        );
        assert!(section_only.is_empty());
    }

    #[test]
    fn command_palette_fuzzy_match_returns_shipping_highlight_indices() {
        let result = palette_fuzzy_score("ops", "Open Settings").expect("subsequence matches");
        assert_eq!(result.indices, [0, 1, 5]);
        assert!(result.score > 0.0);
        assert!(palette_fuzzy_score("settings open", "Open Settings").is_none());
    }
}
