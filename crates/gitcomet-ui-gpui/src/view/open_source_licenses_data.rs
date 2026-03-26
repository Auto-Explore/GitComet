use std::sync::OnceLock;

#[derive(Clone, Copy, Debug)]
pub(crate) struct OpenSourceLicenseRow {
    pub(crate) crate_name: &'static str,
    pub(crate) version: &'static str,
    pub(crate) license: &'static str,
}

pub(crate) fn open_source_license_rows() -> &'static [OpenSourceLicenseRow] {
    static ROWS: OnceLock<Vec<OpenSourceLicenseRow>> = OnceLock::new();
    ROWS.get_or_init(|| {
        include_str!("../../assets/open_source_licenses.tsv")
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    return None;
                }

                let mut fields = line.splitn(3, '\t');
                Some(OpenSourceLicenseRow {
                    crate_name: fields.next()?,
                    version: fields.next()?,
                    license: fields.next()?,
                })
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::open_source_license_rows;

    #[test]
    fn embedded_open_source_license_rows_are_available() {
        assert!(
            !open_source_license_rows().is_empty(),
            "expected bundled open source license data"
        );
    }
}
