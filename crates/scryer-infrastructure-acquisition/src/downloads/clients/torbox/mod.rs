//! TorBox, a hosted torrent and usenet downloader reached over its HTTP API.
//!
//! [`api`] speaks the API and returns typed outcomes; [`budget`] keeps the
//! hourly create allowances TorBox enforces beyond its per-minute limit.

pub mod api;
pub mod budget;

#[cfg(test)]
mod api_tests;
