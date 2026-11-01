//! The display boundary for source rows, wrapped rows and streamed slices.
//! Keep source text intact for copying, searching and whitespace markers.
use super::*;
use std::borrow::Cow;

pub(in crate::view) const DIFF_TEXT_DERIVED_CACHE_MAX_ENTRIES: usize = 8_192;
// Entry count bounds metadata; this separately bounds retained strings and
// highlights, including the source owners of identity-keyed fallback rows.
const DISPLAY_CACHE_MAX_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum PlainTextKey {
    Shared { address: usize, len: usize },
    Inline(SharedString),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum DisplayCacheKey {
    Styled {
        text_hash: u64,
        highlights_hash: u64,
        start: usize,
        end: usize,
        ends_at_line_end: bool,
    },
    Plain(PlainTextKey),
}

struct PlainDisplay {
    // Retain the immutable allocation used by Shared keys so its address
    // cannot be recycled while the cache still contains the key.
    source: SharedString,
    text: SharedString,
    text_hash: OnceLock<u64>,
}

enum CachedDisplay {
    // LF rows need only this marker: no duplicate string or source retention.
    Unchanged,
    Styled(CachedDiffStyledText),
    Plain(PlainDisplay),
}

impl CachedDisplay {
    fn retained_bytes(&self) -> usize {
        match self {
            Self::Unchanged => 0,
            Self::Styled(styled) => styled
                .text
                .len()
                .saturating_add(std::mem::size_of_val(styled.highlights.as_ref())),
            Self::Plain(plain) => {
                plain
                    .source
                    .len()
                    .saturating_add(if plain.source.as_ptr() == plain.text.as_ptr() {
                        0
                    } else {
                        plain.text.len()
                    })
            }
        }
    }
}

struct DisplayCache {
    entries: FxLruCache<DisplayCacheKey, CachedDisplay>,
    retained_bytes: usize,
    max_entries: usize,
    max_bytes: usize,
}

impl DisplayCache {
    fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            entries: new_fx_lru_cache(max_entries),
            retained_bytes: 0,
            max_entries,
            max_bytes,
        }
    }

    fn insert(&mut self, key: DisplayCacheKey, value: CachedDisplay) {
        let bytes = value.retained_bytes();
        if bytes > self.max_bytes {
            return;
        }
        // Pre-evict both limits so the LRU cannot discard an unaccounted value.
        while self.entries.len() >= self.max_entries
            || self.retained_bytes.saturating_add(bytes) > self.max_bytes
        {
            let Some((_, evicted)) = self.entries.pop_lru() else {
                break;
            };
            self.retained_bytes -= evicted.retained_bytes();
        }
        if let Some(replaced) = self.entries.put(key, value) {
            self.retained_bytes -= replaced.retained_bytes();
        }
        self.retained_bytes += bytes;
    }
}

thread_local! {
    static DISPLAY_SLICE_CACHE: RefCell<DisplayCache> =
        RefCell::new(DisplayCache::new(DIFF_TEXT_DERIVED_CACHE_MAX_ENTRIES, DISPLAY_CACHE_MAX_BYTES));
}

#[cfg(test)]
thread_local! {
    static DISPLAY_PREPARATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static DISPLAY_CONTENT_HASHES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn prepare_display(text: &str, ends_at_line_end: bool) -> Cow<'_, str> {
    #[cfg(test)]
    DISPLAY_PREPARATIONS.with(|count| count.set(count.get() + 1));
    crate::text_runs::single_line_display_text(text, ends_at_line_end)
}

fn display_content_hash(text: &str) -> u64 {
    #[cfg(test)]
    DISPLAY_CONTENT_HASHES.with(|count| count.set(count.get() + 1));
    hash_text_content(text)
}

pub(in crate::view) fn line_display_text(text: &str) -> Cow<'_, str> {
    crate::text_runs::single_line_display_text(text, true)
}

fn line_display_styled_text(styled: &CachedDiffStyledText) -> CachedDiffStyledText {
    display_styled_slice(styled, 0..styled.text.len(), true)
}

fn with_plain_display<T>(text: SharedString, consume: impl FnOnce(&PlainDisplay) -> T) -> T {
    let owner = text.clone();
    // Clones share heap/static storage, but small inline strings move. Detect
    // that without assuming a particular SharedString inline capacity. Inline
    // keys hash only their bounded inline bytes; heap keys never hash content.
    let source_key = if owner.as_ptr() == text.as_ptr() {
        PlainTextKey::Shared {
            address: text.as_ptr() as usize,
            len: text.len(),
        }
    } else {
        PlainTextKey::Inline(text.clone())
    };
    let key = DisplayCacheKey::Plain(source_key);
    DISPLAY_SLICE_CACHE.with_borrow_mut(|cache| {
        if let Some(CachedDisplay::Plain(hit)) = cache.entries.get(&key) {
            return consume(hit);
        }
        let display = prepare_display(&text, true);
        let output = match display {
            Cow::Borrowed(display) if display.len() == text.len() => text.clone(),
            // Cow -> SharedString avoids a temporary String for trimmed CRLF.
            display => display.into(),
        };
        let plain = PlainDisplay {
            source: owner,
            text: output,
            text_hash: OnceLock::new(),
        };
        let result = consume(&plain);
        cache.insert(key, CachedDisplay::Plain(plain));
        result
    })
}

/// Selectable fallback rows do not need a content hash at all.
pub(in crate::view) fn display_plain_text(text: SharedString) -> SharedString {
    with_plain_display(text, |plain| plain.text.clone())
}

/// Canvases need a layout hash; compute it once, and only when requested.
pub(in crate::view) fn display_diff_text(
    styled: Option<&CachedDiffStyledText>,
    text: SharedString,
) -> CachedDiffStyledText {
    match styled {
        Some(styled) => line_display_styled_text(styled),
        None => with_plain_display(text, |plain| CachedDiffStyledText {
            text: plain.text.clone(),
            text_hash: *plain
                .text_hash
                .get_or_init(|| display_content_hash(&plain.text)),
            highlights: super::super::canvas_text::empty_highlights(),
            highlights_hash: 0,
        }),
    }
}

/// Ranges address the source. An interior slice-ending CR is a blank source
/// cell, not a terminator. Text, highlights and layout hashes are prepared
/// together, only on a cache miss. Unchanged full rows retain just a marker.
pub(in crate::view) fn display_styled_slice(
    styled: &CachedDiffStyledText,
    range: Range<usize>,
    ends_at_line_end: bool,
) -> CachedDiffStyledText {
    let start = clamp_to_char_boundary(&styled.text, range.start.min(styled.text.len()));
    let end = clamp_to_char_boundary(&styled.text, range.end.min(styled.text.len())).max(start);
    let key = DisplayCacheKey::Styled {
        text_hash: styled.text_hash,
        highlights_hash: styled.highlights_hash,
        start,
        end,
        ends_at_line_end,
    };
    DISPLAY_SLICE_CACHE.with_borrow_mut(|cache| {
        if let Some(hit) = cache.entries.get(&key) {
            return match hit {
                CachedDisplay::Unchanged => styled.clone(),
                CachedDisplay::Styled(styled) => styled.clone(),
                CachedDisplay::Plain(_) => unreachable!("different key variant"),
            };
        }
        let display = prepare_display(&styled.text[start..end], ends_at_line_end);
        if matches!(&display, Cow::Borrowed(_)) && start == 0 && display.len() == styled.text.len()
        {
            cache.insert(key, CachedDisplay::Unchanged);
            return styled.clone();
        }
        let display_end = start + display.len();
        let (highlights, highlights_hash) = if start == 0
            && styled
                .highlights
                .iter()
                .all(|(range, _)| range.end <= display_end)
        {
            (Arc::clone(&styled.highlights), styled.highlights_hash)
        } else {
            let highlights: Vec<_> = styled
                .highlights
                .iter()
                .filter_map(|(range, style)| {
                    let a = range.start.max(start);
                    let b = range.end.min(display_end);
                    (a < b).then_some((a.saturating_sub(start)..b.saturating_sub(start), *style))
                })
                .collect();
            let hash = hash_visible_highlights(&highlights);
            let highlights = if highlights.is_empty() {
                super::super::canvas_text::empty_highlights()
            } else {
                highlights.into()
            };
            (highlights, hash)
        };
        let value = CachedDiffStyledText {
            text_hash: display_content_hash(&display),
            text: display.into(),
            highlights_hash,
            highlights,
        };
        cache.insert(key, CachedDisplay::Styled(value.clone()));
        value
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::rows::canvas_text::empty_highlights;

    #[test]
    fn cr_display_slices_preserve_offsets_and_share_the_cache() {
        // Exceed SharedString's inline capacity so pointer identity actually
        // checks allocation reuse instead of comparing two inline buffers.
        let prefix = "abcdefghijklmnopqrstuvwxyz012345";
        let raw: SharedString = format!("{prefix}\rdelta\r").into();
        let source = build_cached_diff_styled_text_from_relative_highlights(4, &raw, &[]);
        let full = line_display_styled_text(&source);
        assert_eq!(full.text.as_ref(), format!("{prefix} delta"));
        let plain = display_diff_text(None, raw.clone());
        assert_eq!(
            plain.text.as_ptr(),
            display_diff_text(None, raw.clone()).text.as_ptr(),
            "plain rows reuse their allocation without hashing the source"
        );
        assert_eq!(plain.text, full.text);
        assert_eq!(plain.text_hash, full.text_hash);
        let first = display_styled_slice(&source, 0..33, false);
        assert_eq!(
            first.text.as_ref(),
            format!("{prefix} "),
            "slice-ending interior CR keeps its source cell"
        );
        assert_eq!(
            display_styled_slice(&source, 33..39, true).text.as_ref(),
            "delta"
        );
        assert_eq!(
            display_styled_slice(&source, 0..33, false).text.as_ptr(),
            first.text.as_ptr()
        );
        assert_eq!(
            source.text, raw,
            "source remains available for copying and markers"
        );
        for text in ["a\rb", "a\rb\rc\r", "a\rb\r"] {
            assert!(!display_diff_text(None, text.into()).text.contains('\r'));
        }
    }

    fn reset_cache() {
        DISPLAY_SLICE_CACHE.with_borrow_mut(|cache| {
            *cache =
                DisplayCache::new(DIFF_TEXT_DERIVED_CACHE_MAX_ENTRIES, DISPLAY_CACHE_MAX_BYTES);
        });
        DISPLAY_PREPARATIONS.with(|count| count.set(0));
        DISPLAY_CONTENT_HASHES.with(|count| count.set(0));
    }

    #[test]
    fn display_repaints_do_not_rescan_or_rehash_rows() {
        reset_cache();
        let rows: Vec<SharedString> = ["", "\r", "\rinterior\r"]
            .into_iter()
            .map(|end| format!("{}{end}", "x".repeat(40 * 1024)).into())
            .collect();
        let styled: Vec<_> = rows
            .iter()
            .map(|text| build_cached_diff_styled_text_from_relative_highlights(4, text, &[]))
            .collect();
        for text in &rows {
            display_plain_text(text.clone());
        }
        assert_eq!(
            DISPLAY_CONTENT_HASHES.with(|count| count.get()),
            0,
            "plain selectable rows never need a layout hash, even cold"
        );
        for (text, styled) in rows.iter().zip(&styled) {
            display_diff_text(None, text.clone());
            line_display_styled_text(styled);
            display_styled_slice(styled, 0..128, false);
        }
        let preparations = DISPLAY_PREPARATIONS.with(|count| count.get());
        let hashes = DISPLAY_CONTENT_HASHES.with(|count| count.get());
        for _ in 0..100 {
            for (text, styled) in rows.iter().zip(&styled) {
                display_plain_text(text.clone());
                display_diff_text(None, text.clone());
                line_display_styled_text(styled);
                display_styled_slice(styled, 0..128, false);
            }
        }
        assert_eq!(DISPLAY_PREPARATIONS.with(|count| count.get()), preparations);
        assert_eq!(DISPLAY_CONTENT_HASHES.with(|count| count.get()), hashes);
        let lf = line_display_styled_text(&styled[0]);
        assert_eq!(lf.text.as_ptr(), styled[0].text.as_ptr());
        assert!(Arc::ptr_eq(&lf.highlights, &styled[0].highlights));
    }

    #[test]
    fn display_cache_bounds_large_rows_and_retained_plain_sources() {
        reset_cache();
        for ix in 0..300 {
            let raw = format!("{ix:08}{}\r", "x".repeat(40 * 1024 - 8));
            let styled = build_cached_diff_styled_text_from_relative_highlights(4, &raw, &[]);
            line_display_styled_text(&styled);
            DISPLAY_SLICE_CACHE
                .with_borrow(|cache| assert!(cache.retained_bytes <= DISPLAY_CACHE_MAX_BYTES));
        }
        DISPLAY_SLICE_CACHE.with_borrow(|cache| {
            assert_eq!(
                cache.entries.len(),
                204,
                "8 MiB holds at most 204 distinct 40 KiB display copies"
            );
            assert_eq!(cache.retained_bytes, 204 * 40 * 1024);
        });
        reset_cache();
        for ix in 0..300 {
            let raw: SharedString = format!("{ix:08}{}\r", "x".repeat(40 * 1024 - 8)).into();
            display_plain_text(raw);
            DISPLAY_SLICE_CACHE
                .with_borrow(|cache| assert!(cache.retained_bytes <= DISPLAY_CACHE_MAX_BYTES));
        }
        DISPLAY_SLICE_CACHE.with_borrow(|cache| {
            assert_eq!(
                cache.entries.len(),
                102,
                "identity keys also retain and budget the original source"
            );
            assert_eq!(cache.retained_bytes, 102 * (2 * 40 * 1024 + 1));
        });
    }

    #[test]
    fn display_cache_bounds_highlights_oversized_entries_and_metadata() {
        let mut cache = DisplayCache::new(2, 100);
        let key = |ix| DisplayCacheKey::Styled {
            text_hash: ix,
            highlights_hash: 0,
            start: 0,
            end: 1,
            ends_at_line_end: true,
        };
        let value = |text: SharedString, highlights| {
            CachedDisplay::Styled(CachedDiffStyledText {
                text,
                highlights,
                highlights_hash: 0,
                text_hash: 0,
            })
        };
        cache.insert(key(1), value("x".repeat(80).into(), empty_highlights()));
        cache.insert(key(2), value("y".repeat(80).into(), empty_highlights()));
        assert_eq!(cache.entries.len(), 1);
        assert_eq!(cache.retained_bytes, 80);
        assert!(cache.entries.peek(&key(1)).is_none());
        cache.insert(key(3), value("z".repeat(101).into(), empty_highlights()));
        assert!(
            cache.entries.peek(&key(3)).is_none(),
            "oversized text is not retained"
        );
        cache.insert(
            key(4),
            value(
                "x".into(),
                vec![(0..1, gpui::HighlightStyle::default()); 100].into(),
            ),
        );
        assert!(
            cache.entries.peek(&key(4)).is_none(),
            "highlight storage counts against the same budget"
        );
        for ix in 5..10 {
            cache.insert(key(ix), CachedDisplay::Unchanged);
        }
        assert_eq!(cache.retained_bytes, 0);
        assert_eq!(
            cache.entries.len(),
            2,
            "unchanged markers still respect the entry cap"
        );
    }

    #[test]
    fn display_plain_keys_preserve_owners_and_distinguish_inline_text() {
        reset_cache();
        for text in ["a\rb", "abc", "def\r", "abc"] {
            assert_eq!(
                display_plain_text(text.into()).as_ref(),
                line_display_text(text)
            );
        }
        let raw: SharedString = format!("{}\r", "owned".repeat(20)).into();
        let address = raw.as_ptr() as usize;
        let len = raw.len();
        display_plain_text(raw);
        DISPLAY_SLICE_CACHE.with_borrow(|cache| {
            let Some(CachedDisplay::Plain(plain)) =
                cache
                    .entries
                    .peek(&DisplayCacheKey::Plain(PlainTextKey::Shared {
                        address,
                        len,
                    }))
            else {
                panic!("source owner must remain cached")
            };
            assert_eq!(plain.source.as_ptr() as usize, address);
            assert_eq!(plain.source.len(), len);
        });
    }
}

#[cfg(test)]
mod allocation_tests;
