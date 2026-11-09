use super::{MarkdownTaskMarker, toggle_task_marker, toggle_task_marker_in_format};
use crate::view::markdown_preview::parse_markdown;
use gitcomet_core::text_format::{TextEncoding, TextFormat, decode, encode};

fn task_for(source: &str, text: &str) -> MarkdownTaskMarker {
    let doc = parse_markdown(source).expect("parses");
    doc.rows
        .iter()
        .find(|row| row.text.as_ref() == text)
        .and_then(|row| row.task)
        .expect("task row")
}

/// Toggle the task on the row reading `text`, as a click on it would.
fn click(source: &str, text: &str) -> Option<String> {
    toggle_task_marker(source.as_bytes().to_vec(), task_for(source, text))
}

#[test]
fn encoded_task_toggles_reject_lossy_or_malformed_content() {
    for (encoding, bytes) in [
        ("shift_jis", b"\x87\x90\n- [ ] task\n".as_slice()),
        ("gb18030", b"\x80\n- [ ] task\n".as_slice()),
        ("utf-8", b"\xff\n- [ ] task\n".as_slice()),
    ] {
        let format = TextFormat {
            encoding: TextEncoding::from_label(encoding).unwrap(),
            bom: false,
        };
        let source = decode(bytes, format);
        let task = task_for(&source.text, "task");
        assert!(toggle_task_marker_in_format(bytes.to_vec(), task, format).is_none());
    }
}

#[test]
fn encoded_task_toggles_preserve_other_bytes_and_boms() {
    for (encoding, bom, source, task_text) in [
        ("utf-8", false, "préface\r\n- [ ] café\r\n", "café"),
        ("utf-8", true, "préface\r\n- [ ] café\r\n", "café"),
        ("utf-16le", true, "préface\r\n- [ ] café\r\n", "café"),
        ("windows-1252", false, "préface\r\n- [ ] café\r\n", "café"),
        ("shift_jis", false, "前文\r\n- [ ] 項目\r\n", "項目"),
        ("gb18030", false, "€\r\n- [ ] task\r\n", "task"),
    ] {
        let format = TextFormat {
            encoding: TextEncoding::from_label(encoding).unwrap(),
            bom,
        };
        let bytes = encode(source, format).unwrap().into_owned();
        let task = task_for(source, task_text);
        let (text, written) = toggle_task_marker_in_format(bytes.clone(), task, format)
            .expect("lossless task toggles");
        assert_eq!(text, source.replace("[ ]", "[x]"));
        assert_eq!(written.as_bytes(), encode(&text, format).unwrap().as_ref());
        let (_, restored) = toggle_task_marker_in_format(
            written.as_bytes().to_vec(),
            task_for(&text, task_text),
            format,
        )
        .expect("toggle back");
        assert_eq!(restored.as_bytes(), bytes);
    }
}

#[test]
fn toggling_flips_only_the_marker_byte() {
    let source =
        "# Todo\r\n\r\n- [ ] write\r\n  - [X] nested\r\n1. [x] ordered\r\n> * [ ] quoted ✓\n";
    assert_eq!(
        click(source, "write").as_deref(),
        Some(source.replacen("[ ] write", "[x] write", 1).as_str())
    );
    assert_eq!(
        click(source, "nested").as_deref(),
        Some(source.replacen("[X]", "[ ]", 1).as_str())
    );
    assert_eq!(
        click(source, "ordered").as_deref(),
        Some(source.replacen("[x] ordered", "[ ] ordered", 1).as_str())
    );
    assert_eq!(
        click(source, "quoted ✓").as_deref(),
        Some(source.replacen("[ ] quoted", "[x] quoted", 1).as_str())
    );
}

#[test]
fn toggling_twice_restores_the_file() {
    let source = "- [ ] a\n- [x] b\n";
    let once = click(source, "a").expect("toggles");
    assert_eq!(click(&once, "a").as_deref(), Some(source));
}

#[test]
fn a_stale_offset_refuses_to_write() {
    let source = "- [ ] a\n- [ ] b\n";
    let doc = parse_markdown(source).expect("parses");
    let task = doc.rows[1].task.expect("task row");
    // The file gained a line above the item since the preview parsed it.
    let edited = format!("intro\n{source}");
    assert_eq!(toggle_task_marker(edited.into_bytes(), task), None);
    // The file was checked elsewhere since the preview parsed it.
    let checked = source.replacen("[ ] b", "[x] b", 1);
    assert_eq!(toggle_task_marker(checked.into_bytes(), task), None);
    // The file shrank past the marker.
    assert_eq!(toggle_task_marker(b"- [".to_vec(), task), None);
}
