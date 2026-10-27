use gitcomet_core::error::Error;
use serde::{Deserialize, Deserializer, Serialize};
use std::time::{Duration, SystemTime};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize)]
#[serde(transparent)]
pub struct AutoFetchInterval(u16);

impl AutoFetchInterval {
    /// Shorter intervals add little freshness for the remote load they cause
    /// when many repositories are open.
    pub const MIN_MINUTES: u16 = 5;
    pub const MAX_MINUTES: u16 = 1440;

    pub fn new(minutes: u16) -> Option<Self> {
        (Self::MIN_MINUTES..=Self::MAX_MINUTES)
            .contains(&minutes)
            .then_some(Self(minutes))
    }

    pub fn minutes(self) -> u16 {
        self.0
    }

    pub(crate) fn duration(self) -> Duration {
        Duration::from_secs(u64::from(self.0) * 60)
    }
}

impl Default for AutoFetchInterval {
    fn default() -> Self {
        Self(30)
    }
}

impl<'de> Deserialize<'de> for AutoFetchInterval {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let minutes = u16::deserialize(deserializer)?;
        // Intervals saved before the minimum rose keep the closest allowed one.
        let minutes = if minutes == 0 {
            minutes
        } else {
            minutes.max(Self::MIN_MINUTES)
        };
        Self::new(minutes).ok_or_else(|| serde::de::Error::custom("expected 5–1440 minutes"))
    }
}

/// Inactive tabs whose fetches keep finding nothing new wait up to this many
/// intervals between fetches.
pub const AUTO_FETCH_QUIET_MAX_MULTIPLIER: u32 = 4;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct AutoFetchSettings {
    pub enabled: bool,
    pub interval: AutoFetchInterval,
}

#[derive(Clone, Debug, Default)]
pub enum AutoFetchStatus {
    #[default]
    Idle,
    Running,
    Succeeded {
        at: SystemTime,
    },
    Failed {
        at: SystemTime,
        error: Error,
    },
}

#[derive(Clone, Debug, Default)]
pub struct RepoAutoFetchState {
    pub last_success_at: Option<SystemTime>,
    pub status: AutoFetchStatus,
    /// When failures pushed the next attempt past the interval.
    pub retry_at: Option<SystemTime>,
    /// Remotes skipped until a manual fetch succeeds, after a sign-in or
    /// missing-repository failure.
    pub paused_remotes: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CommandSource {
    #[default]
    User,
    Background,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_interval_accepts_whole_minutes_only_within_the_supported_range() {
        for invalid in ["0", "1441", "-1", "1.5", "\"30\""] {
            assert!(
                serde_json::from_str::<AutoFetchInterval>(invalid).is_err(),
                "{invalid}"
            );
        }
        for (saved, minutes) in [(1, 5), (4, 5), (5, 5), (30, 30), (1440, 1440)] {
            let interval = serde_json::from_str::<AutoFetchInterval>(&saved.to_string()).unwrap();
            assert_eq!(interval.minutes(), minutes);
            assert_eq!(
                serde_json::to_string(&interval).unwrap(),
                minutes.to_string()
            );
        }
        assert_eq!(AutoFetchInterval::new(4), None);
        assert!(!AutoFetchSettings::default().enabled);
        assert_eq!(AutoFetchSettings::default().interval.minutes(), 30);
    }
}
