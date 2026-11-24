//! Versioned media facts shared by parsers, application DTOs, and rule inputs,
//! plus Scryer's own policy over them: which suffixes are library items and how
//! audio is labelled and ranked for release decisions.
//! This crate contains data only and never opens media or depends on a parser.

pub use scryer_media_model::*;
mod audio;
pub use audio::{
    audio_codec_rank_for_release_label, normalize_audio_codec_for_release,
    primary_audio_stream_index,
};
pub mod formats;

#[cfg(test)]
mod tests {
    use super::*;

    /// The first revision whose primary audio summary follows the authored
    /// default track rather than the highest channel count or codec rank.
    const AUTHORED_PRIMARY_AUDIO_REVISION: u32 = 2;

    #[test]
    fn analyses_from_before_authored_primary_audio_are_refreshed() {
        const { assert!(ANALYSIS_REVISION >= AUTHORED_PRIMARY_AUDIO_REVISION) };
        assert_eq!(
            primary_audio_stream_index([(false, false, None), (true, false, None)]),
            Some(1)
        );
    }
}
