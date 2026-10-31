//! Authorization restrictions applied before assigning aggregate positions.

use crate::{DownloadSourceKind, IndexerSearchResult};
use scryer_domain::IndexerProtocolCapability;

#[derive(Clone, Default)]
pub(crate) struct RawSearchResultScope {
    pub protocol: Option<IndexerProtocolCapability>,
    /// An exclusively declared source protocol can supply the kind when an
    /// older adapter leaves it absent. Mixed/unknown sources cannot do so.
    pub allow_untyped_protocol: bool,
    pub categories: Option<Vec<String>>,
    pub allow_uncategorized: bool,
}

/// Nab's standard thousand-wide parent families include their subcategories.
/// Custom IDs and string-valued source categories remain exact identities;
/// no new cross-source category mapping is introduced here.
pub(crate) fn category_contains(allowed: &str, candidate: &str) -> bool {
    match (allowed.parse::<u32>(), candidate.parse::<u32>()) {
        (Ok(allowed), Ok(candidate)) => {
            allowed == candidate
                || ((1000..10000).contains(&allowed)
                    && allowed % 1000 == 0
                    && candidate / 1000 == allowed / 1000)
        }
        _ => allowed == candidate,
    }
}

impl RawSearchResultScope {
    pub(super) fn heap_bytes(&self) -> usize {
        self.categories.as_ref().map_or(0, |values| {
            values.capacity() * std::mem::size_of::<String>()
                + values.iter().map(String::capacity).sum::<usize>()
        })
    }

    pub(super) fn allows(&self, result: &IndexerSearchResult) -> bool {
        let protocol_allowed = match (self.protocol, result.source_kind) {
            (None, _) => true,
            (Some(_), None) => self.allow_untyped_protocol,
            (Some(IndexerProtocolCapability::Usenet), Some(kind)) => {
                matches!(
                    kind,
                    DownloadSourceKind::NzbFile | DownloadSourceKind::NzbUrl
                )
            }
            (Some(IndexerProtocolCapability::Torrent), Some(kind)) => {
                matches!(
                    kind,
                    DownloadSourceKind::TorrentFile | DownloadSourceKind::MagnetUri
                )
            }
            _ => false,
        };
        protocol_allowed
            && self.categories.as_ref().is_none_or(|allowed| {
                (self.allow_uncategorized && result.response_attributes.categories.is_empty())
                    || result
                        .response_attributes
                        .categories
                        .iter()
                        .any(|category| {
                            allowed
                                .iter()
                                .any(|grant| category_contains(grant, category))
                        })
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_parents_include_children_but_custom_categories_never_become_ranges() {
        assert!(category_contains("4000", "4010"));
        assert!(category_contains("6000", "6070"));
        assert!(category_contains("04010", "4010"));
        assert!(category_contains("100000", "100000"));
        assert!(!category_contains("100000", "100001"));
        assert!(!category_contains("4000", "14010"));
        assert!(!category_contains("4010", "4000"));
        assert!(category_contains("software", "software"));
        assert!(!category_contains("software", "software-tools"));
    }
}
