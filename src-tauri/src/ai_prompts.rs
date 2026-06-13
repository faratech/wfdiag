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

    format!(
        r#"Analyze this Windows diagnostic:

{task_name}:
{output}

{length_hint}"#,
        task_name = task_name,
        output = readable_output,
        length_hint = length_hint
    )
}

/// Convert JSON diagnostic output to human-readable text
/// This dramatically reduces token count vs raw JSON
pub(crate) fn json_to_readable_text(output: &str, max_chars: usize) -> String {
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
                    format!("{}...", head)
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
    let max_other_fields = 10 - lines.len();
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

/// Format a number, converting bytes to human-readable
fn format_number_value(n: &serde_json::Number) -> String {
    if let Some(i) = n.as_u64() {
        // Check if it looks like bytes (very large number)
        if i > 1_000_000_000 {
            format!("{:.1} GB", i as f64 / 1_073_741_824.0)
        } else if i > 1_000_000 {
            format!("{:.1} MB", i as f64 / 1_048_576.0)
        } else {
            i.to_string()
        }
    } else if let Some(f) = n.as_f64() {
        if f > 1_000_000_000.0 {
            format!("{:.1} GB", f / 1_073_741_824.0)
        } else {
            format!("{:.1}", f)
        }
    } else {
        n.to_string()
    }
}

/// Format a string as bytes if it looks like a byte count
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
        r#"Summarize {section_name} diagnostics:

{section_data}

Status (healthy/needs attention/critical), key findings (2-3 points), recommendation. {length_hint}"#,
        section_name = section_name,
        section_data = readable_data,
        length_hint = length_hint
    )
}

/// Generate prompt for health score explanation
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
        r#"Explain these Windows system health metrics to a user.

**Health Metrics:**
{metrics_data}

Provide:
1. Why the overall score is what it is
2. Which components are affecting the score most
3. What could be done to improve the score

{length_hint} Be encouraging but honest."#,
        metrics_data = metrics_data,
        length_hint = length_hint
    )
}

/// Generate prompt for issue prioritization
pub fn issue_prioritization_prompt(issues_data: &str, max_data_chars: usize) -> String {
    let readable_data = json_to_readable_text(issues_data, max_data_chars);
    let length_hint = if max_data_chars <= COMPACT_BUDGET_CHARS {
        "Under 100 words."
    } else {
        "Under 200 words."
    };

    format!(
        r#"Prioritize these Windows issues:

{issues_data}

Rank by priority, brief reason each, which to fix first. {length_hint}"#,
        issues_data = readable_data,
        length_hint = length_hint
    )
}

/// Truncate output to fit within context limits.
/// Truncates by CHARACTER count (not byte index) so a multi-byte UTF-8 sequence at
/// the boundary can never trigger a "byte index is not a char boundary" panic. With
/// `panic = "abort"` in release this would otherwise crash the whole process.
pub(crate) fn truncate_output(output: &str, max_chars: usize) -> String {
    let total = output.chars().count();
    if total <= max_chars {
        output.to_string()
    } else {
        let head: String = output.chars().take(max_chars).collect();
        format!(
            "{}... [truncated, {} more characters]",
            head,
            total - max_chars
        )
    }
}

/// Compress diagnostic output for Phi Silica's limited context
/// Extracts key information and removes verbose content
#[allow(dead_code)] // Available for Phi Silica optimizations
pub fn compress_for_phi_silica(output: &str, max_chars: usize) -> String {
    // Remove excessive whitespace
    let compressed: String = output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.trim())
        .collect::<Vec<_>>()
        .join("\n");

    // If still too long, truncate smartly
    if compressed.len() <= max_chars {
        compressed
    } else {
        // Try to keep first part (usually headers/summary) and last part (usually
        // important). Slice by char count, not byte index, to avoid a char-boundary panic.
        let half = max_chars / 2;
        let total = compressed.chars().count();
        let first_part: String = compressed.chars().take(half).collect();
        let last_part: String = compressed
            .chars()
            .skip(total.saturating_sub(half))
            .collect();

        format!(
            "{}\n... [content truncated] ...\n{}",
            first_part.trim(),
            last_part.trim()
        )
    }
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
        assert!(truncated.chars().count() < 200);
        assert!(truncated.contains("[truncated"));
    }

    #[test]
    fn test_truncate_output_multibyte_no_panic() {
        // Regression: byte-index slicing panicked when the cut landed inside a
        // multi-byte UTF-8 char. Each 'é' is 2 bytes, so a byte slice at 100 would
        // split a char. Char-based truncation must keep exactly `max_chars` chars.
        let multibyte = "é".repeat(200);
        let truncated = truncate_output(&multibyte, 100);
        assert!(truncated.starts_with(&"é".repeat(100)));
        assert!(truncated.contains("[truncated, 100 more characters]"));

        // Emoji are 4 bytes; truncating right at a boundary must also not panic.
        let emoji = "🚀".repeat(50);
        let _ = truncate_output(&emoji, 25);
        let _ = render_json_value(&serde_json::Value::String("界".repeat(300)), 0);
    }

    #[test]
    fn test_compress_for_phi_silica() {
        let verbose = "line1\n\n\n  line2  \n\n  line3  ";
        let compressed = compress_for_phi_silica(verbose, 1000);
        assert_eq!(compressed, "line1\nline2\nline3");
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
}
