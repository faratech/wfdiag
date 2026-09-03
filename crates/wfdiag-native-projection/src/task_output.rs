//! Bounded projections of diagnostic task output.
//!
//! Collector output is attacker-influenced text of unbounded size. Everything
//! here parses or truncates *before* materializing rows, with hard byte/row
//! caps, so a view (or an export renderer) can never copy an entire payload
//! per frame. Moved out of the native shell's detail view — rule 2: a parse
//! that needs no window belongs in a crate (2026-09-03 audit).

/// Maximum input accepted for structured (JSON) formatting; anything larger
/// falls back to a bounded preview instead of being parsed.
pub const MAX_STRUCTURED_OUTPUT_INPUT_BYTES: usize = 128 * 1024;

/// Maximum rows materialized from one structured document.
pub const MAX_STRUCTURED_OUTPUT_ROWS: usize = 256;

/// Maximum bytes of row content materialized from one structured document.
pub const MAX_STRUCTURED_OUTPUT_BYTES: usize = 48 * 1024;

/// Maximum pretty-printed input for the raw JSON document view.
pub const MAX_RAW_PRETTY_INPUT_BYTES: usize = 32 * 1024;

/// Fallback budget for raw output that cannot be pretty-printed.
pub const MAX_RAW_FALLBACK_BYTES: usize = 24 * 1024;

/// Appended when raw output was cut before formatting.
pub const RAW_OUTPUT_TRUNCATION_NOTICE: &str =
    "… Raw output truncated before formatting; the complete result remains available for export.";

/// Bounded key/value rows rendered for one task's structured output.
#[derive(Debug, Eq, PartialEq)]
pub struct FormattedOutputRows {
    pub rows: Vec<(String, String)>,
    pub byte_len: usize,
    pub truncated: bool,
}

impl Default for FormattedOutputRows {
    fn default() -> Self {
        Self::new()
    }
}

impl FormattedOutputRows {
    #[must_use]
    pub fn new() -> Self {
        Self {
            rows: Vec::new(),
            byte_len: 0,
            truncated: false,
        }
    }

    pub fn accepts(&mut self, key_bytes: usize, value_bytes: usize) -> bool {
        if self.truncated {
            return false;
        }
        let row_bytes = key_bytes.saturating_add(value_bytes);
        if self.rows.len() >= MAX_STRUCTURED_OUTPUT_ROWS
            || self.byte_len.saturating_add(row_bytes) > MAX_STRUCTURED_OUTPUT_BYTES
        {
            self.truncated = true;
            return false;
        }
        self.byte_len += row_bytes;
        true
    }

    pub fn push(&mut self, key: String, value: String) {
        if !self.accepts(key.len(), value.len()) {
            return;
        }
        self.rows.push((key, value));
    }

    pub fn push_str(&mut self, key: String, value: &str) {
        if !self.accepts(key.len(), value.len()) {
            return;
        }
        self.rows.push((key, value.to_string()));
    }
}

/// The at-most-`max_bytes` UTF-8 prefix of `text`, cut on a char boundary,
/// plus whether anything was cut.
#[must_use]
pub fn bounded_utf8_prefix(text: &str, max_bytes: usize) -> (&str, bool) {
    if text.len() <= max_bytes {
        return (text, false);
    }
    let mut end = max_bytes.min(text.len());
    while end != 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], true)
}

/// A single capped preview row for input too large to parse.
#[must_use]
pub fn oversized_structured_preview(output: &str) -> FormattedOutputRows {
    let key = "Output preview".to_string();
    let value_budget = MAX_STRUCTURED_OUTPUT_BYTES.saturating_sub(key.len());
    let (preview, _) = bounded_utf8_prefix(output, value_budget);
    FormattedOutputRows {
        byte_len: key.len().saturating_add(preview.len()),
        rows: vec![(key, preview.to_string())],
        truncated: true,
    }
}

/// Flatten a task's output into human-facing key/value rows ("group · key").
/// JSON objects and arrays flatten; non-JSON output returns `None` (the
/// caller renders it as raw text). Parsing and row materialization are
/// bounded before touching the complete collector payload.
#[must_use]
pub fn format_output_key_values(task_id: &str, output: &str) -> Option<FormattedOutputRows> {
    let oversized = output.len() > MAX_STRUCTURED_OUTPUT_INPUT_BYTES;
    let candidate = if oversized {
        bounded_utf8_prefix(output, MAX_STRUCTURED_OUTPUT_INPUT_BYTES).0
    } else {
        output
    };
    // Collector output decoded from PowerShell may carry a leading BOM
    // (U+FEFF), which str::trim does not remove; strip it explicitly.
    let trimmed = candidate.trim().trim_start_matches('\u{feff}').trim();
    if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
        return None;
    }
    if oversized {
        return Some(oversized_structured_preview(trimmed));
    }

    let value: serde_json::Value = serde_json::from_str(trimmed).ok()?;
    // pending_reboot's raw schema needs the same explanation as the
    // shipping detail view: expose restart state instead of raw flags.
    if task_id == "pending_reboot" {
        let Some(map) = value.as_object() else {
            return Some(FormattedOutputRows::new());
        };
        return Some(pending_reboot_rows(map));
    }
    let mut rows = FormattedOutputRows::new();
    flatten_json("", &value, &mut rows);
    (!rows.rows.is_empty() || rows.truncated).then_some(rows)
}

/// Recursively flatten JSON into bounded rows, mirroring the shipping detail
/// view: arrays flatten through Object.entries semantics — each item gets an
/// index path ("0 · key"), scalars join inline.
fn flatten_json(prefix: &str, value: &serde_json::Value, rows: &mut FormattedOutputRows) {
    if rows.truncated {
        return;
    }
    match value {
        serde_json::Value::Object(map) => {
            for (key, entry) in map {
                if rows.truncated {
                    break;
                }
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix} · {key}")
                };
                flatten_json(&path, entry, rows);
            }
        }
        serde_json::Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                if rows.truncated {
                    break;
                }
                let path = if prefix.is_empty() {
                    index.to_string()
                } else {
                    format!("{prefix} · {index}")
                };
                match item {
                    serde_json::Value::String(text) => {
                        rows.push_str(path, text);
                    }
                    serde_json::Value::Number(number) => {
                        rows.push(path, number.to_string());
                    }
                    serde_json::Value::Bool(flag) => {
                        rows.push(path, flag.to_string());
                    }
                    other => flatten_json(&path, other, rows),
                }
            }
        }
        serde_json::Value::Null => {
            rows.push(prefix.to_string(), String::new());
        }
        serde_json::Value::String(text) => {
            rows.push_str(prefix.to_string(), text);
        }
        other => rows.push(prefix.to_string(), other.to_string()),
    }
}

/// Project `pending_reboot`'s schema: restart state and the reasons that
/// carry it, instead of raw flags.
fn pending_reboot_rows(map: &serde_json::Map<String, serde_json::Value>) -> FormattedOutputRows {
    let mut rows = FormattedOutputRows::new();
    let reasons: Vec<String> = map
        .get("reasons")
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or_default();
    let high_confidence = reasons
        .iter()
        .any(|reason| reason == "windows_update" || reason == "component_based_servicing");
    let legacy_deferred = reasons
        .iter()
        .any(|reason| reason == "pending_file_rename" || reason == "pending_file_operations");
    let explicit = map
        .get("restart_required")
        .and_then(serde_json::Value::as_bool);
    let legacy = map.get("pending").and_then(serde_json::Value::as_bool);
    let restart_required = explicit.unwrap_or(match legacy {
        Some(legacy_pending) => legacy_pending && (high_confidence || legacy_deferred),
        None => false,
    });
    rows.push(
        "Restart required".to_string(),
        if restart_required { "Yes" } else { "No" }.to_string(),
    );
    if !reasons.is_empty() {
        let required_by: Vec<&str> = reasons
            .iter()
            .flat_map(|reason| match reason.as_str() {
                "windows_update" => vec!["Windows Update"],
                "component_based_servicing" => vec!["Windows component servicing"],
                _ => Vec::new(),
            })
            .collect();
        if !required_by.is_empty() {
            rows.push("Required by".to_string(), required_by.join(", "));
        }
    }
    for (key, entry) in map {
        if rows.truncated {
            break;
        }
        if key == "restart_required" || key == "pending" || key == "reasons" {
            continue;
        }
        let path = key.clone();
        flatten_json(&path, entry, &mut rows);
    }
    rows
}

/// The scalar facts the raw document view needs from one task result. The
/// shell builds this from its own task types; keeping them here as scalars
/// keeps this crate free of UI types.
pub struct RawDocumentInput<'a> {
    pub task_id: &'a str,
    /// The task's display name, or the task id when unknown.
    pub task_name: &'a str,
    /// The task's category, or `"Other"` when unknown.
    pub category: &'a str,
    pub success: bool,
    pub duration_ms: u64,
    pub admin_required: bool,
    pub error: Option<&'a str>,
    pub output: &'a str,
}

/// The copyable raw document for one task result: metadata plus the output,
/// pretty-printed as JSON when it parses and bounded when it does not.
///
/// # Panics
///
/// Only if `serde_json` cannot serialize its own `Value` output, which its
/// constructors guarantee does not happen for these value shapes.
#[must_use]
pub fn diagnostic_raw_document(input: &RawDocumentInput<'_>) -> String {
    let source_is_bounded = input.output.len() <= MAX_RAW_PRETTY_INPUT_BYTES;
    let output = if source_is_bounded {
        let trimmed = input.output.trim().trim_start_matches('\u{feff}').trim();
        if trimmed.starts_with('{') || trimmed.starts_with('[') {
            serde_json::from_str::<serde_json::Value>(trimmed)
                .unwrap_or_else(|_| serde_json::Value::String(input.output.to_string()))
        } else {
            serde_json::Value::String(input.output.to_string())
        }
    } else {
        let (source_preview, _) = bounded_utf8_prefix(input.output, MAX_RAW_FALLBACK_BYTES);
        let mut preview = String::with_capacity(
            source_preview
                .len()
                .saturating_add(RAW_OUTPUT_TRUNCATION_NOTICE.len())
                .saturating_add(2),
        );
        preview.push_str(source_preview);
        preview.push_str("\n\n");
        preview.push_str(RAW_OUTPUT_TRUNCATION_NOTICE);
        serde_json::Value::String(preview)
    };
    let mut document = serde_json::json!({
        "task_id": input.task_id,
        "name": input.task_name,
        "category": input.category,
        "success": input.success,
        "duration_ms": input.duration_ms,
        "admin_required": input.admin_required,
        "error": input.error,
        "output": output,
    });
    if !source_is_bounded {
        document
            .as_object_mut()
            .expect("diagnostic raw document is an object")
            .insert(
                "output_truncated".to_string(),
                serde_json::Value::Bool(true),
            );
    }
    serde_json::to_string_pretty(&document)
        .expect("diagnostic raw document contains only serializable values")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_output_flattens_to_key_values() {
        let rows = format_output_key_values(
            "processor",
            r#"{"Name":"Snapdragon X","NumberOfCores":12,"_DERIVATION":["A","B"]}"#,
        )
        .expect("JSON object must produce rows");
        assert!(!rows.truncated);
        assert!(
            rows.rows
                .contains(&("Name".to_string(), "Snapdragon X".to_string())),
            "rows were: {rows:?}"
        );
        assert!(
            rows.rows
                .contains(&("NumberOfCores".to_string(), "12".to_string()))
        );
    }

    #[test]
    fn json_output_with_bom_still_parses() {
        let with_bom = "\u{feff}{\"Name\":\"X\"}".to_string();
        let rows = format_output_key_values("os_info", &with_bom).expect("BOM JSON parses");
        assert_eq!(rows.rows, [("Name".to_string(), "X".to_string())]);
        assert!(!rows.truncated);
    }

    #[test]
    fn non_json_output_stays_raw() {
        assert!(format_output_key_values("os_info", "plain text output").is_none());
    }

    #[test]
    fn structured_output_caps_rows_and_bytes() {
        let output = serde_json::to_string(
            &(0..MAX_STRUCTURED_OUTPUT_ROWS + 10)
                .map(|index| format!("value-{index}"))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        let rows = format_output_key_values("large_array", &output).unwrap();
        assert_eq!(rows.rows.len(), MAX_STRUCTURED_OUTPUT_ROWS);
        assert!(rows.byte_len <= MAX_STRUCTURED_OUTPUT_BYTES);
        assert!(rows.truncated);

        let oversized_value = serde_json::json!({
            "blob": "x".repeat(MAX_STRUCTURED_OUTPUT_BYTES)
        })
        .to_string();
        let rows = format_output_key_values("large_value", &oversized_value).unwrap();
        assert!(rows.rows.is_empty());
        assert!(rows.byte_len <= MAX_STRUCTURED_OUTPUT_BYTES);
        assert!(rows.truncated);
    }

    #[test]
    fn oversized_structured_input_uses_a_bounded_preview_without_parsing() {
        let output = format!(
            "{{\"payload\":\"{}\",\"tail\":not-valid-json}}",
            "x".repeat(MAX_STRUCTURED_OUTPUT_INPUT_BYTES)
        );
        let rows = format_output_key_values("oversized", &output).unwrap();
        assert_eq!(rows.rows.len(), 1);
        assert!(rows.byte_len <= MAX_STRUCTURED_OUTPUT_BYTES);
        assert!(rows.truncated);
        assert!(!rows.rows[0].1.contains("not-valid-json"));
    }

    #[test]
    fn raw_document_preserves_metadata_and_marks_truncation() {
        let document = diagnostic_raw_document(&RawDocumentInput {
            task_id: "os_info",
            task_name: "Operating System",
            category: "System",
            success: true,
            duration_ms: 42,
            admin_required: true,
            error: None,
            output: r#"{"build":26100,"secure":true}"#,
        });
        let value: serde_json::Value = serde_json::from_str(&document).unwrap();
        assert_eq!(value["task_id"], "os_info");
        assert_eq!(value["name"], "Operating System");
        assert_eq!(value["output"]["build"], 26100);
        assert!(value.get("output_truncated").is_none());

        let oversized_output = format!(
            "{}RAW_TAIL_SENTINEL",
            "x".repeat(MAX_RAW_PRETTY_INPUT_BYTES + 100)
        );
        let oversized = diagnostic_raw_document(&RawDocumentInput {
            task_id: "large",
            task_name: "large",
            category: "Other",
            success: true,
            duration_ms: 1,
            admin_required: false,
            error: None,
            output: &oversized_output,
        });
        let value: serde_json::Value = serde_json::from_str(&oversized).unwrap();
        assert!(value["output_truncated"].as_bool().unwrap());
        let raw_output = value["output"].as_str().unwrap();
        assert!(raw_output.contains(RAW_OUTPUT_TRUNCATION_NOTICE));
        assert!(!raw_output.contains("RAW_TAIL_SENTINEL"));
    }
}
