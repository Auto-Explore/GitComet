#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum DateTimeFormat {
    YmdHm,
    YmdHms,
    DmyHm,
    MdyHm,
}

impl DateTimeFormat {
    pub fn all() -> &'static [DateTimeFormat] {
        &[
            DateTimeFormat::YmdHm,
            DateTimeFormat::YmdHms,
            DateTimeFormat::DmyHm,
            DateTimeFormat::MdyHm,
        ]
    }

    pub fn label(self) -> &'static str {
        match self {
            DateTimeFormat::YmdHm => "YYYY-MM-DD HH:MM",
            DateTimeFormat::YmdHms => "YYYY-MM-DD HH:MM:SS",
            DateTimeFormat::DmyHm => "DD.MM.YYYY HH:MM",
            DateTimeFormat::MdyHm => "MM/DD/YYYY HH:MM",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            DateTimeFormat::YmdHm => "ymd_hm_utc",
            DateTimeFormat::YmdHms => "ymd_hms_utc",
            DateTimeFormat::DmyHm => "dmy_hm_utc",
            DateTimeFormat::MdyHm => "mdy_hm_utc",
        }
    }

    pub fn from_key(s: &str) -> Option<Self> {
        match s {
            "ymd_hm_utc" => Some(DateTimeFormat::YmdHm),
            "ymd_hms_utc" => Some(DateTimeFormat::YmdHms),
            "dmy_hm_utc" => Some(DateTimeFormat::DmyHm),
            "mdy_hm_utc" => Some(DateTimeFormat::MdyHm),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub enum Timezone {
    /// The device's timezone, resolved per timestamp (DST-correct via jiff).
    #[default]
    SystemLocal,
    Utc,
    /// Fixed offset from UTC in seconds (positive = east of UTC).
    Fixed(i32),
}

/// A fixed-offset timezone choice.
struct FixedZone {
    /// Seconds east of UTC.
    offset: i32,
    label: &'static str,
    cities: &'static str,
}

const fn zone(offset: i32, label: &'static str, cities: &'static str) -> FixedZone {
    FixedZone {
        offset,
        label,
        cities,
    }
}

const FIXED_ZONES: [FixedZone; 37] = [
    zone(-12 * 3600, "UTC\u{2212}12", "Baker Island"),
    zone(-11 * 3600, "UTC\u{2212}11", "Pago Pago"),
    zone(-10 * 3600, "UTC\u{2212}10", "Honolulu"),
    zone(-9 * 3600 - 30 * 60, "UTC\u{2212}9:30", "Marquesas Islands"),
    zone(-9 * 3600, "UTC\u{2212}9", "Anchorage"),
    zone(-8 * 3600, "UTC\u{2212}8", "Los Angeles, Vancouver"),
    zone(-7 * 3600, "UTC\u{2212}7", "Denver, Phoenix"),
    zone(-6 * 3600, "UTC\u{2212}6", "Chicago, Mexico City"),
    zone(-5 * 3600, "UTC\u{2212}5", "New York, Toronto"),
    zone(-4 * 3600, "UTC\u{2212}4", "Santiago, Halifax"),
    zone(-3 * 3600 - 30 * 60, "UTC\u{2212}3:30", "St. John's"),
    zone(-3 * 3600, "UTC\u{2212}3", "São Paulo, Buenos Aires"),
    zone(-2 * 3600, "UTC\u{2212}2", "South Georgia"),
    zone(-3600, "UTC\u{2212}1", "Azores, Cape Verde"),
    zone(3600, "UTC+1", "Berlin, Paris, Lagos"),
    zone(2 * 3600, "UTC+2", "Helsinki, Cairo, Kyiv"),
    zone(3 * 3600, "UTC+3", "Moscow, Istanbul, Nairobi"),
    zone(3 * 3600 + 30 * 60, "UTC+3:30", "Tehran"),
    zone(4 * 3600, "UTC+4", "Dubai, Baku"),
    zone(4 * 3600 + 30 * 60, "UTC+4:30", "Kabul"),
    zone(5 * 3600, "UTC+5", "Karachi, Tashkent"),
    zone(5 * 3600 + 30 * 60, "UTC+5:30", "Mumbai, Delhi, Colombo"),
    zone(5 * 3600 + 45 * 60, "UTC+5:45", "Kathmandu"),
    zone(6 * 3600, "UTC+6", "Dhaka, Almaty"),
    zone(6 * 3600 + 30 * 60, "UTC+6:30", "Yangon"),
    zone(7 * 3600, "UTC+7", "Bangkok, Jakarta, Hanoi"),
    zone(8 * 3600, "UTC+8", "Singapore, Beijing, Taipei"),
    zone(8 * 3600 + 45 * 60, "UTC+8:45", "Eucla"),
    zone(9 * 3600, "UTC+9", "Tokyo, Seoul"),
    zone(9 * 3600 + 30 * 60, "UTC+9:30", "Adelaide"),
    zone(10 * 3600, "UTC+10", "Sydney, Melbourne"),
    zone(10 * 3600 + 30 * 60, "UTC+10:30", "Lord Howe Island"),
    zone(11 * 3600, "UTC+11", "Noumea, Solomon Islands"),
    zone(12 * 3600, "UTC+12", "Auckland, Fiji"),
    zone(12 * 3600 + 45 * 60, "UTC+12:45", "Chatham Islands"),
    zone(13 * 3600, "UTC+13", "Apia, Tongatapu"),
    zone(14 * 3600, "UTC+14", "Kiritimati"),
];

/// Every choice in picker order: system local, UTC, then fixed offsets west to east.
const ALL_TIMEZONES: [Timezone; FIXED_ZONES.len() + 2] = {
    let mut all = [Timezone::SystemLocal; FIXED_ZONES.len() + 2];
    all[1] = Timezone::Utc;
    let mut ix = 0;
    while ix < FIXED_ZONES.len() {
        all[ix + 2] = Timezone::Fixed(FIXED_ZONES[ix].offset);
        ix += 1;
    }
    all
};

impl Timezone {
    pub fn all() -> &'static [Timezone] {
        &ALL_TIMEZONES
    }

    fn fixed_zone(seconds: i32) -> Option<&'static FixedZone> {
        FIXED_ZONES.iter().find(|zone| zone.offset == seconds)
    }

    pub fn label(self) -> &'static str {
        match self {
            Timezone::SystemLocal => "System local",
            Timezone::Utc => "UTC",
            Timezone::Fixed(s) => Self::fixed_zone(s).map_or("UTC+?", |zone| zone.label),
        }
    }

    pub fn key(self) -> String {
        match self {
            Timezone::SystemLocal => "system_local".to_string(),
            Timezone::Utc => "utc".to_string(),
            Timezone::Fixed(s) => format!("fixed_{s}"),
        }
    }

    pub fn from_key(s: &str) -> Option<Self> {
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

    pub fn cities(self) -> &'static str {
        match self {
            Timezone::SystemLocal => "This device's timezone",
            Timezone::Utc => "London, Reykjavik",
            Timezone::Fixed(s) => Self::fixed_zone(s).map_or("", |zone| zone.cities),
        }
    }

    /// UTC offset in seconds for the given unix timestamp. `SystemLocal`
    /// resolves the device timezone per timestamp, so historical commits keep
    /// their correct DST offset.
    pub fn offset_seconds_at(self, unix_seconds: i64) -> i64 {
        match self {
            Timezone::SystemLocal => jiff::Timestamp::from_second(unix_seconds)
                .map(|ts| i64::from(jiff::tz::TimeZone::system().to_offset(ts).seconds()))
                .unwrap_or(0),
            Timezone::Utc => 0,
            Timezone::Fixed(s) => s as i64,
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn format_datetime(
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
pub fn format_datetime_into(
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
    let timestamp = jiff::Timestamp::from_second(secs).unwrap_or(if secs < 0 {
        jiff::Timestamp::MIN
    } else {
        jiff::Timestamp::MAX
    });
    let datetime = jiff::tz::Offset::UTC.to_datetime(timestamp);
    let (y, m, d) = (
        i32::from(datetime.year()),
        datetime.month() as u32,
        datetime.day() as u32,
    );
    let (hour, minute, second) = (
        datetime.hour() as u32,
        datetime.minute() as u32,
        datetime.second() as u32,
    );

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
#[cfg(any(test, feature = "test-support"))]
pub fn format_datetime_utc(time: std::time::SystemTime, format: DateTimeFormat) -> String {
    format_datetime(time, format, Timezone::Utc, true)
}

/// Convert unix seconds to a `SystemTime`, handling pre-epoch values.
pub fn system_time_from_unix(secs: i64) -> std::time::SystemTime {
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
pub fn format_relative_time(unix_secs: i64, now: std::time::SystemTime) -> String {
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
    fn format_datetime_preserves_gregorian_leap_year_boundaries() {
        for (seconds, expected) in [
            (-2_203_891_201_i64, "1900-02-28 23:59:59"),
            (-2_203_891_200, "1900-03-01 00:00:00"),
            (951_782_400, "2000-02-29 00:00:00"),
            (951_868_800, "2000-03-01 00:00:00"),
            (4_107_542_399, "2100-02-28 23:59:59"),
            (4_107_542_400, "2100-03-01 00:00:00"),
            (1_709_210_096, "2024-02-29 12:34:56"),
        ] {
            let duration = Duration::from_secs(seconds.unsigned_abs());
            let time = if seconds < 0 {
                UNIX_EPOCH - duration
            } else {
                UNIX_EPOCH + duration
            };
            assert_eq!(
                format_datetime(time, DateTimeFormat::YmdHms, Timezone::Utc, false),
                expected
            );
        }
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
            "12/31/1969 20:30 UTC\u{2212}3:30".to_string()
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
