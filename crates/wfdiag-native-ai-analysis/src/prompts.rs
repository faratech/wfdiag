//! AI Prompt Templates
//!
//! Contains prompt templates for different AI analysis scenarios. Every
//! builder takes `max_data_chars` — the per-provider data budget derived from
//! `ai_providers::capabilities()` — instead of assuming Phi Silica's 4k-token
//! limit for everyone. Compact budgets (≲2k chars, i.e. Phi Silica) also get
//! tighter response-length instructions.

/// Budgets at or below this are "compact": data is heavily truncated and the
/// model is told to answer in a couple of sentences.
const COMPACT_BUDGET_CHARS: usize = 2_000;

/// Generate prompt for single diagnostic interpretation
#[must_use]
pub fn diagnostic_interpretation_prompt(
    task_name: &str,
    output: &str,
    max_data_chars: usize,
) -> String {
    // Convert JSON to readable text for better token efficiency
    let readable_output = json_to_readable_text(output, max_data_chars);
    let length_hint = if max_data_chars <= COMPACT_BUDGET_CHARS {
        "Give a 2-3 sentence summary. Note any concerns. Keep response under 80 words."
    } else {
        "Summarize the findings and call out anything concerning, with the values that \
         matter. Keep response under 200 words."
    };
    let task_hint = if task_name.to_ascii_lowercase().contains("windows update") {
        "This diagnostic reports installed update/hotfix records from Windows, not a raw \
         WindowsUpdate.log file. If installed_updates or hotfix_details contain entries, \
         treat that as non-empty data and summarize the KB IDs, descriptions, and install dates. \
         Do not ask for Get-WindowsUpdateLog unless the user specifically needs failure log \
         analysis beyond installed update history.\n\n"
    } else {
        ""
    };

    format!(
        r"Analyze this Windows diagnostic:

{task_name}:
{readable_output}

{task_hint}
Do not infer Windows release channel, Insider/Preview status, support status, or
missing cumulative updates from a base BuildNumber alone. Require UBR/FullBuild
or explicit live grounding before making those claims.

{length_hint}"
    )
}

/// Prefix a prompt with live RAG context. The grounding text is already
/// bounded and source-labeled by `ai_grounding`.
pub fn attach_grounding(prompt: String, grounding: Option<&str>) -> String {
    let Some(grounding) = grounding.map(str::trim).filter(|g| !g.is_empty()) else {
        return prompt;
    };
    format!("{grounding}\n\nANALYSIS TASK\n{prompt}")
}

/// Convert JSON diagnostic output to human-readable text
/// This dramatically reduces token count vs raw JSON
#[must_use]
pub fn json_to_readable_text(output: &str, max_chars: usize) -> String {
    // Try to parse as JSON
    let trimmed = output.trim();

    // If it's JSON, convert to readable format
    if (trimmed.starts_with('{') || trimmed.starts_with('['))
        && let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed)
    {
        let text = render_json_value(&json, 0);
        return truncate_output(&text, max_chars);
    }

    // Not JSON, just truncate the raw text
    truncate_output(output, max_chars)
}

/// Render a JSON value as readable text
fn render_json_value(value: &serde_json::Value, depth: usize) -> String {
    // Limit recursion depth
    if depth > 3 {
        return "[...]".to_string();
    }

    match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::Bool(b) => if *b { "Yes" } else { "No" }.to_string(),
        serde_json::Value::Number(n) => format_number_value(n),
        serde_json::Value::String(s) => {
            // Skip empty or very long strings
            if s.is_empty() || s.len() > 200 {
                if s.len() > 200 {
                    // Slice on a char boundary: &s[..100] panics if byte 100 splits a
                    // multi-byte UTF-8 char (localized strings, accented device names).
                    let head: String = s.chars().take(100).collect();
                    format!("{head}...")
                } else {
                    String::new()
                }
            } else {
                s.clone()
            }
        }
        serde_json::Value::Array(arr) => {
            // For arrays, show count and first few items
            if arr.is_empty() {
                return String::new();
            }

            let mut lines = Vec::new();
            for (i, item) in arr.iter().take(5).enumerate() {
                let rendered = render_json_value(item, depth + 1);
                if !rendered.is_empty() {
                    lines.push(format!("{}. {}", i + 1, rendered));
                }
            }
            if arr.len() > 5 {
                lines.push(format!("... and {} more", arr.len() - 5));
            }
            lines.join("\n")
        }
        serde_json::Value::Object(obj) => render_object_as_text(obj, depth),
    }
}

/// Render a JSON object as key: value text
fn render_object_as_text(obj: &serde_json::Map<String, serde_json::Value>, depth: usize) -> String {
    // Priority fields to show first (most important diagnostic info)
    let priority_fields = [
        "Name",
        "Caption",
        "Description",
        "Status",
        "State",
        "ProductName",
        "DisplayVersion",
        "ReleaseId",
        "CurrentBuild",
        "CurrentBuildNumber",
        "BuildNumber",
        "UBR",
        "HotFixID",
        "InstalledOn",
        "InstalledBy",
        "EditionID",
        "InstallationType",
        "KnownRelease",
        "InsiderEnrollment",
        "Capacity",
        "Size",
        "FreeSpace",
        "TotalPhysicalMemory",
        "Speed",
        "MaxClockSpeed",
        "NumberOfCores",
        "NumberOfLogicalProcessors",
        "Manufacturer",
        "Model",
        "Version",
        "DeviceID",
        "IPAddress",
        "MACAddress",
        "AdapterType",
    ];

    let mut lines = Vec::new();
    let mut shown_keys = std::collections::HashSet::new();

    // Show priority fields first
    for &field in &priority_fields {
        if let Some(val) = obj.get(field) {
            let rendered = render_json_value(val, depth + 1);
            if !rendered.is_empty() && rendered != "null" {
                // Format large numbers nicely
                let display_val = if field.contains("Size")
                    || field.contains("Capacity")
                    || field.contains("Memory")
                    || field.contains("Space")
                {
                    format_bytes_if_numeric(&rendered)
                } else {
                    rendered
                };
                lines.push(format!("{}: {}", format_field_name(field), display_val));
                shown_keys.insert(field.to_string());
            }
        }
    }

    // Show other non-empty fields (limit to avoid bloat)
    let max_other_fields = 10usize.saturating_sub(lines.len());
    let mut other_count = 0;

    for (key, val) in obj {
        if other_count >= max_other_fields {
            break;
        }
        if shown_keys.contains(key) {
            continue;
        }
        // Skip internal/technical fields
        if key.starts_with("__")
            || key.starts_with("Cim")
            || key.contains("Path")
            || key.contains("Class")
        {
            continue;
        }

        let rendered = render_json_value(val, depth + 1);
        if !rendered.is_empty() && rendered != "null" && rendered.len() < 150 {
            lines.push(format!("{}: {}", format_field_name(key), rendered));
            other_count += 1;
        }
    }

    lines.join("\n")
}

/// Format a field name for display (CamelCase -> readable)
fn format_field_name(name: &str) -> String {
    match name {
        "HotFixID" => return "Hot Fix ID".to_string(),
        "UBR" => return "UBR".to_string(),
        "OSArchitecture" => return "OS Architecture".to_string(),
        "IPAddress" => return "IP Address".to_string(),
        "MACAddress" => return "MAC Address".to_string(),
        _ => {}
    }
    // Add spaces before capitals
    let mut result = String::new();
    for (i, c) in name.chars().enumerate() {
        if i > 0 && c.is_uppercase() {
            result.push(' ');
        }
        result.push(c);
    }
    result
}

/// Preserve untyped JSON numbers exactly. A large number is not necessarily a
/// byte count (it may be a timestamp, counter, identifier, or frequency).
/// Byte formatting is applied only by `render_object_as_text` when the field
/// name establishes byte semantics.
fn format_number_value(n: &serde_json::Number) -> String {
    n.to_string()
}

/// Format a string as bytes if it looks like a byte count
// Byte counts are rendered for a human-readable prompt; a GB value losing
// sub-kilobyte precision in f64 is the intended rounding, not an error.
#[allow(clippy::cast_precision_loss)]
fn format_bytes_if_numeric(s: &str) -> String {
    if let Ok(bytes) = s.parse::<u64>() {
        if bytes > 1_000_000_000 {
            return format!("{:.1} GB", bytes as f64 / 1_073_741_824.0);
        } else if bytes > 1_000_000 {
            return format!("{:.1} MB", bytes as f64 / 1_048_576.0);
        }
    }
    s.to_string()
}

/// Generate prompt for section summary (Hardware, System, Storage, Network)
#[must_use]
pub fn section_summary_prompt(
    section_name: &str,
    section_data: &str,
    max_data_chars: usize,
) -> String {
    // Convert JSON to readable text
    let readable_data = json_to_readable_text(section_data, max_data_chars);
    let length_hint = if max_data_chars <= COMPACT_BUDGET_CHARS {
        "Under 100 words."
    } else {
        "Under 200 words."
    };

    format!(
        r"Summarize {section_name} diagnostics:

{readable_data}

Status (healthy/needs attention/critical), key findings (2-3 points), recommendation. {length_hint}"
    )
}

/// Generate prompt for health score explanation
#[must_use]
pub fn health_explanation_prompt(metrics_data: &str, max_data_chars: usize) -> String {
    // Bound the (otherwise unbounded) caller-supplied metrics so this prompt stays
    // within context limits like the sibling builders. truncate_output is char-safe.
    let metrics_data = truncate_output(metrics_data, max_data_chars);
    let length_hint = if max_data_chars <= COMPACT_BUDGET_CHARS {
        "Keep the response under 120 words."
    } else {
        "Keep the response under 200 words."
    };
    format!(
        r"Explain these Windows system health metrics to a user.

**Health Metrics:**
{metrics_data}

Provide:
1. Why the overall score is what it is
2. Which components are affecting the score most
3. What could be done to improve the score

{length_hint} Be encouraging but honest."
    )
}

/// Generate prompt for issue prioritization
#[must_use]
pub fn issue_prioritization_prompt(issues_data: &str, max_data_chars: usize) -> String {
    let readable_data = json_to_readable_text(issues_data, max_data_chars);
    let length_hint = if max_data_chars <= COMPACT_BUDGET_CHARS {
        "Under 100 words."
    } else {
        "Under 200 words."
    };

    format!(
        r"Prioritize these Windows issues:

{readable_data}

Rank by priority, brief reason each, which to fix first. {length_hint}"
    )
}

/// Truncate output to fit within context limits.
/// Truncates by CHARACTER count (not byte index) so a multi-byte UTF-8 sequence at
/// the boundary can never trigger a "byte index is not a char boundary" panic. With
/// `panic = "abort"` in release this would otherwise crash the whole process.
#[must_use]
pub fn truncate_output(output: &str, max_chars: usize) -> String {
    let total = output.chars().count();
    if total <= max_chars {
        return output.to_string();
    }
    if max_chars == 0 {
        return String::new();
    }

    // The old implementation took `max_chars` and then appended a suffix,
    // silently violating every provider budget. Use a fixed-size suffix whose
    // length is included in the bound. It reports the original size rather
    // than an inaccurate "remaining" count that changes with suffix length.
    let suffix = format!("… [truncated from {total} chars]");
    let suffix_chars = suffix.chars().count();
    if suffix_chars >= max_chars {
        return suffix.chars().take(max_chars).collect();
    }
    let mut bounded: String = output.chars().take(max_chars - suffix_chars).collect();
    bounded.push_str(&suffix);
    bounded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_output() {
        let short = "short text";
        assert_eq!(truncate_output(short, 100), short);

        let long = "a".repeat(200);
        let truncated = truncate_output(&long, 100);
        assert_eq!(truncated.chars().count(), 100);
        assert!(truncated.contains("[truncated"));
    }

    #[test]
    fn test_truncate_output_multibyte_no_panic() {
        // Regression: byte-index slicing panicked when the cut landed inside a
        // multi-byte UTF-8 char. Each 'é' is 2 bytes, so a byte slice at 100 would
        // split a char. Char-based truncation must fit exactly within the cap.
        let multibyte = "é".repeat(200);
        let truncated = truncate_output(&multibyte, 100);
        assert_eq!(truncated.chars().count(), 100);
        assert!(truncated.starts_with('é'));
        assert!(truncated.contains("[truncated from 200 chars]"));

        // Emoji are 4 bytes; truncating right at a boundary must also not panic.
        let emoji = "🚀".repeat(50);
        let _ = truncate_output(&emoji, 25);
        let _ = render_json_value(&serde_json::Value::String("界".repeat(300)), 0);

        for cap in 0..40 {
            assert!(truncate_output(&"🧭".repeat(100), cap).chars().count() <= cap);
        }
    }

    #[test]
    fn budgets_control_data_truncation_and_length_hints() {
        let data = "x".repeat(10_000);
        // Compact (Phi Silica-sized) budget: heavy truncation + tight cap
        let compact = diagnostic_interpretation_prompt("Disk Drives", &data, 1_200);
        assert!(compact.len() < 2_000);
        assert!(compact.contains("under 80 words"));
        // Cloud-sized budget: the same data survives intact + relaxed cap
        let roomy = diagnostic_interpretation_prompt("Disk Drives", &data, 20_000);
        assert!(roomy.len() > 9_000);
        assert!(roomy.contains("under 200 words"));

        assert!(section_summary_prompt("Storage", &data, 1_500).contains("Under 100 words."));
        assert!(section_summary_prompt("Storage", &data, 20_000).contains("Under 200 words."));
        assert!(health_explanation_prompt(&data, 1_500).contains("under 120 words"));
        assert!(issue_prioritization_prompt(&data, 20_000).contains("Under 200 words."));
    }

    #[test]
    fn attach_grounding_prefixes_prompt() {
        let prompt = attach_grounding(
            "Analyze this".to_string(),
            Some("LIVE RAG GROUNDING\n- doc"),
        );
        assert!(prompt.starts_with("LIVE RAG GROUNDING"));
        assert!(prompt.contains("ANALYSIS TASK\nAnalyze this"));
        assert_eq!(attach_grounding("No rag".into(), None), "No rag");
    }

    #[test]
    fn windows_update_prompt_treats_hotfixes_as_data() {
        let prompt = diagnostic_interpretation_prompt(
            "Windows Update History",
            r#"{"installed_updates":[{"HotFixID":"KB5094126","Description":"Security Update","InstalledOn":"6/9/2026"}]}"#,
            20_000,
        );
        assert!(prompt.contains("not a raw WindowsUpdate.log file"));
        assert!(prompt.contains("treat that as non-empty data"));
        assert!(prompt.contains("Hot Fix ID: KB5094126"));
    }

    #[test]
    fn untyped_large_numbers_are_not_mislabeled_as_bytes() {
        let rendered = json_to_readable_text(
            r#"{"timestamp":1718123456789,"event_id":4294967296,"Capacity":17179869184}"#,
            1_000,
        );
        assert!(rendered.contains("timestamp: 1718123456789"));
        assert!(rendered.contains("event_id: 4294967296"));
        assert!(rendered.contains("Capacity: 16.0 GB"));
    }
}
