use super::human_readable_bytes;

#[test]
fn formats_decimal_units_with_one_fraction_digit() {
    assert_eq!(human_readable_bytes(0), "0 B");
    assert_eq!(human_readable_bytes(999), "999 B");
    assert_eq!(human_readable_bytes(1_000), "1 KB");
    assert_eq!(human_readable_bytes(1_536), "1.5 KB");
    assert_eq!(human_readable_bytes(999_949), "999.9 KB");
    assert_eq!(human_readable_bytes(999_950), "1 MB");
    assert_eq!(human_readable_bytes(999_950_000), "1 GB");
    assert_eq!(human_readable_bytes(12_345_678), "12.3 MB");
    assert_eq!(human_readable_bytes(1_200_000_000), "1.2 GB");
    assert_eq!(human_readable_bytes(5_000_000_000_000_000), "5000 TB");
}
