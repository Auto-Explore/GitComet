#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DateTimeFormat {
    YmdHm,
    YmdHms,
    DmyHm,
    MdyHm,
}

impl DateTimeFormat {
    pub(super) fn all() -> &'static [DateTimeFormat] {
        &[
            DateTimeFormat::YmdHm,
            DateTimeFormat::YmdHms,
            DateTimeFormat::DmyHm,
            DateTimeFormat::MdyHm,
        ]
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            DateTimeFormat::YmdHm => "YYYY-MM-DD HH:MM",
            DateTimeFormat::YmdHms => "YYYY-MM-DD HH:MM:SS",
            DateTimeFormat::DmyHm => "DD.MM.YYYY HH:MM",
            DateTimeFormat::MdyHm => "MM/DD/YYYY HH:MM",
        }
    }

    pub(super) fn key(self) -> &'static str {
        match self {
            DateTimeFormat::YmdHm => "ymd_hm_utc",
            DateTimeFormat::YmdHms => "ymd_hms_utc",
            DateTimeFormat::DmyHm => "dmy_hm_utc",
            DateTimeFormat::MdyHm => "mdy_hm_utc",
        }
    }

    pub(super) fn from_key(s: &str) -> Option<Self> {
        match s {
            "ymd_hm_utc" => Some(DateTimeFormat::YmdHm),
            "ymd_hms_utc" => Some(DateTimeFormat::YmdHms),
            "dmy_hm_utc" => Some(DateTimeFormat::DmyHm),
            "mdy_hm_utc" => Some(DateTimeFormat::MdyHm),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum Timezone {
    #[default]
    Utc,
    /// Fixed offset from UTC in seconds (positive = east of UTC).
    Fixed(i32),
}

impl Timezone {
    pub(super) fn all() -> &'static [Timezone] {
        use Timezone::*;
        &[
            Utc,
            Fixed(-12 * 3600),
            Fixed(-11 * 3600),
            Fixed(-10 * 3600),
            Fixed(-9 * 3600 - 30 * 60),
            Fixed(-9 * 3600),
            Fixed(-8 * 3600),
            Fixed(-7 * 3600),
            Fixed(-6 * 3600),
            Fixed(-5 * 3600),
            Fixed(-4 * 3600),
            Fixed(-3 * 3600 - 30 * 60),
            Fixed(-3 * 3600),
            Fixed(-2 * 3600),
            Fixed(-3600),
            Fixed(3600),
            Fixed(2 * 3600),
            Fixed(3 * 3600),
            Fixed(3 * 3600 + 30 * 60),
            Fixed(4 * 3600),
            Fixed(4 * 3600 + 30 * 60),
            Fixed(5 * 3600),
            Fixed(5 * 3600 + 30 * 60),
            Fixed(5 * 3600 + 45 * 60),
            Fixed(6 * 3600),
            Fixed(6 * 3600 + 30 * 60),
            Fixed(7 * 3600),
            Fixed(8 * 3600),
            Fixed(8 * 3600 + 45 * 60),
            Fixed(9 * 3600),
            Fixed(9 * 3600 + 30 * 60),
            Fixed(10 * 3600),
            Fixed(10 * 3600 + 30 * 60),
            Fixed(11 * 3600),
            Fixed(12 * 3600),
            Fixed(12 * 3600 + 45 * 60),
            Fixed(13 * 3600),
            Fixed(14 * 3600),
        ]
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Timezone::Utc => "UTC",
            Timezone::Fixed(s) => match s {
                -43200 => "UTC\u{2212}12",
                -39600 => "UTC\u{2212}11",
                -36000 => "UTC\u{2212}10",
                -34200 => "UTC\u{2212}9:30",
                -32400 => "UTC\u{2212}9",
                -28800 => "UTC\u{2212}8",
                -25200 => "UTC\u{2212}7",
                -21600 => "UTC\u{2212}6",
                -18000 => "UTC\u{2212}5",
                -14400 => "UTC\u{2212}4",
                -12600 => "UTC\u{2212}3:30",
                -10800 => "UTC\u{2212}3",
                -7200 => "UTC\u{2212}2",
                -3600 => "UTC\u{2212}1",
                3600 => "UTC+1",
                7200 => "UTC+2",
                10800 => "UTC+3",
                12600 => "UTC+3:30",
                14400 => "UTC+4",
                16200 => "UTC+4:30",
                18000 => "UTC+5",
                19800 => "UTC+5:30",
                20700 => "UTC+5:45",
                21600 => "UTC+6",
                23400 => "UTC+6:30",
                25200 => "UTC+7",
                28800 => "UTC+8",
                31500 => "UTC+8:45",
                32400 => "UTC+9",
                34200 => "UTC+9:30",
                36000 => "UTC+10",
                37800 => "UTC+10:30",
                39600 => "UTC+11",
                43200 => "UTC+12",
                45900 => "UTC+12:45",
                46800 => "UTC+13",
                50400 => "UTC+14",
                _ => "UTC+?",
            },
        }
    }

    pub(super) fn key(self) -> String {
        match self {
            Timezone::Utc => "utc".to_string(),
            Timezone::Fixed(s) => format!("fixed_{s}"),
        }
    }

    pub(super) fn from_key(s: &str) -> Option<Self> {
        match s {
            "utc" => Some(Timezone::Utc),
            _ => {
                let suffix = s.strip_prefix("fixed_")?;
                let seconds: i32 = suffix.parse().ok()?;
                Some(Timezone::Fixed(seconds))
            }
        }
    }

    pub(super) fn cities(self) -> &'static str {
        match self {
            Timezone::Utc => "London, Reykjavik",
            Timezone::Fixed(s) => match s {
                -43200 => "Baker Island",
                -39600 => "Pago Pago",
                -36000 => "Honolulu",
                -34200 => "Marquesas Islands",
                -32400 => "Anchorage",
                -28800 => "Los Angeles, Vancouver",
                -25200 => "Denver, Phoenix",
                -21600 => "Chicago, Mexico City",
                -18000 => "New York, Toronto",
                -14400 => "Santiago, Halifax",
                -12600 => "St. John's",
                -10800 => "São Paulo, Buenos Aires",
                -7200 => "South Georgia",
                -3600 => "Azores, Cape Verde",
                3600 => "Berlin, Paris, Lagos",
                7200 => "Helsinki, Cairo, Kyiv",
                10800 => "Moscow, Istanbul, Nairobi",
                12600 => "Tehran",
                14400 => "Dubai, Baku",
                16200 => "Kabul",
                18000 => "Karachi, Tashkent",
                19800 => "Mumbai, Delhi, Colombo",
                20700 => "Kathmandu",
                21600 => "Dhaka, Almaty",
                23400 => "Yangon",
                25200 => "Bangkok, Jakarta, Hanoi",
                28800 => "Singapore, Beijing, Taipei",
                31500 => "Eucla",
                32400 => "Tokyo, Seoul",
                34200 => "Adelaide",
                36000 => "Sydney, Melbourne",
                37800 => "Lord Howe Island",
                39600 => "Noumea, Solomon Islands",
                43200 => "Auckland, Fiji",
                45900 => "Chatham Islands",
                46800 => "Apia, Tongatapu",
                50400 => "Kiritimati",
                _ => "",
            },
        }
    }

    pub(super) fn offset_seconds(self) -> i64 {
        match self {
            Timezone::Utc => 0,
            Timezone::Fixed(s) => s as i64,
        }
    }
}

pub(super) fn format_datetime(
    time: std::time::SystemTime,
    format: DateTimeFormat,
    timezone: Timezone,
) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unix_seconds(t: SystemTime) -> i64 {
        match t.duration_since(UNIX_EPOCH) {
            Ok(d) => d.as_secs() as i64,
            Err(e) => -(e.duration().as_secs() as i64),
        }
    }

    fn floor_div(a: i64, b: i64) -> i64 {
        let mut q = a / b;
        let r = a % b;
        if (r != 0) && ((r < 0) != (b < 0)) {
            q -= 1;
        }
        q
    }

    // Howard Hinnant's `civil_from_days` algorithm.
    fn civil_from_days(days_since_epoch: i64) -> (i32, u32, u32) {
        let z = days_since_epoch + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = z - era * 146_097; // [0, 146096]
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
        let mp = (5 * doy + 2) / 153; // [0, 11]
        let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
        let m = mp + if mp < 10 { 3 } else { -9 }; // [1, 12]
        let y = y + i64::from(m <= 2);
        (y as i32, m as u32, d as u32)
    }

    let offset = timezone.offset_seconds();
    let secs = unix_seconds(time) + offset;
    let days = floor_div(secs, 86_400);
    let sec_of_day = secs - days * 86_400;
    let sec_of_day: i64 = if sec_of_day < 0 {
        sec_of_day + 86_400
    } else {
        sec_of_day
    };

    let hour = (sec_of_day / 3600) as u32;
    let minute = ((sec_of_day % 3600) / 60) as u32;
    let second = (sec_of_day % 60) as u32;

    let (y, m, d) = civil_from_days(days);

    let tz_label = timezone.label();
    match format {
        DateTimeFormat::YmdHm => {
            format!("{y:04}-{m:02}-{d:02} {hour:02}:{minute:02} {tz_label}")
        }
        DateTimeFormat::YmdHms => {
            format!("{y:04}-{m:02}-{d:02} {hour:02}:{minute:02}:{second:02} {tz_label}")
        }
        DateTimeFormat::DmyHm => {
            format!("{d:02}.{m:02}.{y:04} {hour:02}:{minute:02} {tz_label}")
        }
        DateTimeFormat::MdyHm => {
            format!("{m:02}/{d:02}/{y:04} {hour:02}:{minute:02} {tz_label}")
        }
    }
}

/// Backward-compatible wrapper that formats in UTC.
#[cfg(test)]
pub(super) fn format_datetime_utc(time: std::time::SystemTime, format: DateTimeFormat) -> String {
    format_datetime(time, format, Timezone::Utc)
}
