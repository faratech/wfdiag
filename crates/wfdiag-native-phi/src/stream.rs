//! Bounded delta accumulation and final-result reconciliation.
const MAX_BYTES: usize = 1024 * 1024;

#[derive(Default)]
pub(crate) struct StreamReconciler {
    text: String,
    sent: usize,
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
        match send(self.text[self.sent..].to_string()) {
            Ok(true) => self.sent = self.text.len(),
            Ok(false) => {} // queue full: coalesce until the next callback/final result
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
