//! Token/cost budget guard: estimate what an exchange will cost before it is sent, and
//! warn when a request is unusually large.
//!
//! BYOK users pay per token, so a runaway context (a huge patch, a long back-and-forth)
//! should be visible *before* the send, next to the privacy preview. The estimate is a
//! deliberately simple, provider-agnostic heuristic — roughly four characters per token —
//! not an exact tokenizer; it is meant to catch order-of-magnitude surprises, not to bill.

use crate::provider::ChatRequest;

/// Characters per token in the estimate (a rough industry average for English text).
const CHARS_PER_TOKEN: usize = 4;

/// Per-message framing overhead added to the content estimate (role tags, delimiters).
const MESSAGE_OVERHEAD_TOKENS: usize = 4;

/// Estimates the token count of `text` (`~ chars / 4`, rounded up; empty → 0).
#[must_use]
pub fn estimate_tokens(text: &str) -> usize {
    let chars = text.chars().count();
    if chars == 0 {
        0
    } else {
        chars.div_ceil(CHARS_PER_TOKEN)
    }
}

/// The estimated token usage of one exchange.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    /// Estimated tokens in the prompt (system + conversation).
    pub prompt_tokens: usize,
    /// The reply budget (`ChatRequest::max_tokens`), the most the completion can add.
    pub max_completion_tokens: usize,
}

impl Usage {
    /// The worst-case total: prompt plus the full reply budget.
    #[must_use]
    pub fn total(&self) -> usize {
        self.prompt_tokens + self.max_completion_tokens
    }
}

/// Estimates the usage of `request`: the system prompt plus every message (with a small
/// per-message framing overhead), and the reply budget from `max_tokens`.
#[must_use]
pub fn estimate_usage(request: &ChatRequest) -> Usage {
    let mut prompt = estimate_tokens(&request.system);
    for message in &request.messages {
        prompt += estimate_tokens(&message.content) + MESSAGE_OVERHEAD_TOKENS;
    }
    Usage {
        prompt_tokens: prompt,
        max_completion_tokens: request.max_tokens as usize,
    }
}

/// Estimated cost in USD for `tokens` at a `usd_per_1k`-dollars-per-1000-tokens rate.
#[must_use]
pub fn estimate_cost_usd(tokens: usize, usd_per_1k: f64) -> f64 {
    (tokens as f64 / 1000.0) * usd_per_1k
}

/// A guard that flags a request whose estimated prompt exceeds a threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetGuard {
    /// Warn when the estimated *prompt* token count exceeds this.
    pub warn_over_tokens: usize,
}

/// The guard's verdict, carrying the estimate either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetCheck {
    /// The request is within budget.
    Within(Usage),
    /// The request is unusually large — warn before sending.
    Over(Usage),
}

impl BudgetCheck {
    /// The usage estimate, regardless of verdict.
    #[must_use]
    pub fn usage(&self) -> Usage {
        match self {
            BudgetCheck::Within(u) | BudgetCheck::Over(u) => *u,
        }
    }

    /// Whether the request is over budget.
    #[must_use]
    pub fn is_over(&self) -> bool {
        matches!(self, BudgetCheck::Over(_))
    }
}

impl BudgetGuard {
    /// A guard warning over `warn_over_tokens` prompt tokens.
    #[must_use]
    pub fn new(warn_over_tokens: usize) -> Self {
        Self { warn_over_tokens }
    }

    /// Estimates `request` and rules on it against the threshold.
    #[must_use]
    pub fn check(&self, request: &ChatRequest) -> BudgetCheck {
        let usage = estimate_usage(request);
        if usage.prompt_tokens > self.warn_over_tokens {
            BudgetCheck::Over(usage)
        } else {
            BudgetCheck::Within(usage)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{ChatRequest, Message, ProviderConfig};

    #[test]
    fn token_estimate_is_about_four_chars_each() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2, "rounds up");
        // ~40 chars → ~10 tokens.
        assert_eq!(estimate_tokens(&"x".repeat(40)), 10);
    }

    #[test]
    fn usage_counts_system_plus_messages_and_the_reply_budget() {
        let config = ProviderConfig {
            max_tokens: 100,
            ..Default::default()
        };
        // system 8 chars → 2 tokens; one 8-char message → 2 + 4 overhead.
        let request = ChatRequest::new(&config, "systemAB", vec![Message::user("userABCD")]);
        let usage = estimate_usage(&request);
        assert_eq!(usage.prompt_tokens, 2 + (2 + MESSAGE_OVERHEAD_TOKENS));
        assert_eq!(usage.max_completion_tokens, 100);
        assert_eq!(usage.total(), usage.prompt_tokens + 100);
    }

    #[test]
    fn the_guard_flags_only_oversized_prompts() {
        let config = ProviderConfig::default();
        let small = ChatRequest::new(&config, "hi", vec![Message::user("there")]);
        let guard = BudgetGuard::new(1_000);
        let verdict = guard.check(&small);
        assert!(!verdict.is_over());
        assert!(matches!(verdict, BudgetCheck::Within(_)));

        // A large system prompt trips the threshold.
        let big = ChatRequest::new(&config, "x".repeat(8_000), vec![]);
        let verdict = guard.check(&big);
        assert!(verdict.is_over());
        assert_eq!(verdict.usage().prompt_tokens, 2_000);
    }

    #[test]
    fn cost_scales_with_tokens_and_rate() {
        // 2000 tokens at $3 / 1k = $6.
        assert!((estimate_cost_usd(2_000, 3.0) - 6.0).abs() < 1e-9);
        assert_eq!(estimate_cost_usd(0, 3.0), 0.0);
    }
}
