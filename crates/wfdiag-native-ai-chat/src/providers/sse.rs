//! Thin SSE helper over `eventsource-stream` for the native Anthropic and
//! Gemini clients. Handles chunk-boundary reassembly; comment/ping lines are
//! dropped by the parser.

use eventsource_stream::Eventsource;
use futures::StreamExt;

/// Bound raw transport accumulation, including partial SSE frames, tool
/// arguments and provider replay/thinking blocks, before JSON assembly.
pub(crate) const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

pub(crate) fn charge_response_bytes(used: &mut usize, additional: usize) -> Result<(), String> {
    *used = used.saturating_add(additional);
    if *used > MAX_RESPONSE_BYTES {
        Err("AI response exceeded the local memory budget".to_string())
    } else {
        Ok(())
    }
}

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
    let mut received = 0;
    let bytes = response.bytes_stream().map(move |chunk| {
        let chunk = chunk.map_err(|error| error.to_string())?;
        charge_response_bytes(&mut received, chunk.len())?;
        Ok::<_, String>(chunk)
    });
    let mut stream = bytes.eventsource();
    while let Some(event) = stream.next().await {
        let event = event.map_err(|e| format!("stream error: {e}"))?;
        if !on_event(&event.event, &event.data)? {
            break;
        }
        tokio::task::yield_now().await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn response_budget_rejects_oversized_chunks_and_accumulation() {
        let mut used = 0;
        assert!(charge_response_bytes(&mut used, MAX_RESPONSE_BYTES).is_ok());
        assert!(charge_response_bytes(&mut used, 1).is_err());
        assert!(charge_response_bytes(&mut used, usize::MAX).is_err());
    }
}
