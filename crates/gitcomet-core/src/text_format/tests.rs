use super::*;

fn resolve(bytes: &[u8], kind: SideKind, attributes: &TextAttributes) -> SideTextFormat {
    ContentSniff::of(bytes).resolve(kind, attributes, None)
}

fn plain(bytes: &[u8]) -> SideTextFormat {
    resolve(bytes, SideKind::Worktree, &TextAttributes::default())
}

fn encoded(text: &str, label: &str) -> Vec<u8> {
    let encoding = TextEncoding::from_label(label).unwrap();
    encode(
        text,
        TextFormat {
            encoding,
            bom: false,
        },
    )
    .unwrap()
    .into_owned()
}

#[test]
fn ascii_and_utf8_resolve_to_utf8() {
    assert_eq!(plain(b"hello\n").format, TextFormat::UTF_8);
    assert_eq!(plain("päivää\n".as_bytes()).source, FormatSource::Utf8);
    assert_eq!(plain(b"").format, TextFormat::UTF_8);
}

#[test]
fn staged_latin1_repro_byte_is_not_utf8() {
    // `test (1).txt`: a single 0xE9.
    let format = plain(b"\xe9");
    assert_ne!(format.format.encoding, TextEncoding::UTF_8);
    assert!(!format.binary);
    let decoded = decode_bytes(
        b"\xe9",
        SideKind::Worktree,
        &TextAttributes::default(),
        None,
    );
    assert_eq!(decoded.text, "é");
}

#[test]
fn detector_picks_the_script() {
    let finnish = encoded(
        "Hyvää päivää! Tämä on lyhyt suomenkielinen teksti, jossa on ääkkösiä.\n",
        "windows-1252",
    );
    assert_eq!(plain(&finnish).format.encoding, TextEncoding::WINDOWS_1252);

    let russian = encoded(
        "Привет, мир! Это короткий текст на русском языке для проверки кодировки.\n",
        "windows-1251",
    );
    assert_eq!(plain(&russian).format.encoding.name(), "Windows-1251");

    let japanese = encoded(
        "こんにちは世界。これは文字コードの判定を確認するための日本語の文章です。\n",
        "shift_jis",
    );
    assert_eq!(plain(&japanese).format.encoding.name(), "Shift_JIS");
}

#[test]
fn bom_wins_over_detection() {
    let format = plain(b"\xEF\xBB\xBFhi");
    assert_eq!(format.format.encoding, TextEncoding::UTF_8);
    assert!(format.format.bom);
    assert_eq!(format.source, FormatSource::Bom);

    let format = plain(b"\xFE\xFF\x00h\x00i");
    assert_eq!(format.format.encoding, TextEncoding::UTF_16BE);
    assert!(format.format.bom);
}

#[test]
fn utf32_boms_are_rejected_before_utf16_detection_or_overrides() {
    for bytes in [
        b"\xFF\xFE\x00\x00a\x00\x00\x00\n\x00\x00\x00".as_slice(),
        b"\x00\x00\xFE\xFF\x00\x00\x00a\x00\x00\x00\n".as_slice(),
    ] {
        assert_eq!(TextEncoding::for_bom(bytes), None);
        for attributes in [
            TextAttributes::default(),
            TextAttributes {
                encoding: Some(EncodingAttr::from_label("UTF-16LE")),
                ..TextAttributes::default()
            },
            TextAttributes {
                working_tree_encoding: Some(EncodingAttr::from_label("UTF-16LE")),
                ..TextAttributes::default()
            },
        ] {
            for kind in [SideKind::Worktree, SideKind::GitInternal] {
                for override_encoding in [
                    None,
                    Some(TextEncoding::UTF_16LE),
                    Some(TextEncoding::UTF_16BE),
                ] {
                    let decoded = decode_bytes(bytes, kind, &attributes, override_encoding);
                    assert!(decoded.format.binary);
                    assert!(!decoded.format.is_writable());
                    assert_eq!(decoded.format.source, FormatSource::Binary);
                }
            }
        }
        // Recognize the four-byte BOM even when reads split it into pieces.
        for chunk_size in 1..=bytes.len() {
            let mut sniffer = ContentSniffer::new();
            for chunk in bytes.chunks(chunk_size) {
                sniffer.feed(chunk);
            }
            let sniff = sniffer.finish();
            assert_eq!(sniff.bom, None);
            assert!(
                sniff
                    .resolve(SideKind::Worktree, &TextAttributes::default(), None)
                    .binary
            );
        }
    }
    let utf16 = decode_bytes(
        b"\xFF\xFEa\x00\n\x00",
        SideKind::Worktree,
        &TextAttributes::default(),
        None,
    );
    assert_eq!(utf16.text, "a\n");
    assert!(utf16.format.is_writable());
    assert_eq!(utf16.format.format.encoding, TextEncoding::UTF_16LE);
}

#[test]
fn bomless_ascii_utf16_is_found_before_utf8() {
    // Valid UTF-8 byte-wise (NUL is valid), so this must be sniffed first.
    let utf16 = encode(
        "line one\r\nline two\r\n",
        TextFormat {
            encoding: TextEncoding::UTF_16LE,
            bom: false,
        },
    )
    .unwrap()
    .into_owned();
    let decoded = decode_bytes(&utf16, SideKind::Worktree, &TextAttributes::default(), None);
    assert_eq!(decoded.format.format.encoding, TextEncoding::UTF_16LE);
    assert_eq!(decoded.text, "line one\r\nline two\r\n");
    assert_eq!(decoded.format.line_endings.crlf, 2);
}

#[test]
fn nul_bytes_that_are_not_utf16_are_binary() {
    let format = plain(b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\xff\xd8");
    assert!(format.binary);
    // Valid UTF-8 with a NUL keeps showing as text, as before.
    assert!(!plain(b"a\x00b").binary);
}

#[test]
fn working_tree_encoding_reads_both_forms() {
    let attributes = TextAttributes {
        working_tree_encoding: Some(EncodingAttr::from_label("UTF-16LE-BOM")),
        ..TextAttributes::default()
    };
    let worktree = encode(
        "é\n",
        TextFormat {
            encoding: TextEncoding::UTF_16LE,
            bom: true,
        },
    )
    .unwrap()
    .into_owned();
    let format = resolve(&worktree, SideKind::Worktree, &attributes);
    assert_eq!(format.format.encoding, TextEncoding::UTF_16LE);
    assert!(format.format.bom);
    assert_eq!(format.source, FormatSource::WorkingTreeEncoding);

    let git = resolve("é\n".as_bytes(), SideKind::GitInternal, &attributes);
    assert_eq!(git.format, TextFormat::UTF_8);
    assert_eq!(git.source, FormatSource::GitInternalUtf8);

    // A blob committed before the attribute existed is still raw UTF-16.
    let old_blob = resolve(&worktree, SideKind::GitInternal, &attributes);
    assert_eq!(old_blob.format.encoding, TextEncoding::UTF_16LE);
}

#[test]
fn override_applies_except_to_git_form_of_working_tree_encoding_files() {
    let sniff = ContentSniff::of("päivää".as_bytes());
    let cp1252 = Some(TextEncoding::WINDOWS_1252);
    let plain_attributes = TextAttributes::default();
    let format = sniff.resolve(SideKind::GitInternal, &plain_attributes, cp1252);
    assert_eq!(format.format.encoding, TextEncoding::WINDOWS_1252);
    assert_eq!(format.source, FormatSource::Override);

    let wte = TextAttributes {
        working_tree_encoding: Some(EncodingAttr::from_label("ISO-8859-1")),
        ..TextAttributes::default()
    };
    let format = sniff.resolve(SideKind::GitInternal, &wte, cp1252);
    assert_eq!(format.format, TextFormat::UTF_8);
    let format = sniff.resolve(SideKind::Worktree, &wte, cp1252);
    assert_eq!(format.source, FormatSource::Override);
}

#[test]
fn encoding_attribute_beats_detection_and_gui_encoding_only_fills_in() {
    let attributes = TextAttributes {
        encoding: Some(EncodingAttr::from_label("koi8-r")),
        gui_encoding: Some(EncodingAttr::from_label("cp1250")),
        ..TextAttributes::default()
    };
    // Even valid UTF-8 follows an explicit per-path attribute.
    let format = resolve(b"plain", SideKind::Worktree, &attributes);
    assert_eq!(format.format.encoding.name(), "KOI8-R");
    assert_eq!(format.source, FormatSource::EncodingAttribute);

    let gui_only = TextAttributes {
        gui_encoding: Some(EncodingAttr::from_label("cp1250")),
        ..TextAttributes::default()
    };
    assert_eq!(
        resolve(b"plain", SideKind::Worktree, &gui_only).format,
        TextFormat::UTF_8
    );
    let format = resolve(b"\xe9", SideKind::Worktree, &gui_only);
    assert_eq!(format.format.encoding.name(), "Windows-1250");
    assert_eq!(format.source, FormatSource::GuiEncoding);
}

#[test]
fn sniffing_in_chunks_matches_one_pass() {
    let text = encoded(
        &"Grüße aus Köln, schöne Straße. ".repeat(4000),
        "windows-1252",
    );
    let whole = ContentSniff::of(&text);
    for chunk_len in [1, 2, 3, 7, 4096] {
        let mut sniffer = ContentSniffer::new();
        for chunk in text.chunks(chunk_len) {
            sniffer.feed(chunk);
        }
        let chunked = sniffer.finish();
        assert_eq!(chunked.utf8_valid, whole.utf8_valid, "{chunk_len}");
        assert_eq!(chunked.has_nul, whole.has_nul);
        assert_eq!(chunked.line_endings, whole.line_endings);
        assert_eq!(
            chunked
                .resolve(SideKind::Worktree, &TextAttributes::default(), None)
                .format,
            whole
                .resolve(SideKind::Worktree, &TextAttributes::default(), None)
                .format
        );
    }
}

#[test]
fn utf8_split_mid_sequence_across_chunks_stays_valid() {
    let text = "aé€😀".as_bytes();
    for split in 0..=text.len() {
        let mut sniffer = ContentSniffer::new();
        sniffer.feed(&text[..split]);
        sniffer.feed(&text[split..]);
        assert!(sniffer.finish().utf8_valid, "split at {split}");
    }
    let mut truncated = ContentSniffer::new();
    truncated.feed(&"é".as_bytes()[..1]);
    assert!(!truncated.finish().utf8_valid);
}

#[test]
fn lossy_and_malformed_are_reported() {
    let decoded = decode_bytes(
        b"caf\xe9",
        SideKind::Worktree,
        &TextAttributes::default(),
        Some(TextEncoding::UTF_8),
    );
    assert!(decoded.format.malformed);
    assert!(!decoded.format.is_writable());

    let decoded = decode_bytes(
        b"caf\xe9",
        SideKind::Worktree,
        &TextAttributes::default(),
        Some(TextEncoding::WINDOWS_1252),
    );
    assert!(decoded.format.is_writable());
    assert_eq!(decoded.text, "café");
}

#[test]
fn an_encoding_attribute_does_not_turn_binary_content_into_text() {
    let utf8_everywhere = TextAttributes {
        encoding: Some(EncodingAttr::from_label("utf-8")),
        ..TextAttributes::default()
    };
    let png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR\x00\x00\x01\x00";
    assert!(resolve(png, SideKind::Worktree, &utf8_everywhere).binary);
    assert!(resolve(png, SideKind::GitInternal, &utf8_everywhere).binary);

    // UTF-16 text is full of NULs, and a UTF-16 attribute still reads it.
    let utf16: Vec<u8> = "hi\n".encode_utf16().flat_map(u16::to_le_bytes).collect();
    let utf16_attribute = TextAttributes {
        encoding: Some(EncodingAttr::from_label("utf-16le")),
        ..TextAttributes::default()
    };
    let format = resolve(&utf16, SideKind::Worktree, &utf16_attribute);
    assert!(!format.binary);
    assert_eq!(format.source, FormatSource::EncodingAttribute);
}

#[test]
fn gb18030_checks_round_trips_for_whole_files_and_streams() {
    let format = TextFormat {
        encoding: TextEncoding::from_label("gb18030").unwrap(),
        bom: false,
    };
    // Both forms decode as the euro sign, but only A2 E3 is emitted by the
    // encoder. Include a sequence crossing the streaming read boundary.
    for (euro, lossy) in [(b"\x80".as_slice(), true), (b"\xa2\xe3".as_slice(), false)] {
        for padding in [0, 65_535, 65_536] {
            let mut bytes = vec![b'a'; padding];
            bytes.extend_from_slice(euro);
            bytes.push(b'\n');
            let decoded = decode_bytes(
                &bytes,
                SideKind::Worktree,
                &TextAttributes::default(),
                Some(format.encoding),
            );
            assert!(decoded.text.ends_with("€\n"));
            assert!(!decoded.format.malformed);
            assert_eq!(decoded.format.lossy, lossy);
            assert_eq!(decoded.format.is_writable(), !lossy);

            let mut utf8 = Vec::new();
            let stats = transcode_to_utf8(
                bytes.as_slice(),
                &mut utf8,
                format,
                &crate::services::CancellationToken::new(),
            )
            .unwrap();
            assert_eq!(utf8, decoded.text.as_bytes());
            assert!(!stats.malformed);
            assert_eq!(stats.lossy, lossy);
        }
    }
}

#[test]
fn a_streamed_transcode_reports_text_that_would_not_write_back_the_same() {
    let sjis = TextFormat {
        encoding: TextEncoding::from_label("shift_jis").unwrap(),
        bom: false,
    };
    let transcode = |bytes: &[u8]| {
        transcode_to_utf8(
            bytes,
            std::io::sink(),
            sjis,
            &crate::services::CancellationToken::new(),
        )
        .unwrap()
    };
    // 0xED40 is an NEC-selected duplicate of an IBM extension: it decodes
    // cleanly and encodes back as 0xFA5C.
    let duplicate = b"a\xed\x40b\n";
    assert!(!round_trips(duplicate, &decode(duplicate, sjis).text, sjis));
    let stats = transcode(duplicate);
    assert!(stats.lossy);
    assert!(!stats.malformed);

    // A character split by the 64 KiB read, then a duplicate past it.
    let mut long = b"a".to_vec();
    long.extend(encoded(&"日本".repeat(40_000), "shift_jis"));
    assert!(!transcode(&long).lossy);
    long.extend_from_slice(b"\xed\x40");
    assert!(transcode(&long).lossy);
}

/// Throughput of the paths a large file takes. Run with
/// `cargo test -p gitcomet-core --release --lib measure_text_format_throughput -- --ignored --nocapture`.
#[test]
#[ignore]
fn measure_text_format_throughput() {
    use std::time::Instant;
    const MB: usize = 1024 * 1024;
    let line = "Grüße aus Köln, schöne Straßen — ein ganz normaler Satz.\n";
    let utf8 = line.repeat(100 * MB / line.len());
    let cp1252 = encode(
        &utf8.replace('—', "-"),
        TextFormat {
            encoding: TextEncoding::WINDOWS_1252,
            bom: false,
        },
    )
    .unwrap()
    .into_owned();
    let sjis_text =
        "日本語のテキストです。文字コードの判定と変換を測ります。\n".repeat(100 * MB / 90);
    let sjis_format = TextFormat {
        encoding: TextEncoding::from_label("shift_jis").unwrap(),
        bom: false,
    };
    let sjis = encode(&sjis_text, sjis_format).unwrap().into_owned();
    let utf16_format = TextFormat {
        encoding: TextEncoding::UTF_16LE,
        bom: true,
    };
    let utf16 = encode(&utf8, utf16_format).unwrap().into_owned();

    let report = |label: &str, bytes: usize, run: &mut dyn FnMut()| {
        let start = Instant::now();
        run();
        let elapsed = start.elapsed();
        let rate = bytes as f64 / MB as f64 / elapsed.as_secs_f64();
        println!(
            "{label:<40} {:>8.1} ms  {:>8.0} MiB/s",
            elapsed.as_secs_f64() * 1e3,
            rate
        );
    };
    let sniff = |bytes: &[u8]| {
        let mut sniffer = ContentSniffer::new();
        for chunk in bytes.chunks(64 * 1024) {
            sniffer.feed(chunk);
        }
        sniffer.finish()
    };
    let transcode = |bytes: &[u8], format: TextFormat| {
        let mut out = Vec::with_capacity(bytes.len() * 2);
        transcode_to_utf8(
            bytes,
            &mut out,
            format,
            &crate::services::CancellationToken::new(),
        )
        .unwrap();
        out.len()
    };

    report("sniff UTF-8 (fast path)", utf8.len(), &mut || {
        let sniff = sniff(utf8.as_bytes());
        assert!(sniff.utf8_valid);
    });
    report("sniff + resolve windows-1252", cp1252.len(), &mut || {
        let format = sniff(&cp1252).resolve(SideKind::Worktree, &TextAttributes::default(), None);
        assert_eq!(format.format.encoding, TextEncoding::WINDOWS_1252);
    });
    report("sniff + resolve Shift_JIS", sjis.len(), &mut || {
        let format = sniff(&sjis).resolve(SideKind::Worktree, &TextAttributes::default(), None);
        assert_eq!(format.format.encoding.name(), "Shift_JIS");
    });
    report("transcode windows-1252 -> UTF-8", cp1252.len(), &mut || {
        transcode(
            &cp1252,
            TextFormat {
                encoding: TextEncoding::WINDOWS_1252,
                bom: false,
            },
        );
    });
    report("transcode Shift_JIS -> UTF-8", sjis.len(), &mut || {
        transcode(&sjis, sjis_format);
    });
    report("transcode UTF-16LE -> UTF-8", utf16.len(), &mut || {
        transcode(&utf16, utf16_format);
    });
    report("encode UTF-8 -> windows-1252", cp1252.len(), &mut || {
        let text = decode(
            &cp1252,
            TextFormat {
                encoding: TextEncoding::WINDOWS_1252,
                bom: false,
            },
        )
        .text
        .into_owned();
        encode(
            &text,
            TextFormat {
                encoding: TextEncoding::WINDOWS_1252,
                bom: false,
            },
        )
        .unwrap();
    });
    report("encode UTF-8 -> Shift_JIS", sjis.len(), &mut || {
        encode(&sjis_text, sjis_format).unwrap();
    });
}

#[test]
fn streaming_binary_rejection_respects_boms_overrides_and_chunk_boundaries() {
    let attributes = TextAttributes::default();
    let mut sniffer = ContentSniffer::new();
    sniffer.feed(&b"\0\xff\n".repeat(4096));
    assert!(sniffer.is_binary(SideKind::Worktree, &attributes, None));
    assert!(!sniffer.is_binary(
        SideKind::Worktree,
        &attributes,
        Some(TextEncoding::WINDOWS_1252)
    ));
    let mut text = vec![b'a'; 8192];
    text[0] = 0;
    text.push(0xc3);
    let mut sniffer = ContentSniffer::new();
    sniffer.feed(&text);
    assert!(!sniffer.is_binary(SideKind::Worktree, &attributes, None));
    sniffer.feed(b"\xa4");
    assert!(sniffer.finish().utf8_valid);
    for encoding in [TextEncoding::UTF_16LE, TextEncoding::UTF_16BE] {
        let bytes = encode(
            &"hello\n".repeat(2048),
            TextFormat {
                encoding,
                bom: true,
            },
        )
        .unwrap()
        .into_owned();
        let mut sniffer = ContentSniffer::new();
        sniffer.feed(&bytes);
        assert!(!sniffer.is_binary(SideKind::Worktree, &attributes, None));
    }
}
