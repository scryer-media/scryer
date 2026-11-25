//! Bounds shared by the Plex, Jellyfin and Emby catalog scans.
//!
//! Each scan pages through the server's library until the server says the
//! listing is done. These bounds make a server that never says so, or that
//! answers without end, fail the scan instead of holding it forever.

use std::collections::HashSet;
use std::fmt;

use scryer_application::{AppError, AppResult};
use scryer_outbound_http::{BoundedBodyError, read_body_bounded};
use serde_json::Value;

/// What one catalog scan reads within.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CatalogScanLimits {
    /// The most pages one listing may take before the scan gives up on it.
    pub(crate) page_ceiling: usize,
    /// The most bytes one answer may decode to.
    pub(crate) response_max_bytes: usize,
}

impl CatalogScanLimits {
    /// At 500 rows a page, 4,000 pages is two million items, far past any
    /// real library, so only a server that is not paging as asked reaches
    /// it. A 500-row page is kilobytes to a few megabytes and the unpaged
    /// answers (Plex `library/sections`, one item's metadata) are smaller, so
    /// 32 MiB only stops an answer that never ends.
    pub(crate) const STANDARD: Self = Self {
        page_ceiling: 4_000,
        response_max_bytes: 32 * 1024 * 1024,
    };
}

/// Why a catalog answer could not be read.
#[derive(Debug)]
pub(crate) enum CatalogBodyError {
    Body(BoundedBodyError),
    Json(serde_json::Error),
}

impl fmt::Display for CatalogBodyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Body(error) => error.fmt(formatter),
            Self::Json(error) => error.fmt(formatter),
        }
    }
}

/// Reads one catalog answer as JSON within the scan's byte limit.
pub(crate) async fn read_catalog_json(
    response: reqwest::Response,
    limits: CatalogScanLimits,
) -> Result<Value, CatalogBodyError> {
    let body = read_body_bounded(response, limits.response_max_bytes)
        .await
        .map_err(CatalogBodyError::Body)?;
    serde_json::from_slice(&body).map_err(CatalogBodyError::Json)
}

/// Follows one paged listing, so a server that is not paging as asked fails
/// the scan instead of looping.
///
/// A page that brings no item the scan has not already seen means the server
/// is answering from somewhere other than where it was asked. A page that
/// repeats some items but brings new ones is still progress: a library that
/// changes mid-scan shifts its pages that way.
pub(crate) struct CatalogPager {
    provider: &'static str,
    page_ceiling: usize,
    pages: usize,
    seen: HashSet<String>,
}

impl CatalogPager {
    pub(crate) fn new(provider: &'static str, limits: CatalogScanLimits) -> Self {
        Self {
            provider,
            page_ceiling: limits.page_ceiling,
            pages: 0,
            seen: HashSet::new(),
        }
    }

    /// Records one page by its items' ids. An item without an id counts as
    /// progress, since there is no telling it was seen before.
    pub(crate) fn record_page<I>(&mut self, ids: I) -> AppResult<()>
    where
        I: IntoIterator<Item = Option<String>>,
    {
        self.pages += 1;
        let mut any = false;
        let mut progressed = false;
        for id in ids {
            any = true;
            progressed |= id.is_none_or(|id| self.seen.insert(id));
        }
        if any && !progressed {
            return Err(AppError::Repository(format!(
                "{} catalog scan failed: the server repeated a page instead of paging as asked",
                self.provider
            )));
        }
        Ok(())
    }

    /// Checked before another page is requested.
    pub(crate) fn ensure_another_page_allowed(&self) -> AppResult<()> {
        if self.pages >= self.page_ceiling {
            return Err(AppError::Repository(format!(
                "{} catalog scan failed: the server never signalled the end of the listing",
                self.provider
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_of_only_repeated_ids_fails() {
        let mut pager = CatalogPager::new("Test", CatalogScanLimits::STANDARD);
        pager
            .record_page(["a", "b"].map(|id| Some(id.to_string())))
            .expect("first page");
        pager
            .record_page(["b", "c"].map(|id| Some(id.to_string())))
            .expect("a shifted page still progresses");
        assert!(
            pager
                .record_page(["a", "c"].map(|id| Some(id.to_string())))
                .is_err()
        );
    }

    #[test]
    fn items_without_ids_and_empty_pages_are_not_repeats() {
        let mut pager = CatalogPager::new("Test", CatalogScanLimits::STANDARD);
        pager.record_page([None, None]).expect("no ids");
        pager.record_page([None]).expect("still no ids");
        pager
            .record_page(std::iter::empty())
            .expect("an empty page ends the listing, it is not a repeat");
    }

    #[test]
    fn the_ceiling_stops_a_listing_that_never_ends() {
        let mut pager = CatalogPager::new(
            "Test",
            CatalogScanLimits {
                page_ceiling: 2,
                ..CatalogScanLimits::STANDARD
            },
        );
        pager.record_page([Some("a".to_string())]).expect("page 1");
        pager
            .ensure_another_page_allowed()
            .expect("under the ceiling");
        pager.record_page([Some("b".to_string())]).expect("page 2");
        assert!(pager.ensure_another_page_allowed().is_err());
    }

    #[test]
    fn the_standard_ceiling_is_far_past_any_real_library() {
        const { assert!(CatalogScanLimits::STANDARD.page_ceiling * 500 >= 1_000_000) };
    }
}
