//! Bounded delta accumulation and final-result reconciliation.
const MAX_BYTES: usize = 1024 * 1024;

#[derive(Default)]
pub(crate) struct StreamReconciler {
    text: String,
    sent: usize,
    blocked: bool,
    error: Option<String>,
}

impl StreamReconciler {
    pub(crate) fn failed(&self) -> bool {
        self.error.is_some()
    }

    pub(crate) fn forward(
        &mut self,
        delta: &str,
        mut send: impl FnMut(String) -> Result<bool, String>,
    ) {
        if self.failed() {
            return;
        }
        if self.text.len().saturating_add(delta.len()) > MAX_BYTES {
            self.error = Some("Aion response exceeded the stream limit".into());
            return;
        }
        self.text.push_str(delta);
        // While the consumer queue is full, only accumulate: re-sending
        // `text[sent..]` on every delta would recopy a growing tail
        // (quadratic total bytes). `finish` delivers the remainder.
        if self.blocked {
            return;
        }
        match send(self.text[self.sent..].to_string()) {
            Ok(true) => self.sent = self.text.len(),
            Ok(false) => self.blocked = true, // queue full: coalesce until finish
            Err(error) => self.error = Some(error),
        }
    }

    pub(crate) fn finish(
        &mut self,
        text: &str,
        mut send: impl FnMut(String) -> Result<bool, String>,
    ) -> Result<(), String> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        if text.len() > MAX_BYTES {
            return Err("Aion response exceeded the stream limit".into());
        }
        if !text.starts_with(&self.text) {
            return Err("Aion final response differs from its streamed prefix".into());
        }
        if text.len() > self.sent && !send(text[self.sent..].to_string())? {
            return Err("Aion response consumer is not accepting text".into());
        }
        self.text = text.to_string();
        self.sent = text.len();
        self.blocked = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backpressure_coalesces_without_losing_or_repeating_unicode() {
        let mut state = StreamReconciler::default();
        let mut received = String::new();
        state.forward("α", |_| Ok(false));
        state.forward("β", |s| {
            received.push_str(&s);
            Ok(true)
        });
        state
            .finish("αβγ", |s| {
                received.push_str(&s);
                Ok(true)
            })
            .unwrap();
        assert_eq!(received, "αβγ");
        state
            .finish("αβγ", |_| panic!("must not duplicate final text"))
            .unwrap();
    }

    #[test]
    fn full_queue_forwards_do_not_recopy_tail() {
        let mut state = StreamReconciler::default();
        let mut bytes_to_send = 0usize;
        for _ in 0..100 {
            state.forward("x", |s| {
                bytes_to_send += s.len();
                Ok(false)
            });
        }
        // Only the first (1-byte) attempt reaches `send`; the rest accumulate.
        assert_eq!(bytes_to_send, 1);
        let mut received = String::new();
        state
            .finish(&"x".repeat(100), |s| {
                received.push_str(&s);
                Ok(true)
            })
            .unwrap();
        assert_eq!(received, "x".repeat(100));
    }

    #[test]
    fn final_only_and_divergent_results() {
        let mut state = StreamReconciler::default();
        state
            .finish("answer", |s| {
                assert_eq!(s, "answer");
                Ok(true)
            })
            .unwrap();
        assert!(state.finish("different", |_| Ok(true)).is_err());
    }

    #[test]
    fn overflow_and_disconnection_are_terminal() {
        let mut state = StreamReconciler::default();
        state.forward(&"x".repeat(MAX_BYTES + 1), |_| panic!("must stay bounded"));
        assert!(state.failed());
        let mut state = StreamReconciler::default();
        state.forward("x", |_| Err("closed".into()));
        assert!(state.finish("x", |_| Ok(true)).is_err());
    }
}
