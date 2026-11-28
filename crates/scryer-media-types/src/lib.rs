//! Versioned media facts shared by parsers, application DTOs, and rule inputs,
//! plus Scryer's own policy over them: which suffixes are library items and how
//! audio is labelled and ranked for release decisions.
//! This crate contains data only and never opens media or depends on a parser.

pub use mediaprobe::*;
mod audio;
pub use audio::{audio_codec_rank_for_release_label, normalize_audio_codec_for_release};
pub mod formats;
