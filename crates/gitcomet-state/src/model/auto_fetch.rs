use gitcomet_core::error::Error;
use serde::{Deserialize, Deserializer, Serialize};
use std::time::{Duration, SystemTime};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize)]
#[serde(transparent)]
pub struct AutoFetchInterval(u16);

impl AutoFetchInterval {
    pub fn new(minutes: u16) -> Option<Self> {
        (1..=1440).contains(&minutes).then_some(Self(minutes))
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
        Self::new(minutes).ok_or_else(|| serde::de::Error::custom("expected 1–1440 minutes"))
    }
}

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
        for minutes in [1, 30, 1440] {
            let interval = serde_json::from_str::<AutoFetchInterval>(&minutes.to_string()).unwrap();
            assert_eq!(interval.minutes(), minutes);
            assert_eq!(
                serde_json::to_string(&interval).unwrap(),
                minutes.to_string()
            );
        }
        assert!(!AutoFetchSettings::default().enabled);
        assert_eq!(AutoFetchSettings::default().interval.minutes(), 30);
    }
}
