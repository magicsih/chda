//! App-wide progress of a user-initiated application update.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum UpdateProgress {
    #[default]
    Idle,
    Checking,
    Downloading {
        received: u64,
        total: u64,
    },
    Verifying,
    Ready,
    Waiting,
    Preparing,
    Installing,
    Reconnecting,
    Installed,
    Failed {
        message: String,
    },
}
impl UpdateProgress {
    pub fn busy(&self) -> bool {
        !matches!(self, Self::Idle | Self::Installed | Self::Failed { .. })
    }
    pub fn label(&self) -> String {
        match self {
            Self::Idle => String::new(),
            Self::Checking => "Checking update…".into(),
            Self::Downloading { received, total } if *total > 0 => {
                format!(
                    "Downloading {}%",
                    received
                        .saturating_mul(100)
                        .checked_div(*total)
                        .unwrap_or(0)
                        .min(100)
                )
            }
            Self::Downloading { .. } => "Downloading…".into(),
            Self::Verifying => "Verifying update…".into(),
            Self::Waiting => "Waiting for current task…".into(),
            Self::Ready | Self::Preparing => "Preserving sessions…".into(),
            Self::Installing => "Installing update…".into(),
            Self::Reconnecting => "Reconnecting sessions…".into(),
            Self::Installed => "Update installed".into(),
            Self::Failed { .. } => "Update failed".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn progress_is_bounded_and_failures_allow_retry() {
        assert_eq!(
            UpdateProgress::Downloading {
                received: u64::MAX,
                total: 2
            }
            .label(),
            "Downloading 100%"
        );
        assert!(UpdateProgress::Preparing.busy());
        assert!(
            !UpdateProgress::Failed {
                message: "cancelled".into()
            }
            .busy()
        );
        assert_eq!(
            serde_json::from_str::<UpdateProgress>(r#"{"phase":"ready"}"#).unwrap(),
            UpdateProgress::Ready
        );
    }
}
