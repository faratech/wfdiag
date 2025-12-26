//! AI Prompt Templates
//!
//! Contains prompt templates for different AI analysis scenarios.

/// Generate prompt for single diagnostic interpretation
pub fn diagnostic_interpretation_prompt(task_name: &str, output: &str) -> String {
    format!(
        r#"Analyze this Windows diagnostic output and provide a brief interpretation.

**Diagnostic:** {task_name}

**Output:**
{output}

Provide:
1. A 1-2 sentence summary of what this data shows
2. Any notable concerns or anomalies (if any)
3. A brief recommendation if action is needed

Keep the response under 100 words. Be direct and technical but accessible."#,
        task_name = task_name,
        output = truncate_output(output, 3000)
    )
}

/// Generate prompt for section summary (Hardware, System, Storage, Network)
pub fn section_summary_prompt(section_name: &str, section_data: &str) -> String {
    format!(
        r#"Summarize these {section_name} diagnostics for a Windows system.

**Diagnostic Results:**
{section_data}

Provide:
1. Overall status assessment (healthy/needs attention/critical)
2. Key findings in 2-3 bullet points
3. Top recommendation if any issues found

Keep the response under 150 words. Focus on actionable insights."#,
        section_name = section_name,
        section_data = truncate_output(section_data, 6000)
    )
}

/// Generate prompt for health score explanation
pub fn health_explanation_prompt(metrics_data: &str) -> String {
    format!(
        r#"Explain these Windows system health metrics to a user.

**Health Metrics:**
{metrics_data}

Provide:
1. Why the overall score is what it is
2. Which components are affecting the score most
3. What could be done to improve the score

Keep the response under 120 words. Be encouraging but honest."#,
        metrics_data = metrics_data
    )
}

/// Generate prompt for issue prioritization
pub fn issue_prioritization_prompt(issues_data: &str) -> String {
    format!(
        r#"Prioritize these Windows system issues by importance and impact.

**Detected Issues:**
{issues_data}

Provide:
1. Issues ranked by priority (highest first)
2. Brief reason for each ranking
3. Which issue to address first and why

Keep the response under 150 words. Focus on practical impact."#,
        issues_data = truncate_output(issues_data, 4000)
    )
}

/// Truncate output to fit within context limits
fn truncate_output(output: &str, max_chars: usize) -> String {
    if output.len() <= max_chars {
        output.to_string()
    } else {
        format!(
            "{}... [truncated, {} more characters]",
            &output[..max_chars],
            output.len() - max_chars
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
        // Try to keep first part (usually headers/summary) and last part (usually important)
        let half = max_chars / 2;
        let first_part = &compressed[..half];
        let last_part = &compressed[compressed.len() - half..];

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
        assert!(truncated.len() < 200);
        assert!(truncated.contains("[truncated"));
    }

    #[test]
    fn test_compress_for_phi_silica() {
        let verbose = "line1\n\n\n  line2  \n\n  line3  ";
        let compressed = compress_for_phi_silica(verbose, 1000);
        assert_eq!(compressed, "line1\nline2\nline3");
    }
}
