//! Pure JSON leaf-diff projection for the native History screen.
//!
//! This mirrors the observable contract of the shipping React
//! `useJsonDiff.ts` hook: malformed input has no structured projection,
//! objects use dot paths, arrays use bracketed indexes, null changes are
//! modifications, and type changes use JavaScript `typeof` names. The raw
//! Previous/Current payloads remain the caller's responsibility.

#![deny(unsafe_code)]

use serde_json::{Map, Value};
use std::collections::HashSet;

/// The shipping History screen shows at most twelve structured changes before
/// its "more changes" summary.
pub const MAX_VISIBLE_JSON_DIFFERENCES: usize = 12;

/// Closed set of leaf-change categories emitted by the shipping JSON diff.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JsonDifferenceKind {
    Added,
    Removed,
    Modified,
    TypeChanged,
}

/// One addressable JSON change.
///
/// Added values have no `old_value`; removed values have no `new_value`.
/// Modified and type-changed values carry both sides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsonDifference {
    pub path: String,
    pub kind: JsonDifferenceKind,
    pub old_value: Option<Value>,
    pub new_value: Option<Value>,
}

impl JsonDifference {
    #[must_use]
    pub fn formatted(&self) -> String {
        format_difference(self)
    }
}

/// Parse and compare two JSON strings.
///
/// `None` deliberately means at least one side was not valid JSON, matching the
/// React hook's raw-output-only fallback. `Some(Vec::new())` means both inputs
/// were valid JSON and had no observable leaf differences.
#[must_use]
pub fn find_json_differences(
    previous_json: &str,
    current_json: &str,
) -> Option<Vec<JsonDifference>> {
    let previous = serde_json::from_str::<Value>(previous_json).ok()?;
    let current = serde_json::from_str::<Value>(current_json).ok()?;
    Some(compare_json_values(&previous, &current))
}

/// Compare two already-parsed JSON values with the shipping hook's path and
/// classification rules.
#[must_use]
pub fn compare_json_values(previous: &Value, current: &Value) -> Vec<JsonDifference> {
    let mut differences = Vec::new();
    compare_values(Some(previous), Some(current), "", &mut differences);
    differences
}

/// Format one difference exactly as the shipping History screen does.
#[must_use]
pub fn format_difference(difference: &JsonDifference) -> String {
    match difference.kind {
        JsonDifferenceKind::Added => format!(
            "Added: {} = {}",
            difference.path,
            stringify_optional(difference.new_value.as_ref())
        ),
        JsonDifferenceKind::Removed => format!(
            "Removed: {} = {}",
            difference.path,
            stringify_optional(difference.old_value.as_ref())
        ),
        JsonDifferenceKind::Modified => format!(
            "Changed: {} from {} to {}",
            difference.path,
            stringify_optional(difference.old_value.as_ref()),
            stringify_optional(difference.new_value.as_ref())
        ),
        JsonDifferenceKind::TypeChanged => format!(
            "Type changed: {} from {} to {}",
            difference.path,
            javascript_type_optional(difference.old_value.as_ref()),
            javascript_type_optional(difference.new_value.as_ref())
        ),
    }
}

/// Split a complete diff into the shipping screen's visible prefix and hidden
/// count. Invalid JSON is represented before this boundary by `None`.
#[must_use]
pub fn visible_differences(differences: &[JsonDifference]) -> (&[JsonDifference], usize) {
    let visible_count = differences.len().min(MAX_VISIBLE_JSON_DIFFERENCES);
    (
        &differences[..visible_count],
        differences.len() - visible_count,
    )
}

fn compare_values(
    previous: Option<&Value>,
    current: Option<&Value>,
    path: &str,
    differences: &mut Vec<JsonDifference>,
) {
    let (Some(previous), Some(current)) = (previous, current) else {
        let (kind, old_value, new_value) = match (previous, current) {
            (None, Some(current)) => (JsonDifferenceKind::Added, None, Some(current.clone())),
            (Some(previous), None) => (JsonDifferenceKind::Removed, Some(previous.clone()), None),
            (None, None) => return,
            (Some(_), Some(_)) => unreachable!("both values were handled above"),
        };
        differences.push(JsonDifference {
            path: root_path(path),
            kind,
            old_value,
            new_value,
        });
        return;
    };

    if javascript_strict_equal(previous, current) {
        return;
    }

    // The React hook classifies every null/non-null transition as modified,
    // even when JavaScript `typeof` would otherwise differ.
    if previous.is_null() || current.is_null() {
        differences.push(modified(path, previous, current));
        return;
    }

    if javascript_type(previous) != javascript_type(current) {
        differences.push(JsonDifference {
            path: root_path(path),
            kind: JsonDifferenceKind::TypeChanged,
            old_value: Some(previous.clone()),
            new_value: Some(current.clone()),
        });
        return;
    }

    if let (Value::Array(previous), Value::Array(current)) = (previous, current) {
        let count = previous.len().max(current.len());
        for index in 0..count {
            let current_path = format!("{path}[{index}]");
            compare_values(
                previous.get(index),
                current.get(index),
                &current_path,
                differences,
            );
        }
        return;
    }

    // JavaScript reports both arrays and ordinary objects as `typeof object`.
    // Consequently an array-vs-object pair reaches the object-key walk rather
    // than becoming a type-changed leaf. Preserve that slightly surprising but
    // user-visible shipping behavior.
    if is_object_like(previous) && is_object_like(current) {
        for key in union_object_keys(previous, current) {
            let current_path = if path.is_empty() {
                key.clone()
            } else {
                format!("{path}.{key}")
            };
            compare_values(
                object_like_get(previous, &key),
                object_like_get(current, &key),
                &current_path,
                differences,
            );
        }
        return;
    }

    differences.push(modified(path, previous, current));
}

// This clones the whole `previous`/`current` subtree at a recorded
// difference node (here, and at the type-changed and added/removed branches
// above). `compare_values` only reaches this without recursing further when
// the values are leaves, a null/non-null transition, or a type change — a
// case where per-field decomposition wouldn't be meaningful anyway, so
// something has to be cloned whole to show the user what changed. This runs
// on a user-triggered history comparison, not a render-loop hot path, so the
// bound (however large the one differing subtree is) is acceptable.
fn modified(path: &str, previous: &Value, current: &Value) -> JsonDifference {
    JsonDifference {
        path: root_path(path),
        kind: JsonDifferenceKind::Modified,
        old_value: Some(previous.clone()),
        new_value: Some(current.clone()),
    }
}

fn root_path(path: &str) -> String {
    if path.is_empty() {
        "root".to_string()
    } else {
        path.to_string()
    }
}

fn javascript_strict_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(left), Value::Bool(right)) => left == right,
        (Value::Number(left), Value::Number(right)) => left
            .as_f64()
            .zip(right.as_f64())
            .is_some_and(|(left, right)| left == right),
        (Value::String(left), Value::String(right)) => left == right,
        // Independently parsed arrays/objects are never reference-equal in the
        // React hook. Recursion below establishes structural equality instead.
        _ => false,
    }
}

fn javascript_type(value: &Value) -> &'static str {
    match value {
        Value::Null | Value::Array(_) | Value::Object(_) => "object",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
    }
}

fn javascript_type_optional(value: Option<&Value>) -> &'static str {
    value.map_or("undefined", javascript_type)
}

fn is_object_like(value: &Value) -> bool {
    matches!(value, Value::Array(_) | Value::Object(_))
}

fn union_object_keys(previous: &Value, current: &Value) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut keys = Vec::new();
    for key in object_like_keys(previous)
        .into_iter()
        .chain(object_like_keys(current))
    {
        if seen.insert(key.clone()) {
            keys.push(key);
        }
    }
    keys
}

fn object_like_keys(value: &Value) -> Vec<String> {
    match value {
        Value::Array(values) => (0..values.len()).map(|index| index.to_string()).collect(),
        Value::Object(values) => javascript_object_keys(values),
        _ => Vec::new(),
    }
}

fn object_like_get<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Array(values) => javascript_array_index(key)
            .and_then(|index| usize::try_from(index).ok())
            .and_then(|index| values.get(index)),
        Value::Object(values) => values.get(key),
        _ => None,
    }
}

/// `Object.keys` puts canonical array-index property names first in ascending
/// order, followed by the remaining keys in insertion order.
fn javascript_object_keys(values: &Map<String, Value>) -> Vec<String> {
    let mut indexes = Vec::new();
    let mut ordinary = Vec::new();
    for key in values.keys() {
        if let Some(index) = javascript_array_index(key) {
            indexes.push((index, key.clone()));
        } else {
            ordinary.push(key.clone());
        }
    }
    indexes.sort_unstable_by_key(|(index, _)| *index);
    indexes
        .into_iter()
        .map(|(_, key)| key)
        .chain(ordinary)
        .collect()
}

fn javascript_array_index(key: &str) -> Option<u32> {
    if key == "0" {
        return Some(0);
    }
    if key.is_empty() || key.starts_with('0') {
        return None;
    }
    let index = key.parse::<u32>().ok()?;
    (index != u32::MAX && index.to_string() == key).then_some(index)
}

fn stringify_optional(value: Option<&Value>) -> String {
    value.map_or_else(|| "undefined".to_string(), javascript_stringify)
}

/// JSON.stringify-compatible rendering for parsed JSON values.
///
/// In particular, numbers are compared and rendered as JavaScript `Number`
/// values rather than preserving serde_json's wider integer representation.
fn javascript_stringify(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value
            .as_f64()
            .map_or_else(|| "null".to_string(), javascript_number_to_string),
        Value::String(value) => {
            serde_json::to_string(value).expect("serializing a JSON string cannot fail")
        }
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(javascript_stringify)
                .collect::<Vec<_>>()
                .join(",")
        ),
        Value::Object(values) => {
            let entries = javascript_object_keys(values)
                .into_iter()
                .filter_map(|key| {
                    values.get(&key).map(|value| {
                        let key = serde_json::to_string(&key)
                            .expect("serializing a JSON object key cannot fail");
                        format!("{key}:{}", javascript_stringify(value))
                    })
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{entries}}}")
        }
    }
}

/// Format a finite f64 with the notation thresholds used by ECMAScript's
/// Number-to-string operation. Rust's shortest decimal supplies the digits;
/// this function only applies the JavaScript fixed/scientific layout rules.
fn javascript_number_to_string(value: f64) -> String {
    if !value.is_finite() {
        return "null".to_string();
    }
    if value == 0.0 {
        return "0".to_string();
    }

    let negative = value.is_sign_negative();
    let decimal = value.abs().to_string();
    let (whole, fraction) = decimal
        .split_once('.')
        .map_or((decimal.as_str(), ""), |(whole, fraction)| {
            (whole, fraction)
        });
    let mut digits = format!("{whole}{fraction}");
    let mut scale = -i32::try_from(fraction.len()).expect("f64 decimal length fits in i32");

    let first_nonzero = digits.find(|character| character != '0').unwrap_or(0);
    digits.drain(..first_nonzero);
    while digits.ends_with('0') {
        digits.pop();
        scale += 1;
    }

    let digit_count = i32::try_from(digits.len()).expect("f64 decimal length fits in i32");
    let decimal_position = digit_count + scale;
    let magnitude = if decimal_position > 0 && decimal_position <= 21 {
        if digit_count <= decimal_position {
            let zero_count = usize::try_from(decimal_position - digit_count)
                .expect("non-negative fixed-decimal padding");
            format!("{digits}{}", "0".repeat(zero_count))
        } else {
            let split = usize::try_from(decimal_position).expect("positive decimal position");
            format!("{}.{}", &digits[..split], &digits[split..])
        }
    } else if decimal_position <= 0 && decimal_position > -6 {
        let zero_count =
            usize::try_from(-decimal_position).expect("non-negative fractional zero padding");
        format!("0.{}{digits}", "0".repeat(zero_count))
    } else {
        let mut scientific = digits[..1].to_string();
        if digits.len() > 1 {
            scientific.push('.');
            scientific.push_str(&digits[1..]);
        }
        let exponent = decimal_position - 1;
        if exponent >= 0 {
            format!("{scientific}e+{exponent}")
        } else {
            format!("{scientific}e{exponent}")
        }
    };

    if negative {
        format!("-{magnitude}")
    } else {
        magnitude
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed_differences(previous: &str, current: &str) -> Vec<JsonDifference> {
        find_json_differences(previous, current).expect("fixtures must be valid JSON")
    }

    #[test]
    fn malformed_input_uses_the_raw_output_fallback() {
        assert_eq!(find_json_differences("{bad json", "{}"), None);
        assert_eq!(find_json_differences("{}", "[unterminated"), None);
    }

    #[test]
    fn valid_identical_json_is_distinct_from_invalid_json() {
        assert_eq!(find_json_differences("null", "null"), Some(Vec::new()));
        assert_eq!(
            find_json_differences(
                r#"{"system":{"cpu":8},"items":[true,null]}"#,
                r#"{"system":{"cpu":8},"items":[true,null]}"#,
            ),
            Some(Vec::new())
        );
    }

    #[test]
    fn nested_object_changes_keep_shipping_paths_order_and_categories() {
        let differences = parsed_differences(
            r#"{"system":{"cpu":{"cores":8},"name":"old"},"removed":{"x":1}}"#,
            r#"{"system":{"cpu":{"cores":12},"name":"old","new":true},"added":[1,2]}"#,
        );

        assert_eq!(
            differences,
            vec![
                JsonDifference {
                    path: "system.cpu.cores".to_string(),
                    kind: JsonDifferenceKind::Modified,
                    old_value: Some(Value::from(8)),
                    new_value: Some(Value::from(12)),
                },
                JsonDifference {
                    path: "system.new".to_string(),
                    kind: JsonDifferenceKind::Added,
                    old_value: None,
                    new_value: Some(Value::Bool(true)),
                },
                JsonDifference {
                    path: "removed".to_string(),
                    kind: JsonDifferenceKind::Removed,
                    old_value: Some(serde_json::json!({"x": 1})),
                    new_value: None,
                },
                JsonDifference {
                    path: "added".to_string(),
                    kind: JsonDifferenceKind::Added,
                    old_value: None,
                    new_value: Some(serde_json::json!([1, 2])),
                },
            ]
        );
    }

    #[test]
    fn arrays_use_bracket_paths_and_whole_values_for_added_or_removed_slots() {
        let differences = parsed_differences(
            r#"[1,{"state":"old"},3,4]"#,
            r#"[1,{"state":"new"},3,{"replacement":true},5]"#,
        );

        assert_eq!(
            differences
                .iter()
                .map(|difference| (difference.path.as_str(), difference.kind))
                .collect::<Vec<_>>(),
            vec![
                ("[1].state", JsonDifferenceKind::Modified),
                ("[3]", JsonDifferenceKind::TypeChanged),
                ("[4]", JsonDifferenceKind::Added),
            ]
        );
        assert_eq!(differences[2].new_value, Some(Value::from(5)));

        let removed = parsed_differences("[1,2]", "[1]");
        assert_eq!(removed[0].path, "[1]");
        assert_eq!(removed[0].kind, JsonDifferenceKind::Removed);
        assert_eq!(removed[0].old_value, Some(Value::from(2)));
    }

    #[test]
    fn null_transitions_are_modified_before_type_classification() {
        let differences = parsed_differences("null", r#"{"ready":true}"#);
        assert_eq!(differences[0].kind, JsonDifferenceKind::Modified);
        assert_eq!(differences[0].path, "root");
        assert_eq!(
            differences[0].formatted(),
            r#"Changed: root from null to {"ready":true}"#
        );
    }

    #[test]
    fn primitive_type_changes_use_javascript_typeof_names() {
        let differences = parsed_differences("true", "42");
        assert_eq!(differences[0].kind, JsonDifferenceKind::TypeChanged);
        assert_eq!(
            differences[0].formatted(),
            "Type changed: root from boolean to number"
        );
    }

    #[test]
    fn array_to_object_uses_the_shipping_object_key_walk() {
        let differences = parsed_differences(r#"[1]"#, r#"{"0":2,"extra":true}"#);
        assert_eq!(
            differences
                .iter()
                .map(|difference| (difference.path.as_str(), difference.kind))
                .collect::<Vec<_>>(),
            vec![
                ("0", JsonDifferenceKind::Modified),
                ("extra", JsonDifferenceKind::Added),
            ]
        );
    }

    #[test]
    fn javascript_object_key_order_puts_array_indexes_first() {
        let differences = parsed_differences(
            r#"{"b":0,"2":0,"1":0,"a":0,"01":0,"4294967294":0,"4294967295":0}"#,
            r#"{"b":1,"2":1,"1":1,"a":1,"01":1,"4294967294":1,"4294967295":1}"#,
        );
        assert_eq!(
            differences
                .iter()
                .map(|difference| difference.path.as_str())
                .collect::<Vec<_>>(),
            vec!["1", "2", "4294967294", "b", "a", "01", "4294967295"]
        );
    }

    #[test]
    fn paths_are_intentionally_unescaped_like_the_react_hook() {
        let differences = parsed_differences(r#"{"a.b":{"x":1}}"#, r#"{"a.b":{"x":2}}"#);
        assert_eq!(differences[0].path, "a.b.x");
    }

    #[test]
    fn formatting_matches_added_removed_and_modified_strings() {
        let differences = parsed_differences(
            r#"{"old":"gone","changed":"before"}"#,
            r#"{"changed":"after","new":{"ok":true}}"#,
        );
        assert_eq!(differences[0].formatted(), r#"Removed: old = "gone""#);
        assert_eq!(
            differences[1].formatted(),
            r#"Changed: changed from "before" to "after""#
        );
        assert_eq!(differences[2].formatted(), r#"Added: new = {"ok":true}"#);
    }

    #[test]
    fn numbers_follow_javascript_precision_and_stringification() {
        // JSON.parse rounds both integers to the same JavaScript Number.
        assert!(parsed_differences("9007199254740992", "9007199254740993").is_empty());
        assert!(parsed_differences("-0", "0").is_empty());
        assert!(parsed_differences("1", "1.0").is_empty());

        assert_eq!(javascript_number_to_string(1e20), "100000000000000000000");
        assert_eq!(javascript_number_to_string(1e21), "1e+21");
        assert_eq!(javascript_number_to_string(1e-6), "0.000001");
        assert_eq!(javascript_number_to_string(1e-7), "1e-7");
        assert_eq!(javascript_number_to_string(-0.0), "0");

        let changed = parsed_differences("9007199254740993", r#""large""#);
        assert_eq!(
            changed[0].formatted(),
            "Type changed: root from number to string"
        );
        let added = parsed_differences("{}", r#"{"large":9007199254740993}"#);
        assert_eq!(added[0].formatted(), "Added: large = 9007199254740992");
    }

    #[test]
    fn visible_projection_is_capped_at_twelve_with_an_exact_hidden_count() {
        let previous = format!(
            "[{}]",
            (0..15)
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );
        let current = format!(
            "[{}]",
            (100..115)
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );
        let differences = parsed_differences(&previous, &current);
        let (visible, hidden) = visible_differences(&differences);

        assert_eq!(visible.len(), MAX_VISIBLE_JSON_DIFFERENCES);
        assert_eq!(hidden, 3);
        assert_eq!(
            visible.first().map(|difference| difference.path.as_str()),
            Some("[0]")
        );
        assert_eq!(
            visible.last().map(|difference| difference.path.as_str()),
            Some("[11]")
        );

        let (visible, hidden) = visible_differences(&differences[..2]);
        assert_eq!(visible.len(), 2);
        assert_eq!(hidden, 0);
    }
}
