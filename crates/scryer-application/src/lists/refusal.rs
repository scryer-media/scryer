//! Stable names for the conditions a lists request is refused on.
//!
//! Each one travels beside the English sentence as `extensions.reason` on a
//! `VALIDATION_ERROR`, so a client can say the same thing in its own language
//! instead of showing or matching the sentence. A name states the condition,
//! never the wording, and is API once released: reword a sentence freely, but
//! do not rename or reuse its reason.
//!
//! Some lists failures carry no reason. A sentence that only relays what a
//! provider answered (a failed preview fetch) has nothing stable to name. An
//! account authentication failure already carries its own bounded code
//! (`account_transport::AUTH_FAILURE_CODES`). The malformed-filter guards in
//! the GraphQL mappers reject shapes no client builds on purpose. And the
//! refusal while lists are switched off is raised before any lists request is
//! looked at.

use crate::AppError;

/// The list is already followed: for everyone, or by this member's account.
pub const ALREADY_FOLLOWED: &str = "LIST_ALREADY_FOLLOWED";
/// Neither a provider and list nor a link was given.
pub const SOURCE_REQUIRED: &str = "LIST_SOURCE_REQUIRED";
/// The link is not one any installed provider recognises.
pub const LINK_NOT_RECOGNIZED: &str = "LIST_LINK_NOT_RECOGNIZED";
/// No installed plugin serves the named provider.
pub const PROVIDER_NOT_INSTALLED: &str = "LIST_PROVIDER_NOT_INSTALLED";
/// The provider's plugin is installed but cannot be used right now.
pub const PROVIDER_UNAVAILABLE: &str = "LIST_PROVIDER_UNAVAILABLE";
/// The provider does not offer the named list.
pub const SOURCE_NOT_OFFERED: &str = "LIST_SOURCE_NOT_OFFERED";
/// The named chart is not in the chart catalog.
pub const CHART_UNAVAILABLE: &str = "LIST_CHART_UNAVAILABLE";
/// The list needs a member's own account and cannot be followed for everyone.
pub const MEMBER_ACCOUNT_ONLY: &str = "LIST_MEMBER_ACCOUNT_ONLY";
/// A parameter the list requires is missing or blank.
pub const PARAM_REQUIRED: &str = "LIST_PARAM_REQUIRED";
/// A parameter the list does not have was sent.
pub const PARAM_UNKNOWN: &str = "LIST_PARAM_UNKNOWN";
/// A parameter's value is not one the list accepts.
pub const PARAM_INVALID: &str = "LIST_PARAM_INVALID";
/// The chosen mode is not available for this kind of list.
pub const MODE_NOT_ALLOWED: &str = "LIST_MODE_NOT_ALLOWED";
/// A chosen title kind is one the list does not contain.
pub const KIND_NOT_IN_LIST: &str = "LIST_KIND_NOT_IN_LIST";
/// No title kind is chosen.
pub const KINDS_REQUIRED: &str = "LIST_KINDS_REQUIRED";
/// A route targets a title kind the list does not keep.
pub const ROUTE_KIND_NOT_KEPT: &str = "LIST_ROUTE_KIND_NOT_KEPT";
/// Two routes target the same title kind.
pub const ROUTE_KIND_DUPLICATED: &str = "LIST_ROUTE_KIND_DUPLICATED";
/// A kept title kind has no route.
pub const ROUTE_MISSING_FOR_KIND: &str = "LIST_ROUTE_MISSING_FOR_KIND";
/// A route names no library.
pub const ROUTE_LIBRARY_REQUIRED: &str = "LIST_ROUTE_LIBRARY_REQUIRED";
/// A route names a library that does not exist.
pub const ROUTE_LIBRARY_NOT_FOUND: &str = "LIST_ROUTE_LIBRARY_NOT_FOUND";
/// A route sends a title kind to a library of another kind.
pub const ROUTE_LIBRARY_KIND_MISMATCH: &str = "LIST_ROUTE_LIBRARY_KIND_MISMATCH";
/// The per-sync cap is below one.
pub const SYNC_CAP_INVALID: &str = "LIST_SYNC_CAP_INVALID";
/// The list is turned off, so it cannot be synced on request.
pub const DISABLED: &str = "LIST_DISABLED";
/// An exclusion names no external id.
pub const EXCLUSION_IDS_REQUIRED: &str = "LIST_EXCLUSION_IDS_REQUIRED";
/// An exclusion has no title to show.
pub const EXCLUSION_TITLE_REQUIRED: &str = "LIST_EXCLUSION_TITLE_REQUIRED";
/// A personal list was requested without a linked account.
pub const ACCOUNT_REQUIRED: &str = "LIST_ACCOUNT_REQUIRED";
/// The linked account's grant is no longer usable; it must be linked again.
pub const ACCOUNT_RECONNECT_REQUIRED: &str = "LIST_ACCOUNT_RECONNECT_REQUIRED";
/// The linked account belongs to a different provider than the list.
pub const ACCOUNT_PROVIDER_MISMATCH: &str = "LIST_ACCOUNT_PROVIDER_MISMATCH";
/// The provider signed the member in but returned no account identity.
pub const ACCOUNT_IDENTITY_MISSING: &str = "LIST_ACCOUNT_IDENTITY_MISSING";
/// The provider would not confirm whose account the grant belongs to.
pub const ACCOUNT_IDENTITY_UNVERIFIED: &str = "LIST_ACCOUNT_IDENTITY_UNVERIFIED";
/// The provider has no member accounts to link.
pub const ACCOUNT_LINKING_UNSUPPORTED: &str = "LIST_ACCOUNT_LINKING_UNSUPPORTED";
/// The address the link was started from is not a usable origin.
pub const ACCOUNT_LINK_ORIGIN_INVALID: &str = "LIST_ACCOUNT_LINK_ORIGIN_INVALID";
/// Too many account links are waiting to be finished.
pub const ACCOUNT_LINKS_TOO_MANY: &str = "LIST_ACCOUNT_LINKS_TOO_MANY";
/// The provider's answer to an account link is empty or oversized.
pub const ACCOUNT_LINK_RESULT_INVALID: &str = "LIST_ACCOUNT_LINK_RESULT_INVALID";
/// An account link's answer came from a different issuer than the provider.
pub const ACCOUNT_LINK_ISSUER_INVALID: &str = "LIST_ACCOUNT_LINK_ISSUER_INVALID";
/// A link that finishes by redirect was asked to poll.
pub const ACCOUNT_LINK_NOT_POLLED: &str = "LIST_ACCOUNT_LINK_NOT_POLLED";
/// This instance has no way to authenticate list accounts.
pub const ACCOUNT_AUTH_UNAVAILABLE: &str = "LIST_ACCOUNT_AUTH_UNAVAILABLE";
/// The provider does not take an instance-owned app.
pub const PROVIDER_APP_UNSUPPORTED: &str = "LIST_PROVIDER_APP_UNSUPPORTED";
/// A provider app's redirect URI is not a URL.
pub const PROVIDER_APP_REDIRECT_INVALID: &str = "LIST_PROVIDER_APP_REDIRECT_INVALID";
/// A provider app is missing its client ID, a required secret, or a usable
/// account callback URI.
pub const PROVIDER_APP_INCOMPLETE: &str = "LIST_PROVIDER_APP_INCOMPLETE";
/// The key is not a server-wide setting of the list provider.
pub const PROVIDER_SETTING_UNKNOWN: &str = "LIST_PROVIDER_SETTING_UNKNOWN";

/// Every reason a lists refusal can name. Clients key translations on these,
/// so adding, renaming, or removing one must be deliberate.
pub const ALL: &[&str] = &[
    ALREADY_FOLLOWED,
    SOURCE_REQUIRED,
    LINK_NOT_RECOGNIZED,
    PROVIDER_NOT_INSTALLED,
    PROVIDER_UNAVAILABLE,
    SOURCE_NOT_OFFERED,
    CHART_UNAVAILABLE,
    MEMBER_ACCOUNT_ONLY,
    PARAM_REQUIRED,
    PARAM_UNKNOWN,
    PARAM_INVALID,
    MODE_NOT_ALLOWED,
    KIND_NOT_IN_LIST,
    KINDS_REQUIRED,
    ROUTE_KIND_NOT_KEPT,
    ROUTE_KIND_DUPLICATED,
    ROUTE_MISSING_FOR_KIND,
    ROUTE_LIBRARY_REQUIRED,
    ROUTE_LIBRARY_NOT_FOUND,
    ROUTE_LIBRARY_KIND_MISMATCH,
    SYNC_CAP_INVALID,
    DISABLED,
    EXCLUSION_IDS_REQUIRED,
    EXCLUSION_TITLE_REQUIRED,
    ACCOUNT_REQUIRED,
    ACCOUNT_RECONNECT_REQUIRED,
    ACCOUNT_PROVIDER_MISMATCH,
    ACCOUNT_IDENTITY_MISSING,
    ACCOUNT_IDENTITY_UNVERIFIED,
    ACCOUNT_LINKING_UNSUPPORTED,
    ACCOUNT_LINK_ORIGIN_INVALID,
    ACCOUNT_LINKS_TOO_MANY,
    ACCOUNT_LINK_RESULT_INVALID,
    ACCOUNT_LINK_ISSUER_INVALID,
    ACCOUNT_LINK_NOT_POLLED,
    ACCOUNT_AUTH_UNAVAILABLE,
    PROVIDER_APP_UNSUPPORTED,
    PROVIDER_APP_REDIRECT_INVALID,
    PROVIDER_APP_INCOMPLETE,
    PROVIDER_SETTING_UNKNOWN,
];

/// A lists validation failure naming `reason`. `message` stays the sentence
/// shown where no translation is known and written to logs.
pub fn refused(reason: &'static str, message: impl Into<String>) -> AppError {
    AppError::validation_refused(reason, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_reason_is_a_distinct_lists_name_in_screaming_snake() {
        let distinct = ALL.iter().collect::<BTreeSet<_>>();
        assert_eq!(distinct.len(), ALL.len(), "a reason is listed twice");
        for reason in ALL {
            assert!(reason.starts_with("LIST_"), "{reason}");
            assert!(
                reason
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'),
                "{reason}"
            );
            assert!(!reason.ends_with('_') && !reason.contains("__"), "{reason}");
        }
    }

    #[test]
    fn a_refusal_reads_as_the_validation_failure_it_is() {
        let error = refused(ALREADY_FOLLOWED, "this list is already followed");
        assert_eq!(
            error.to_string(),
            "validation: this list is already followed"
        );
        assert_eq!(error.validation_reason(), Some(ALREADY_FOLLOWED));
        assert_eq!(
            AppError::Validation("this list is already followed".into()).validation_reason(),
            None
        );
    }
}
