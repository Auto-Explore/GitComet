use gitcomet_core::domain::{SignatureFormat, SignatureStatus};

#[test]
fn an_untrusted_key_is_not_a_verified_identity() {
    assert!(SignatureStatus::Good.is_verified());
    assert!(!SignatureStatus::GoodUncertified.is_verified());
}

#[test]
fn signature_armor_matches_gits_raw_prefix_rules() {
    for (armor, format) in [
        ("-----BEGIN PGP SIGNATURE-----", SignatureFormat::OpenPgp),
        ("-----BEGIN PGP MESSAGE-----", SignatureFormat::OpenPgp),
        ("-----BEGIN SSH SIGNATURE-----", SignatureFormat::Ssh),
        ("-----BEGIN SIGNED MESSAGE-----", SignatureFormat::X509),
    ] {
        assert_eq!(SignatureFormat::from_armor(armor.as_bytes()), Some(format));
        assert_eq!(
            SignatureFormat::from_armor(format!("{armor}suffix\n").as_bytes()),
            Some(format)
        );
        assert_eq!(
            SignatureFormat::from_armor(format!(" {armor}\n").as_bytes()),
            None
        );
        assert_eq!(
            SignatureFormat::from_armor(format!("\n{armor}\n").as_bytes()),
            None
        );
    }
}
