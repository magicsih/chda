//! API list prices per model, for estimating what a session would cost if
//! it had been billed per token. Subscriptions (Claude Max, ChatGPT plans)
//! are not billed this way; the estimate says what the same work costs on
//! the API.
//!
//! Sources, checked 2026-10-03:
//! - <https://platform.claude.com/docs/en/about-claude/pricing>
//! - <https://developers.openai.com/api/docs/pricing>
//!
//! Standard rates only: no batch discount, fast mode, data residency or
//! OpenAI's long-context surcharge, which session totals cannot tell apart.

/// Dollars per million tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Price {
    pub input: f64,
    /// Writing to the 5-minute prompt cache (Claude).
    pub cache_write_5m: f64,
    /// Writing to the 1-hour prompt cache (Claude).
    pub cache_write_1h: f64,
    /// Cache hits: Claude's cache reads, OpenAI's cached input.
    pub cache_read: f64,
    pub output: f64,
}

/// A Claude model: cache writes cost 1.25x (5 minutes) and 2x (1 hour) the
/// input price; cache reads are listed per model.
const fn claude(input: f64, cache_read: f64, output: f64) -> Price {
    Price {
        input,
        cache_write_5m: input * 1.25,
        cache_write_1h: input * 2.0,
        cache_read,
        output,
    }
}

/// An OpenAI model: no charge for writing the cache.
const fn openai(input: f64, cached: f64, output: f64) -> Price {
    Price {
        input,
        cache_write_5m: input,
        cache_write_1h: input,
        cache_read: cached,
        output,
    }
}

const PRICES: &[(&str, Price)] = &[
    ("claude-fable-5-1", claude(10.0, 0.25, 50.0)),
    ("claude-mythos-5-1", claude(10.0, 0.25, 50.0)),
    ("claude-fable-5", claude(10.0, 1.0, 50.0)),
    ("claude-mythos-5", claude(10.0, 1.0, 50.0)),
    ("claude-opus-5-5", claude(4.0, 0.20, 20.0)),
    ("claude-opus-5", claude(5.0, 0.50, 25.0)),
    ("claude-opus-4-8", claude(5.0, 0.50, 25.0)),
    ("claude-opus-4-7", claude(5.0, 0.50, 25.0)),
    ("claude-opus-4-6", claude(5.0, 0.50, 25.0)),
    ("claude-opus-4-5", claude(5.0, 0.50, 25.0)),
    ("claude-opus-4-1", claude(15.0, 1.50, 75.0)),
    ("claude-opus-4", claude(15.0, 1.50, 75.0)),
    ("claude-sonnet-5-5", claude(2.0, 0.20, 10.0)),
    ("claude-sonnet-5", claude(2.0, 0.20, 10.0)),
    ("claude-sonnet-4-6", claude(3.0, 0.30, 15.0)),
    ("claude-sonnet-4-5", claude(3.0, 0.30, 15.0)),
    ("claude-sonnet-4", claude(3.0, 0.30, 15.0)),
    ("claude-haiku-4-5", claude(1.0, 0.10, 5.0)),
    ("claude-3-5-haiku", claude(0.80, 0.08, 4.0)),
    ("gpt-6-astra", openai(10.0, 1.0, 50.0)),
    ("gpt-6.1-sol", openai(2.0, 0.10, 10.0)),
    ("gpt-6-sol", openai(2.0, 0.20, 10.0)),
    ("gpt-6-luna", openai(0.10, 0.01, 0.50)),
    ("gpt-5.6-sol", openai(4.0, 0.40, 20.0)),
    ("gpt-5.6-terra", openai(2.0, 0.20, 12.0)),
    ("gpt-5.6-luna", openai(0.20, 0.02, 1.20)),
    ("gpt-5.5", openai(5.0, 0.50, 30.0)),
    ("gpt-5.4", openai(2.5, 0.25, 15.0)),
    ("gpt-5.3-codex", openai(1.75, 0.175, 14.0)),
    ("gpt-5.2", openai(1.75, 0.175, 14.0)),
    ("gpt-5.1", openai(1.25, 0.125, 10.0)),
    ("gpt-5", openai(1.25, 0.125, 10.0)),
];

/// The list price of `model`, by its id as the agent records it. Dated
/// snapshots (`claude-sonnet-4-5-20250929`) and context-window tags
/// (`claude-opus-4-6[1m]`) price like their base model.
pub fn price(model: &str) -> Option<Price> {
    let base = model.split('[').next().unwrap_or(model);
    let base = match base.rsplit_once('-') {
        Some((head, date)) if date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit()) => head,
        _ => base,
    };
    PRICES
        .iter()
        .find(|(id, _)| *id == base)
        .map(|(_, price)| *price)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_snapshots_and_tags() {
        assert_eq!(price("claude-opus-5-5").unwrap().cache_read, 0.20);
        assert_eq!(
            price("claude-sonnet-4-5-20250929"),
            price("claude-sonnet-4-5")
        );
        assert_eq!(price("claude-opus-4-6[1m]"), price("claude-opus-4-6"));
        // A shorter id is not a prefix match for a longer one.
        assert_eq!(price("claude-opus-4-20250514").unwrap().input, 15.0);
        assert_eq!(price("claude-opus-4-8").unwrap().input, 5.0);
        assert_eq!(price("claude-opus-5-5").unwrap().cache_write_1h, 8.0);
        assert_eq!(price("gpt-6.1-sol").unwrap().cache_read, 0.10);
        assert_eq!(price("<synthetic>"), None);
        assert_eq!(price("gpt-9"), None);
    }
}
