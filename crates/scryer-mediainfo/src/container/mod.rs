//! One parser per container format. Each reads through a bounded
//! [`MediaSource`](crate::source::MediaSource) and returns a
//! [`RawContainer`](crate::raw::RawContainer).

pub(crate) mod asf;
pub(crate) mod audio_only;
pub(crate) mod avi;
pub(crate) mod dv;
pub(crate) mod elementary;
pub(crate) mod flv;
pub(crate) mod mkv;
pub(crate) mod mp4;
pub(crate) mod ogg;
pub(crate) mod ps;
pub(crate) mod realmedia;
pub(crate) mod ts;
