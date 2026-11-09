use super::*;
#[test]
fn history_author_initials_are_cached_with_their_hash() {
    for (name, initials) in [
        ("", "?"),
        ("Élodie 李", "É李"),
        ("ßeta", "SSE"),
        ("Ada Lovelace", "AL"),
    ] {
        let author = HistoryAuthorVm::new(name.into());
        assert_eq!(author.initials.as_ref(), initials);
        assert_eq!(author.initials.text_hash(), history_text_hash(initials));
        let clone = author.clone();
        // Short SharedStrings are inline: cloning their cached value need
        // not preserve its address and still performs no recomputation.
        assert_eq!(clone.initials.shared(), author.initials.shared());
        assert_eq!(clone.initials.text_hash(), author.initials.text_hash());
    }
}
