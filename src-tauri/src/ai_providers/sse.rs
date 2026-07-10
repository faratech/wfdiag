//! Thin SSE helper over `eventsource-stream` for the native Anthropic and
//! Gemini clients. Handles chunk-boundary reassembly; comment/ping lines are
//! dropped by the parser.

use eventsource_stream::Eventsource;
use futures::StreamExt;

/// Drive an SSE response, invoking `on_event(event_name, data)` per event.
/// Stops cleanly when the stream ends or `on_event` returns `false`
/// (e.g. a terminal event was seen). Transport errors are returned.
///
/// Callers typically forward text deltas to a bounded mpsc channel with
/// `try_send` while racing this future against the receiver in a
/// `tokio::select!` loop. If several SSE frames are already buffered,
/// `stream.next().await` can resolve immediately many times in a row within
/// a single poll of this future, so the consumer never gets scheduled and
/// `try_send` starts silently dropping deltas once the channel fills up. The
/// explicit yield after every event guarantees the surrounding `select!`
/// re-polls its other arms (the channel receiver) between events, so the
/// channel never has a chance to back up in the first place.
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
        tokio::task::yield_now().await;
    }
    Ok(())
}
