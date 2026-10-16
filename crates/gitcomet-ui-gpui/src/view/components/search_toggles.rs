//! The kit search bar's toggles as the core search engine's options.
use gitcomet_core::text_search::TextSearchOptions;
use gitcomet_ui_kit::components::SearchToggles;

pub fn search_toggles(options: TextSearchOptions) -> SearchToggles {
    SearchToggles {
        match_case: options.match_case,
        whole_word: options.whole_word,
        regex: options.regex,
    }
}

pub fn search_options(toggles: SearchToggles) -> TextSearchOptions {
    TextSearchOptions {
        match_case: toggles.match_case,
        whole_word: toggles.whole_word,
        regex: toggles.regex,
    }
}
