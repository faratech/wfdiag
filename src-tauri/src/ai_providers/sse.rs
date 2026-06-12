//! Thin SSE helper over `eventsource-stream` for the native Anthropic and
//! Gemini clients. Handles chunk-boundary reassembly; comment/ping lines are
//! dropped by the parser.

use eventsource_stream::Eventsource;
use futures::StreamExt;

/// Drive an SSE response, invoking `on_event(event_name, data)` per event.
/// Stops cleanly when the stream ends or `on_event` returns `false`
/// (e.g. a terminal event was seen). Transport errors are returned.
pub(crate) async fn for_each_event<F>(
    response: reqwest::Response,
    mut on_event: F,
) -> Result<(), String>
where
    F: FnMut(&str, &str) -> Result<bool, String>,
{
    let mut stream = response.bytes_stream().eventsource();
    while let Some(event) = stream.next().await {
        let event = event.map_err(|e| format!("stream error: {}", e))?;
        if !on_event(&event.event, &event.data)? {
            break;
        }
    }
    Ok(())
}
