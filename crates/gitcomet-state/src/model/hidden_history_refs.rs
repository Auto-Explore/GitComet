use gitcomet_core::branch_name::is_valid_ref_name;
use gitcomet_core::identity::HistoryRefPreset;
use gitcomet_core::services::HistoryRefFilter;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

/// Namespaces with their own visibility controls: branches are what the log
/// shows, remotes hide per remote, and tags never seed the walk.
const OWNED_NAMESPACES: [&str; 3] = ["refs/heads", "refs/remotes", "refs/tags"];

/// A user-written ref namespace to hide, normalized without a trailing `/`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HiddenRefPrefix(Arc<str>);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HiddenRefPrefixError {
    Empty,
    OutsideRefs,
    AllRefs,
    OwnedNamespace,
    InvalidName,
}

impl fmt::Display for HiddenRefPrefixError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Empty => "Enter a ref namespace, such as refs/review.",
            Self::OutsideRefs => "Ref namespaces start with refs/.",
            Self::AllRefs => "Hiding all of refs/ would hide every branch.",
            Self::OwnedNamespace => {
                "Branches, remotes and tags have their own visibility controls."
            }
            Self::InvalidName => "Git does not accept this as a ref name.",
        })
    }
}

impl HiddenRefPrefix {
    pub fn parse(input: &str) -> Result<Self, HiddenRefPrefixError> {
        let prefix = input.trim().trim_end_matches('/');
        if prefix.is_empty() {
            return Err(HiddenRefPrefixError::Empty);
        }
        if prefix == "refs" {
            return Err(HiddenRefPrefixError::AllRefs);
        }
        if !prefix.starts_with("refs/") {
            return Err(HiddenRefPrefixError::OutsideRefs);
        }
        if !is_valid_ref_name(prefix) {
            return Err(HiddenRefPrefixError::InvalidName);
        }
        if OWNED_NAMESPACES.iter().any(|namespace| {
            prefix
                .strip_prefix(namespace)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
        }) {
            return Err(HiddenRefPrefixError::OwnedNamespace);
        }
        Ok(Self(prefix.into()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// How a preset's switch reads: forced by the product, or the user's choice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PresetVisibility {
    Fixed,
    Hidden,
    Shown,
}

/// Which ref namespaces the all-branches Git log hides.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HiddenRefSettings {
    pub enabled: bool,
    /// Presets the user switched away from the product's default, by preset id.
    pub preset_overrides: BTreeMap<String, bool>,
    /// Unique, in the order the user added them.
    custom_prefixes: Vec<HiddenRefPrefix>,
}

impl Default for HiddenRefSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            preset_overrides: BTreeMap::new(),
            custom_prefixes: Vec::new(),
        }
    }
}

impl HiddenRefSettings {
    pub fn preset_hidden(&self, preset: &HistoryRefPreset) -> bool {
        self.preset_overrides
            .get(preset.id)
            .copied()
            .unwrap_or(preset.hidden_by_default)
    }

    /// `fixed` is the product's always-hidden floor; it wins over the switch.
    pub fn preset_visibility(
        &self,
        preset: &HistoryRefPreset,
        fixed: &HistoryRefFilter,
    ) -> PresetVisibility {
        if !preset.prefixes.is_empty() && preset.prefixes.iter().all(|p| fixed.excludes(p)) {
            return PresetVisibility::Fixed;
        }
        if self.enabled && self.preset_hidden(preset) {
            return PresetVisibility::Hidden;
        }
        PresetVisibility::Shown
    }

    /// Namespaces hidden now, the product's fixed presets included.
    pub fn hidden_count(&self, presets: &[HistoryRefPreset], fixed: &HistoryRefFilter) -> usize {
        let presets = presets
            .iter()
            .filter(|preset| self.preset_visibility(preset, fixed) != PresetVisibility::Shown)
            .count();
        let custom = if self.enabled {
            self.custom_prefixes.len()
        } else {
            0
        };
        presets + custom
    }

    /// Stores only a departure from the default, so a product that changes a
    /// preset's default reaches every user who never touched it.
    pub fn set_preset_hidden(&mut self, preset: &HistoryRefPreset, hidden: bool) {
        if hidden == preset.hidden_by_default {
            self.preset_overrides.remove(preset.id);
        } else {
            self.preset_overrides.insert(preset.id.to_owned(), hidden);
        }
    }

    pub fn custom_prefixes(&self) -> &[HiddenRefPrefix] {
        &self.custom_prefixes
    }

    /// Whether `prefix` was new.
    pub fn add_custom_prefix(&mut self, prefix: HiddenRefPrefix) -> bool {
        if self.custom_prefixes.contains(&prefix) {
            return false;
        }
        self.custom_prefixes.push(prefix);
        true
    }

    pub fn remove_custom_prefix(&mut self, prefix: &HiddenRefPrefix) {
        self.custom_prefixes.retain(|kept| kept != prefix);
    }

    pub fn filter(&self, presets: &[HistoryRefPreset]) -> HistoryRefFilter {
        if !self.enabled {
            return HistoryRefFilter::default();
        }
        HistoryRefFilter::excluding(
            presets
                .iter()
                .filter(|preset| self.preset_hidden(preset))
                .flat_map(|preset| preset.prefixes.iter().copied())
                .chain(self.custom_prefixes.iter().map(HiddenRefPrefix::as_str)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REVIEW: HistoryRefPreset =
        HistoryRefPreset::hidden("review", "Review", &["refs/review"], "Review refs.");
    const NOTES: HistoryRefPreset =
        HistoryRefPreset::hidden("notes", "Notes", &["refs/notes"], "Notes.").shown_by_default();

    fn prefixes(filter: &HistoryRefFilter) -> Vec<&str> {
        filter.prefixes().iter().map(|prefix| &**prefix).collect()
    }

    #[test]
    fn custom_prefixes_parse_to_normalized_namespaces() {
        for (input, parsed) in [
            ("refs/t3", "refs/t3"),
            ("  refs/t3/ ", "refs/t3"),
            ("refs/stash", "refs/stash"),
            ("refs/heads-archive", "refs/heads-archive"),
        ] {
            assert_eq!(HiddenRefPrefix::parse(input).unwrap().as_str(), parsed);
        }
        for (input, error) in [
            ("", HiddenRefPrefixError::Empty),
            (" / ", HiddenRefPrefixError::Empty),
            ("refs", HiddenRefPrefixError::AllRefs),
            ("refs/", HiddenRefPrefixError::AllRefs),
            ("t3/checkpoints", HiddenRefPrefixError::OutsideRefs),
            ("refs/heads", HiddenRefPrefixError::OwnedNamespace),
            ("refs/heads/wip", HiddenRefPrefixError::OwnedNamespace),
            ("refs/remotes/origin", HiddenRefPrefixError::OwnedNamespace),
            ("refs/tags", HiddenRefPrefixError::OwnedNamespace),
            ("refs/t3/*", HiddenRefPrefixError::InvalidName),
            ("refs/my review", HiddenRefPrefixError::InvalidName),
            ("refs//t3", HiddenRefPrefixError::InvalidName),
            ("refs/.hidden", HiddenRefPrefixError::InvalidName),
            ("refs/a..b", HiddenRefPrefixError::InvalidName),
            ("refs/t3.lock", HiddenRefPrefixError::InvalidName),
        ] {
            assert_eq!(HiddenRefPrefix::parse(input), Err(error), "{input:?}");
        }
    }

    #[test]
    fn hidden_ref_prefix_errors_name_the_fix() {
        assert_eq!(
            HiddenRefPrefixError::OutsideRefs.to_string(),
            "Ref namespaces start with refs/."
        );
        assert_eq!(
            HiddenRefPrefixError::InvalidName.to_string(),
            "Git does not accept this as a ref name."
        );
    }

    #[test]
    fn a_fixed_floor_wins_over_the_switch_and_counts_as_hidden() {
        let presets = [REVIEW, NOTES];
        let fixed = HistoryRefFilter::excluding(["refs/notes/"]);
        let mut settings = HiddenRefSettings::default();
        assert_eq!(
            settings.preset_visibility(&REVIEW, &fixed),
            PresetVisibility::Hidden
        );
        assert_eq!(
            settings.preset_visibility(&NOTES, &fixed),
            PresetVisibility::Fixed,
            "shown by default, but the product always hides it"
        );
        settings.add_custom_prefix(HiddenRefPrefix::parse("refs/t3").unwrap());
        assert_eq!(settings.hidden_count(&presets, &fixed), 3);

        settings.enabled = false;
        assert_eq!(
            settings.preset_visibility(&REVIEW, &fixed),
            PresetVisibility::Shown
        );
        assert_eq!(
            settings.hidden_count(&presets, &fixed),
            1,
            "switched off, only the fixed floor still hides"
        );
        assert_eq!(
            settings.hidden_count(&presets, &HistoryRefFilter::default()),
            0
        );
    }

    #[test]
    fn the_filter_follows_preset_defaults_overrides_and_custom_prefixes() {
        let presets = [REVIEW, NOTES];
        let mut settings = HiddenRefSettings::default();
        assert_eq!(prefixes(&settings.filter(&presets)), ["refs/review"]);

        settings.set_preset_hidden(&NOTES, true);
        settings.set_preset_hidden(&REVIEW, false);
        assert_eq!(prefixes(&settings.filter(&presets)), ["refs/notes"]);
        assert_eq!(settings.preset_overrides.len(), 2);

        settings.set_preset_hidden(&REVIEW, true);
        assert!(
            !settings.preset_overrides.contains_key("review"),
            "back at the default stores nothing"
        );

        assert!(settings.add_custom_prefix(HiddenRefPrefix::parse("refs/t3").unwrap()));
        assert!(!settings.add_custom_prefix(HiddenRefPrefix::parse("refs/t3/").unwrap()));
        assert_eq!(
            prefixes(&settings.filter(&presets)),
            ["refs/notes", "refs/review", "refs/t3"]
        );

        settings.enabled = false;
        assert!(settings.filter(&presets).is_empty());
        settings.enabled = true;
        settings.remove_custom_prefix(&HiddenRefPrefix::parse("refs/t3").unwrap());
        assert_eq!(
            prefixes(&settings.filter(&presets)),
            ["refs/notes", "refs/review"]
        );
    }
}
