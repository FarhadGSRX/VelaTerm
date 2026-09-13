//! Stream timing for the current root turn. Never divide by time spent executing tools or waiting for
//! user input. Retain only completed responses whose provider token count can be matched to the stream.
use serde_json::Value;
use std::collections::HashMap;

#[derive(Clone, Debug, Default)]
struct Sample {
    start: u64,
    end: u64,
    tokens: Option<u64>,
    complete: bool,
}
#[derive(Clone, Debug, Default)]
pub(super) struct GenerationTiming {
    samples: HashMap<String, Sample>,
    claude_message: Option<String>,
    codex_start: Option<u64>,
    codex_end: Option<u64>,
    codex_total: Option<u64>,
}
impl GenerationTiming {
    pub fn rate(&self) -> Option<f64> {
        let mut tokens = 0u64;
        let mut elapsed = 0u64;
        for sample in self.samples.values() {
            if !sample.complete || sample.end <= sample.start {
                continue;
            }
            if let Some(count) = sample.tokens {
                tokens = tokens.saturating_add(count);
                elapsed = elapsed.saturating_add(sample.end - sample.start);
            }
        }
        (elapsed >= 100 && tokens > 0).then(|| tokens as f64 * 1000.0 / elapsed as f64)
    }
    pub fn claude(&mut self, value: &Value, now: u64) {
        if value
            .get("parent_tool_use_id")
            .is_some_and(|v| !v.is_null())
        {
            return;
        }
        if value.get("type").and_then(Value::as_str) == Some("assistant") {
            if let (Some(id), Some(tokens)) = (
                value.pointer("/message/id").and_then(Value::as_str),
                value
                    .pointer("/message/usage/output_tokens")
                    .and_then(Value::as_u64),
            ) {
                if let Some(sample) = self.samples.get_mut(id) {
                    sample.tokens = Some(tokens);
                }
            }
        }
        let Some(event) = value.get("event") else {
            return;
        };
        match event.get("type").and_then(Value::as_str) {
            Some("message_start") => {
                self.claude_message = event
                    .pointer("/message/id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                if let Some(id) = &self.claude_message {
                    self.samples.insert(
                        id.clone(),
                        Sample {
                            start: now,
                            end: now,
                            ..Sample::default()
                        },
                    );
                }
            }
            Some("message_delta") => {
                if let Some(sample) = self
                    .claude_message
                    .as_ref()
                    .and_then(|id| self.samples.get_mut(id))
                {
                    sample.tokens = event
                        .pointer("/usage/output_tokens")
                        .and_then(Value::as_u64)
                        .or(sample.tokens);
                }
            }
            Some("message_stop") => {
                if let Some(id) = self.claude_message.take() {
                    if let Some(sample) = self.samples.get_mut(&id) {
                        sample.end = now;
                        sample.complete = true;
                    }
                }
            }
            _ => {}
        }
    }
    /// Pi and OMP stream one assistant message at a time, so the message itself is the measured window:
    /// `message_start` opens it and `message_end` closes it with the provider's own output-token count.
    pub fn pi_start(&mut self, key: &str, now: u64) {
        self.samples.insert(
            key.to_string(),
            Sample {
                start: now,
                end: now,
                ..Sample::default()
            },
        );
    }
    pub fn pi_end(&mut self, key: &str, tokens: Option<u64>, now: u64) {
        let Some(sample) = self.samples.get_mut(key) else {
            return;
        };
        sample.end = now;
        sample.tokens = tokens;
        sample.complete = true;
    }
    pub fn codex(&mut self, method: &str, params: &Value, now: u64) {
        let streamed = matches!(
            method,
            "item/agentMessage/delta"
                | "item/reasoning/textDelta"
                | "item/reasoning/summaryTextDelta"
        );
        if streamed {
            self.codex_start.get_or_insert(now);
            self.codex_end = Some(now);
        }
        // An output item finishes before tool execution. This also accounts for the final fragment.
        if method == "item/completed"
            && matches!(
                params.pointer("/item/type").and_then(Value::as_str),
                Some("agentMessage" | "reasoning")
            )
        {
            if self.codex_start.is_some() {
                self.codex_end = Some(now);
            }
        }
        if method != "thread/tokenUsage/updated" {
            return;
        }
        let Some(total) = params
            .pointer("/tokenUsage/total/outputTokens")
            .and_then(Value::as_u64)
        else {
            return;
        };
        if self.codex_total == Some(total) {
            return;
        }
        self.codex_total = Some(total);
        let (start, end) = (self.codex_start.take(), self.codex_end.take());
        if let (Some(start), Some(end), Some(tokens)) = (
            start,
            end,
            params
                .pointer("/tokenUsage/last/outputTokens")
                .and_then(Value::as_u64),
        ) {
            self.samples.insert(
                format!("codex-{total}"),
                Sample {
                    start,
                    end,
                    tokens: Some(tokens),
                    complete: true,
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn claude_matches_usage_once_and_excludes_tool_wait() {
        let mut timing = GenerationTiming::default();
        timing.claude(
            &json!({"event":{"type":"message_start","message":{"id":"a"}}}),
            1000,
        );
        timing.claude(
            &json!({"event":{"type":"message_delta","usage":{"output_tokens":120}}}),
            2900,
        );
        timing.claude(&json!({"event":{"type":"message_stop"}}), 3000);
        timing.claude(
            &json!({"type":"assistant","message":{"id":"a","usage":{"output_tokens":120}}}),
            50000,
        );
        assert_eq!(timing.rate(), Some(60.0));
    }
    #[test]
    fn codex_deduplicates_usage_and_times_only_streamed_output() {
        let mut timing = GenerationTiming::default();
        timing.codex("item/reasoning/summaryTextDelta", &json!({}), 1000);
        timing.codex("item/agentMessage/delta", &json!({}), 3000);
        let usage =
            json!({"tokenUsage":{"total":{"outputTokens":500},"last":{"outputTokens":100}}});
        timing.codex("thread/tokenUsage/updated", &usage, 50000);
        timing.codex("thread/tokenUsage/updated", &usage, 100000);
        assert_eq!(timing.rate(), Some(50.0));
        assert_eq!(timing.samples.len(), 1);
        assert_eq!(GenerationTiming::default().rate(), None);
    }
    #[test]
    fn pi_times_each_assistant_message_and_ignores_one_without_a_token_count() {
        let mut timing = GenerationTiming::default();
        timing.pi_start("pi-a0", 1000);
        timing.pi_end("pi-a0", Some(120), 3000);
        timing.pi_start("pi-a1", 4000);
        timing.pi_end("pi-a1", None, 9000);
        assert_eq!(timing.rate(), Some(60.0));
    }
}
