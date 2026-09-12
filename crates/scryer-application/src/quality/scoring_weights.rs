use serde::{Deserialize, Serialize};

/// Scoring persona — a named preset that sets all default scoring weights.
/// Users pick one persona and optionally flip a few overrides.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum ScoringPersona {
    #[default]
    Balanced,
    Audiophile,
    Efficient,
    Compatible,
}

/// Five toggles that patch specific weights regardless of persona.
/// `None` means "use the persona's default". `Some(true/false)` overrides it.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ScoringOverrides {
    /// Allow x265/HEVC at non-4K resolutions without penalty.
    #[serde(default)]
    pub allow_x265_non4k: Option<bool>,

    /// Block Dolby Vision releases that lack an HDR10 fallback layer.
    #[serde(default)]
    pub block_dv_without_fallback: Option<bool>,

    /// Invert the size curve to reward smaller, more efficient encodes.
    #[serde(default)]
    pub prefer_compact_encodes: Option<bool>,

    /// Extra bonus for lossless audio codecs (TrueHD, FLAC, DTS-HD MA, PCM).
    #[serde(default)]
    pub prefer_lossless_audio: Option<bool>,

    /// Block releases with AI upscale indicators.
    #[serde(default)]
    pub block_upscaled: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn omitted_overrides_preserve_unset_values() {
        let overrides: ScoringOverrides = serde_json::from_str("{}").unwrap();
        assert_eq!(overrides, ScoringOverrides::default());
        let value = serde_json::to_value(overrides).unwrap();
        assert!(
            value
                .as_object()
                .unwrap()
                .values()
                .all(|value| value.is_null())
        );
    }

    #[test]
    fn persona_and_explicit_false_round_trip() {
        assert_eq!(ScoringPersona::default(), ScoringPersona::Balanced);
        for persona in [
            ScoringPersona::Balanced,
            ScoringPersona::Audiophile,
            ScoringPersona::Efficient,
            ScoringPersona::Compatible,
        ] {
            assert_eq!(
                serde_json::from_value::<ScoringPersona>(serde_json::to_value(&persona).unwrap())
                    .unwrap(),
                persona
            );
        }
        let overrides: ScoringOverrides =
            serde_json::from_str(r#"{"allow_x265_non4k":false,"prefer_lossless_audio":true}"#)
                .unwrap();
        assert_eq!(overrides.allow_x265_non4k, Some(false));
        assert_eq!(overrides.prefer_lossless_audio, Some(true));
        assert_eq!(overrides.block_upscaled, None);
        assert_eq!(
            serde_json::from_value::<ScoringOverrides>(serde_json::to_value(&overrides).unwrap())
                .unwrap(),
            overrides
        );
    }
}
