//! Token usage of an agent session, read from its transcript, and what it
//! would cost at API list prices ([`crate::pricing`]).

use serde::{Deserialize, Serialize};

use crate::pricing;

/// Tokens one model used in a session.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelUsage {
    pub model: String,
    /// Input that was neither written to nor read from the cache.
    pub input: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
    pub cache_read: u64,
    /// Output, reasoning included.
    pub output: u64,
}

impl ModelUsage {
    pub fn total(&self) -> u64 {
        self.input + self.cache_write_5m + self.cache_write_1h + self.cache_read + self.output
    }

    /// Dollars at the model's list price, or `None` for a model without one.
    pub fn cost(&self) -> Option<f64> {
        let p = pricing::price(&self.model)?;
        let per = |tokens: u64, rate: f64| tokens as f64 * rate / 1_000_000.0;
        Some(
            per(self.input, p.input)
                + per(self.cache_write_5m, p.cache_write_5m)
                + per(self.cache_write_1h, p.cache_write_1h)
                + per(self.cache_read, p.cache_read)
                + per(self.output, p.output),
        )
    }

    fn add(&mut self, other: &ModelUsage) {
        self.input += other.input;
        self.cache_write_5m += other.cache_write_5m;
        self.cache_write_1h += other.cache_write_1h;
        self.cache_read += other.cache_read;
        self.output += other.output;
    }
}

/// How much of a plan's rate limit was used, as the agent last reported it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LimitUsage {
    /// Percent of the limit used, rounded.
    pub used_percent: u32,
    /// Length of the limit's window, e.g. 10080 for a week.
    pub window_minutes: u64,
}

/// A session's token usage, per model.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub models: Vec<ModelUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<LimitUsage>,
}

impl Usage {
    /// Count `usage` toward its model. Models without a name (`""`) and
    /// empty entries are skipped.
    pub fn add(&mut self, usage: &ModelUsage) {
        if usage.model.is_empty() || usage.total() == 0 {
            return;
        }
        match self.models.iter_mut().find(|m| m.model == usage.model) {
            Some(m) => m.add(usage),
            None => self.models.push(usage.clone()),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.models.is_empty()
    }

    pub fn total(&self) -> u64 {
        self.models.iter().map(ModelUsage::total).sum()
    }

    /// The sum over all models: input, cache writes, cache reads, output.
    pub fn summed(&self) -> ModelUsage {
        let mut sum = ModelUsage::default();
        for m in &self.models {
            sum.add(m);
        }
        sum
    }

    /// Dollars at list prices, and whether every model had a price.
    pub fn cost(&self) -> (f64, bool) {
        let mut dollars = 0.0;
        let mut complete = true;
        for m in &self.models {
            match m.cost() {
                Some(c) => dollars += c,
                None => complete = false,
            }
        }
        (dollars, complete)
    }
}

/// Tokens as a short label: `950`, `9.5K`, `340K`, `1.2M`.
pub fn compact_tokens(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..10_000 => format!("{:.1}K", n as f64 / 1_000.0),
        10_000..1_000_000 => format!("{}K", n / 1_000),
        1_000_000..10_000_000 => format!("{:.1}M", n as f64 / 1_000_000.0),
        _ => format!("{}M", n / 1_000_000),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(model: &str, input: u64, w5: u64, w1: u64, read: u64, output: u64) -> ModelUsage {
        ModelUsage {
            model: model.into(),
            input,
            cache_write_5m: w5,
            cache_write_1h: w1,
            cache_read: read,
            output,
        }
    }

    #[test]
    fn costs_add_up_per_model() {
        let mut u = Usage::default();
        // Opus 5.5: $4 in, $5 / $8 cache writes, $0.20 cache reads, $20 out.
        u.add(&usage(
            "claude-opus-5-5",
            1_000_000,
            1_000_000,
            1_000_000,
            1_000_000,
            1_000_000,
        ));
        u.add(&usage("claude-opus-5-5", 1_000_000, 0, 0, 0, 0));
        u.add(&usage("claude-haiku-4-5", 0, 0, 0, 0, 1_000_000));
        u.add(&usage("<synthetic>", 0, 0, 0, 0, 0));
        assert_eq!(u.models.len(), 2);
        let (dollars, complete) = u.cost();
        assert!((dollars - (4.0 * 2.0 + 5.0 + 8.0 + 0.2 + 20.0 + 5.0)).abs() < 1e-9);
        assert!(complete);
        u.add(&usage("someday-model", 10, 0, 0, 0, 0));
        assert!(!u.cost().1);
        assert_eq!(u.total(), 7_000_010);
    }

    #[test]
    fn compact_labels() {
        assert_eq!(compact_tokens(950), "950");
        assert_eq!(compact_tokens(9_500), "9.5K");
        assert_eq!(compact_tokens(340_400), "340K");
        assert_eq!(compact_tokens(1_234_567), "1.2M");
        assert_eq!(compact_tokens(42_000_000), "42M");
    }
}
