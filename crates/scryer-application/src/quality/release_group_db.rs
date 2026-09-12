use std::sync::LazyLock;

use std::collections::HashSet;

/// A release-group spelling accepted solely to remove a leading group tag from
/// a title-identity anchor. This is lexical parsing data, never a reputation
/// score or source/facet policy.
#[derive(Debug, Clone, Copy)]
pub struct KnownReleaseGroupRule {
    pub matcher: &'static str,
    pub match_kind: GroupMatchKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupMatchKind {
    Exact,
    Prefix,
}

include!("trash_guides_release_groups.generated.rs");

struct KnownReleaseGroupIndex {
    exact: HashSet<String>,
    prefixes: Vec<&'static KnownReleaseGroupRule>,
}

static KNOWN_RELEASE_GROUP_INDEX: LazyLock<KnownReleaseGroupIndex> = LazyLock::new(|| {
    let mut exact = HashSet::new();
    let mut prefixes = Vec::new();
    for rule in KNOWN_RELEASE_GROUP_RULES {
        match rule.match_kind {
            GroupMatchKind::Exact => {
                exact.insert(rule.matcher.to_ascii_uppercase());
            }
            GroupMatchKind::Prefix => prefixes.push(rule),
        }
    }
    KnownReleaseGroupIndex { exact, prefixes }
});

fn known_release_group_matches(rule: &KnownReleaseGroupRule, candidate: &str) -> bool {
    match rule.match_kind {
        GroupMatchKind::Exact => rule.matcher.eq_ignore_ascii_case(candidate),
        GroupMatchKind::Prefix => candidate
            .get(..rule.matcher.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(rule.matcher)),
    }
}

/// Returns whether a candidate is an ordinary lexical release-group prefix
/// accepted by title anchoring. It does not expose scoring reputation.
pub(crate) fn is_known_release_group(candidate: &str) -> bool {
    let candidate_upper = candidate.to_ascii_uppercase();
    KNOWN_RELEASE_GROUP_INDEX.exact.contains(&candidate_upper)
        || KNOWN_RELEASE_GROUP_INDEX
            .prefixes
            .iter()
            .any(|rule| known_release_group_matches(rule, candidate))
}

#[cfg(test)]
mod lexical_tests {
    use super::is_known_release_group;

    #[test]
    fn lexical_prefix_index_accepts_known_groups_without_reputation_context() {
        assert!(is_known_release_group("Erai-raws"));
        assert!(is_known_release_group("erai-raws"));
        assert!(!is_known_release_group("Totally Unknown Grp"));
    }
}
