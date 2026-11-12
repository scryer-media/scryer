//! TorBox, a hosted torrent and usenet downloader reached over its HTTP API.
//!
//! [`api`] speaks the API and returns typed outcomes; [`budget`] keeps the
//! hourly create allowances TorBox enforces beyond its per-minute limit.
//! [`client`] is the `torbox` download client built on both: TorBox fetches
//! the item, and Scryer's HTTP engine then downloads its files.

pub mod api;
pub mod budget;
pub(crate) mod client;
mod fetch;
mod records;
mod state;

pub use client::TorboxDownloadClient;

#[cfg(test)]
mod api_tests;
#[cfg(test)]
mod client_tests;
