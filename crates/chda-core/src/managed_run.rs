//! Process lifecycle is separate from an agent's turn status.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ManagedRun {
    Starting {
        started_at: u64,
    },
    Running {
        started_at: u64,
    },
    Stopped {
        code: Option<u32>,
        signal: Option<String>,
        error: Option<String>,
    },
}

impl ManagedRun {
    pub fn started_at(&self) -> Option<u64> {
        match self {
            Self::Starting { started_at } | Self::Running { started_at } => Some(*started_at),
            Self::Stopped { .. } => None,
        }
    }
    pub fn stopped(&self) -> bool {
        matches!(self, Self::Stopped { .. })
    }
    pub fn failed(error: String) -> Self {
        Self::Stopped {
            code: None,
            signal: None,
            error: Some(error),
        }
    }
    pub fn label(&self) -> String {
        match self {
            Self::Starting { .. } => "Starting…".into(),
            Self::Running { .. } => "Running".into(),
            Self::Stopped {
                code,
                signal,
                error,
            } => error.clone().unwrap_or_else(|| {
                if let Some(signal) = signal {
                    format!("Exited — signal {signal}")
                } else if let Some(code) = code {
                    format!("Exited — code {code}")
                } else {
                    "Exited — status unavailable".into()
                }
            }),
        }
    }
}
