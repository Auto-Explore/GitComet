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
    /// The device's timezone, resolved per timestamp (DST-correct via jiff).
    #[default]
    SystemLocal,
    Utc,
    /// Fixed offset from UTC in seconds (positive = east of UTC).
    Fixed(i32),
}

impl Timezone {
    pub(super) fn all() -> &'static [Timezone] {
        use Timezone::*;
        &[
            SystemLocal,
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
            Timezone::SystemLocal => "System local",
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
            Timezone::SystemLocal => "system_local".to_string(),
            Timezone::Utc => "utc".to_string(),
            Timezone::Fixed(s) => format!("fixed_{s}"),
        }
    }

    pub(super) fn from_key(s: &str) -> Option<Self> {
        match s {
            "system_local" => Some(Timezone::SystemLocal),
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
            Timezone::SystemLocal => "This device's timezone",
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

    /// UTC offset in seconds for the given unix timestamp. `SystemLocal`
    /// resolves the device timezone per timestamp, so historical commits keep
    /// their correct DST offset.
    pub(super) fn offset_seconds_at(self, unix_seconds: i64) -> i64 {
        match self {
            Timezone::SystemLocal => jiff::Timestamp::from_second(unix_seconds)
                .map(|ts| i64::from(jiff::tz::TimeZone::system().to_offset(ts).seconds()))
                .unwrap_or(0),
            Timezone::Utc => 0,
            Timezone::Fixed(s) => s as i64,
        }
    }
}

#[cfg(test)]
pub(super) fn format_datetime(
    time: std::time::SystemTime,
    format: DateTimeFormat,
    timezone: Timezone,
    show_timezone: bool,
) -> String {
    let mut buf = String::with_capacity(24);
    format_datetime_into(&mut buf, time, format, timezone, show_timezone);
    buf
}

/// Like `format_datetime` but writes into a caller-owned buffer,
/// allowing the allocation to be reused across many calls.
pub(super) fn format_datetime_into(
    buf: &mut String,
    time: std::time::SystemTime,
    format: DateTimeFormat,
    timezone: Timezone,
    show_timezone: bool,
) {
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
        let z = days_since_epoch.saturating_add(719_468);
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

    /// Two-digit ASCII lookup table: DEC_PAIR[n] = "00".."99" for n in 0..100.
    static DEC_PAIR: [[u8; 2]; 100] = {
        let mut table = [[0u8; 2]; 100];
        let mut i = 0usize;
        while i < 100 {
            table[i][0] = b'0' + (i / 10) as u8;
            table[i][1] = b'0' + (i % 10) as u8;
            i += 1;
        }
        table
    };

    #[inline(always)]
    fn write2(arr: &mut [u8; 19], pos: usize, val: u32) {
        let pair = DEC_PAIR[(val % 100) as usize];
        arr[pos] = pair[0];
        arr[pos + 1] = pair[1];
    }

    #[inline(always)]
    fn write4_year(arr: &mut [u8; 19], pos: usize, y: i32) {
        // The fixed 4-digit field can only represent years 0..=9999; clamp so a
        // corrupt or extreme timestamp shows a boundary value rather than
        // silently dropping the sign or high digits (e.g. 12025 -> "2025",
        // -44 -> "0044").
        let y = y.clamp(0, 9999) as u32;
        let hi = y / 100;
        let lo = y % 100;
        let p1 = DEC_PAIR[hi as usize];
        let p2 = DEC_PAIR[lo as usize];
        arr[pos] = p1[0];
        arr[pos + 1] = p1[1];
        arr[pos + 2] = p2[0];
        arr[pos + 3] = p2[1];
    }

    buf.clear();

    let unix = unix_seconds(time);
    let offset = timezone.offset_seconds_at(unix);
    let secs = unix.saturating_add(offset);
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

    // Build the date-time string in a fixed stack buffer (all ASCII, always
    // valid UTF-8) and push_str once — avoids std::fmt dispatch overhead.
    let mut arr = [0u8; 19]; // max: "YYYY-MM-DD HH:MM:SS"

    match format {
        DateTimeFormat::YmdHm => {
            // "YYYY-MM-DD HH:MM" — 16 bytes
            write4_year(&mut arr, 0, y);
            arr[4] = b'-';
            write2(&mut arr, 5, m);
            arr[7] = b'-';
            write2(&mut arr, 8, d);
            arr[10] = b' ';
            write2(&mut arr, 11, hour);
            arr[13] = b':';
            write2(&mut arr, 14, minute);
            // SAFETY: all bytes are ASCII digits, '-', ' ', or ':'
            buf.push_str(std::str::from_utf8(&arr[..16]).unwrap());
        }
        DateTimeFormat::YmdHms => {
            // "YYYY-MM-DD HH:MM:SS" — 19 bytes
            write4_year(&mut arr, 0, y);
            arr[4] = b'-';
            write2(&mut arr, 5, m);
            arr[7] = b'-';
            write2(&mut arr, 8, d);
            arr[10] = b' ';
            write2(&mut arr, 11, hour);
            arr[13] = b':';
            write2(&mut arr, 14, minute);
            arr[16] = b':';
            write2(&mut arr, 17, second);
            buf.push_str(std::str::from_utf8(&arr[..19]).unwrap());
        }
        DateTimeFormat::DmyHm => {
            // "DD.MM.YYYY HH:MM" — 16 bytes
            write2(&mut arr, 0, d);
            arr[2] = b'.';
            write2(&mut arr, 3, m);
            arr[5] = b'.';
            write4_year(&mut arr, 6, y);
            arr[10] = b' ';
            write2(&mut arr, 11, hour);
            arr[13] = b':';
            write2(&mut arr, 14, minute);
            buf.push_str(std::str::from_utf8(&arr[..16]).unwrap());
        }
        DateTimeFormat::MdyHm => {
            // "MM/DD/YYYY HH:MM" — 16 bytes
            write2(&mut arr, 0, m);
            arr[2] = b'/';
            write2(&mut arr, 3, d);
            arr[5] = b'/';
            write4_year(&mut arr, 6, y);
            arr[10] = b' ';
            write2(&mut arr, 11, hour);
            arr[13] = b':';
            write2(&mut arr, 14, minute);
            buf.push_str(std::str::from_utf8(&arr[..16]).unwrap());
        }
    }
    if show_timezone {
        buf.push(' ');
        match timezone {
            // The static label ("System local") would hide the actual offset;
            // print the offset that was applied instead.
            Timezone::SystemLocal => push_utc_offset_label(buf, offset),
            other => buf.push_str(other.label()),
        }
    }
}

/// Append "UTC", "UTC+3", "UTC−9:30", … matching the fixed-offset labels.
fn push_utc_offset_label(buf: &mut String, offset_seconds: i64) {
    use std::fmt::Write;

    buf.push_str("UTC");
    if offset_seconds == 0 {
        return;
    }
    let (sign, magnitude) = if offset_seconds < 0 {
        ('\u{2212}', -offset_seconds)
    } else {
        ('+', offset_seconds)
    };
    buf.push(sign);
    let hours = magnitude / 3600;
    let minutes = (magnitude % 3600) / 60;
    let _ = write!(buf, "{hours}");
    if minutes != 0 {
        let _ = write!(buf, ":{minutes:02}");
    }
}

/// Backward-compatible wrapper that formats in UTC.
#[cfg(test)]
pub(super) fn format_datetime_utc(time: std::time::SystemTime, format: DateTimeFormat) -> String {
    format_datetime(time, format, Timezone::Utc, true)
}

/// Convert unix seconds to a `SystemTime`, handling pre-epoch values.
pub(super) fn system_time_from_unix(secs: i64) -> std::time::SystemTime {
    use std::time::{Duration, UNIX_EPOCH};
    if secs >= 0 {
        UNIX_EPOCH + Duration::from_secs(secs as u64)
    } else {
        UNIX_EPOCH - Duration::from_secs(secs.unsigned_abs())
    }
}

/// Format a unix timestamp (seconds) as a coarse relative duration such as
/// `just now`, `30 mins ago`, `2 hours ago`, `5 months ago`.
///
/// Used by the blame/annotate column. Future or zero deltas render as
/// `just now`. The breakpoints intentionally favour readable approximations
/// (30-day months, 365-day years) over calendar accuracy.
pub(super) fn format_relative_time(unix_secs: i64, now: std::time::SystemTime) -> String {
    use std::time::UNIX_EPOCH;

    let now_secs = match now.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64),
    };

    // `unix_secs` is an untrusted i64 straight from a git author timestamp; a
    // corrupt/crafted commit near i64::MIN would overflow a plain subtraction
    // (debug-build panic, release wrap). Saturate so the worst case is a
    // clamped-but-finite delta.
    let delta = now_secs.saturating_sub(unix_secs);
    if delta < 10 {
        return "just now".to_string();
    }

    fn unit(value: i64, singular: &str, plural: &str) -> String {
        if value == 1 {
            format!("1 {singular} ago")
        } else {
            format!("{value} {plural} ago")
        }
    }

    let mins = delta / 60;
    let hours = delta / 3_600;
    let days = delta / 86_400;

    if delta < 60 {
        unit(delta, "sec", "secs")
    } else if mins < 60 {
        unit(mins, "min", "mins")
    } else if hours < 24 {
        unit(hours, "hour", "hours")
    } else if days < 7 {
        unit(days, "day", "days")
    } else if days < 30 {
        unit(days / 7, "week", "weeks")
    } else if days < 365 {
        unit(days / 30, "month", "months")
    } else {
        unit(days / 365, "year", "years")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn date_time_format_keys_round_trip_and_labels_are_unique() {
        let mut seen_labels = HashSet::new();

        for &format in DateTimeFormat::all() {
            assert_eq!(DateTimeFormat::from_key(format.key()), Some(format));
            assert!(
                seen_labels.insert(format.label()),
                "date-time format labels should stay unique"
            );
        }
    }

    #[test]
    fn timezone_keys_round_trip_for_all_supported_offsets() {
        for &timezone in Timezone::all() {
            let key = timezone.key();
            assert_eq!(Timezone::from_key(&key), Some(timezone));
            assert_eq!(
                Timezone::from_key(&key).map(|tz| tz.offset_seconds_at(0)),
                Some(timezone.offset_seconds_at(0))
            );
        }

        assert_eq!(Timezone::from_key("fixed_not_a_number"), None);
    }

    #[test]
    fn system_local_is_the_default_and_formats_a_resolved_offset_suffix() {
        assert_eq!(Timezone::default(), Timezone::SystemLocal);

        // The concrete offset depends on the machine; the suffix must be the
        // resolved "UTC±X" form, never the static "System local" label.
        let formatted = format_datetime(
            UNIX_EPOCH,
            DateTimeFormat::YmdHm,
            Timezone::SystemLocal,
            true,
        );
        let suffix = formatted
            .split_once(" UTC")
            .map(|(_, rest)| rest)
            .expect("expected a UTC-offset suffix");
        assert!(
            suffix.is_empty() || suffix.starts_with('+') || suffix.starts_with('\u{2212}'),
            "unexpected suffix in {formatted:?}"
        );
    }

    #[test]
    fn format_datetime_into_reuses_buffer_and_clears_previous_suffix() {
        let mut buf = String::from("stale-data");

        format_datetime_into(
            &mut buf,
            UNIX_EPOCH,
            DateTimeFormat::YmdHm,
            Timezone::Fixed(2 * 3600),
            true,
        );
        assert_eq!(buf, "1970-01-01 02:00 UTC+2");

        format_datetime_into(
            &mut buf,
            UNIX_EPOCH,
            DateTimeFormat::DmyHm,
            Timezone::Utc,
            false,
        );
        assert_eq!(buf, "01.01.1970 00:00");
    }

    #[test]
    fn format_datetime_handles_negative_epoch_and_day_rollover() {
        let before_epoch = UNIX_EPOCH - Duration::from_secs(1);

        assert_eq!(
            format_datetime(before_epoch, DateTimeFormat::YmdHms, Timezone::Utc, true),
            "1969-12-31 23:59:59 UTC"
        );
        assert_eq!(
            format_datetime(
                before_epoch,
                DateTimeFormat::YmdHms,
                Timezone::Fixed(3600),
                true
            ),
            "1970-01-01 00:59:59 UTC+1"
        );
    }

    #[test]
    fn format_datetime_supports_fractional_hour_offsets() {
        assert_eq!(
            format_datetime(
                UNIX_EPOCH,
                DateTimeFormat::YmdHm,
                Timezone::Fixed(5 * 3600 + 45 * 60),
                true
            ),
            "1970-01-01 05:45 UTC+5:45"
        );
        assert_eq!(
            format_datetime(
                UNIX_EPOCH,
                DateTimeFormat::MdyHm,
                Timezone::Fixed(-3 * 3600 - 30 * 60),
                true
            ),
            format!("12/31/1969 20:30 UTC\u{2212}3:30")
        );
    }

    #[test]
    fn format_relative_time_covers_all_breakpoints() {
        let now = UNIX_EPOCH + Duration::from_secs(10_000_000_000);
        let at = |secs_ago: i64| {
            let now_secs = 10_000_000_000_i64;
            format_relative_time(now_secs - secs_ago, now)
        };

        assert_eq!(at(0), "just now");
        assert_eq!(at(5), "just now");
        assert_eq!(at(30), "30 secs ago");
        assert_eq!(at(60), "1 min ago");
        assert_eq!(at(120), "2 mins ago");
        assert_eq!(at(3_600), "1 hour ago");
        assert_eq!(at(7_200), "2 hours ago");
        assert_eq!(at(86_400), "1 day ago");
        assert_eq!(at(3 * 86_400), "3 days ago");
        assert_eq!(at(7 * 86_400), "1 week ago");
        assert_eq!(at(30 * 86_400), "1 month ago");
        assert_eq!(at(60 * 86_400), "2 months ago");
        assert_eq!(at(365 * 86_400), "1 year ago");
        assert_eq!(at(800 * 86_400), "2 years ago");
    }

    #[test]
    fn format_relative_time_treats_future_as_just_now() {
        let now = UNIX_EPOCH + Duration::from_secs(1_000);
        assert_eq!(format_relative_time(5_000, now), "just now");
    }

    #[test]
    fn format_relative_time_saturates_on_extreme_timestamps() {
        // `unix_secs` is an untrusted git author timestamp. A plain
        // `now_secs - unix_secs` would overflow i64 at the extremes (debug panic
        // / release wrap); saturating arithmetic must keep it finite.
        let now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        // i64::MIN in the past -> enormous positive delta -> "years ago".
        assert!(
            format_relative_time(i64::MIN, now).ends_with("years ago"),
            "extreme-past timestamp must not panic and reads as long ago"
        );
        // i64::MAX in the future -> negative delta saturates -> "just now".
        assert_eq!(format_relative_time(i64::MAX, now), "just now");
    }

    #[test]
    fn format_datetime_clamps_out_of_range_years() {
        // ~year 11476 — past the 4-digit field. Must clamp to 9999 rather than
        // render a misleading truncated year (e.g. 11476 -> "1476") or panic.
        let far_future = UNIX_EPOCH + Duration::from_secs(300_000_000_000);
        let s = format_datetime(far_future, DateTimeFormat::YmdHm, Timezone::Utc, false);
        assert!(s.starts_with("9999-"), "got {s:?}");

        // A negative civil year (pre-year-1) must clamp to 0000, not drop its
        // sign via unsigned_abs (e.g. -44 -> "0044").
        // Windows SystemTime only goes back to 1601-01-01, so this assertion
        // is only meaningful on platforms that can represent pre-epoch times.
        #[cfg(not(windows))]
        {
            let far_past = UNIX_EPOCH - Duration::from_secs(100_000_000_000);
            let s = format_datetime(far_past, DateTimeFormat::YmdHm, Timezone::Utc, false);
            assert!(s.starts_with("0000-"), "got {s:?}");
        }
        // On Windows, verify clamping with a safe pre-epoch value.
        #[cfg(windows)]
        {
            let far_past = UNIX_EPOCH - Duration::from_secs(11_600_000_000);
            let s = format_datetime(far_past, DateTimeFormat::YmdHm, Timezone::Utc, false);
            assert!(s.starts_with("160"), "expected early-1600s year, got {s:?}");
        }
    }
}
