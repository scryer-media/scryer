//! Stable names for the conditions a lists request is refused on.
//!
//! Each one travels beside the English sentence as `extensions.reason` on a
//! `VALIDATION_ERROR`, so a client can say the same thing in its own language
//! instead of showing or matching the sentence. A name states the condition,
//! never the wording, and is API once released: reword a sentence freely, but
//! do not rename or reuse its reason.
//!
//! An account authentication failure names one too: each code in
//! `account_transport::AUTH_FAILURE_CODES` has an `ACCOUNT_AUTH_*` reason
//! here. The rate-limited one is a `TEMPORARY_UNAVAILABLE` rather than a
//! `VALIDATION_ERROR`, and carries its reason the same way.
//!
//! Some lists failures carry no reason. A sentence that only relays what a
//! provider answered (a failed preview fetch) has nothing stable to name. The
//! malformed-filter guards in the GraphQL mappers reject shapes no client
//! builds on purpose. And the refusal while lists are switched off is raised
//! before any lists request is looked at.

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
/// A route names an audio language Scryer does not know.
pub const ROUTE_AUDIO_LANGUAGE_UNKNOWN: &str = "LIST_ROUTE_AUDIO_LANGUAGE_UNKNOWN";
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
/// The provider's accounts are not linked with a key the member supplies.
pub const ACCOUNT_KEY_UNSUPPORTED: &str = "LIST_ACCOUNT_KEY_UNSUPPORTED";
/// The provider's accounts are linked with the member's own key, and the
/// link was started without one.
pub const ACCOUNT_KEY_REQUIRED: &str = "LIST_ACCOUNT_KEY_REQUIRED";
/// The key the member supplied is empty, oversized, or not a single line.
pub const ACCOUNT_KEY_INVALID: &str = "LIST_ACCOUNT_KEY_INVALID";
/// The provider refused the key the member supplied.
pub const ACCOUNT_KEY_REJECTED: &str = "LIST_ACCOUNT_KEY_REJECTED";
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
/// The member or the provider declined the sign-in.
pub const ACCOUNT_AUTH_ACCESS_DENIED: &str = "LIST_ACCOUNT_AUTH_ACCESS_DENIED";
/// The provider did not accept the approval or code the sign-in ended with.
pub const ACCOUNT_AUTH_AUTHORIZATION_FAILED: &str = "LIST_ACCOUNT_AUTH_AUTHORIZATION_FAILED";
/// This instance is already handling as many account sign-ins as it allows.
pub const ACCOUNT_AUTH_BUSY: &str = "LIST_ACCOUNT_AUTH_BUSY";
/// The sign-in relay did not recognise this instance.
pub const ACCOUNT_AUTH_INSTANCE_AUTH_REQUIRED: &str = "LIST_ACCOUNT_AUTH_INSTANCE_AUTH_REQUIRED";
/// This instance could not prove its identity to the sign-in relay.
pub const ACCOUNT_AUTH_INSTANCE_AUTH_UNAVAILABLE: &str =
    "LIST_ACCOUNT_AUTH_INSTANCE_AUTH_UNAVAILABLE";
/// The instance's own provider app has an unusable callback address.
pub const ACCOUNT_AUTH_INVALID_PROVIDER_APP: &str = "LIST_ACCOUNT_AUTH_INVALID_PROVIDER_APP";
/// The provider's answer to a sign-in was malformed or out of bounds.
pub const ACCOUNT_AUTH_INVALID_PROVIDER_RESPONSE: &str =
    "LIST_ACCOUNT_AUTH_INVALID_PROVIDER_RESPONSE";
/// A value in the sign-in request was empty, oversized or malformed.
pub const ACCOUNT_AUTH_INVALID_REQUEST: &str = "LIST_ACCOUNT_AUTH_INVALID_REQUEST";
/// The provider's sign-in is not set up, or its app credentials were refused.
pub const ACCOUNT_AUTH_PROVIDER_NOT_CONFIGURED: &str = "LIST_ACCOUNT_AUTH_PROVIDER_NOT_CONFIGURED";
/// The provider refused the sign-in without saying why.
pub const ACCOUNT_AUTH_PROVIDER_REJECTED: &str = "LIST_ACCOUNT_AUTH_PROVIDER_REJECTED";
/// The provider could not answer the sign-in.
pub const ACCOUNT_AUTH_PROVIDER_UNAVAILABLE: &str = "LIST_ACCOUNT_AUTH_PROVIDER_UNAVAILABLE";
/// The provider or the relay asked for fewer sign-in requests.
pub const ACCOUNT_AUTH_RATE_LIMITED: &str = "LIST_ACCOUNT_AUTH_RATE_LIMITED";
/// The provider no longer accepts the grant or code the sign-in presented.
pub const ACCOUNT_AUTH_RECONNECT_REQUIRED: &str = "LIST_ACCOUNT_AUTH_RECONNECT_REQUIRED";
/// The sign-in relay could not answer.
pub const ACCOUNT_AUTH_RELAY_UNAVAILABLE: &str = "LIST_ACCOUNT_AUTH_RELAY_UNAVAILABLE";
/// This instance could not reach the provider or the relay.
pub const ACCOUNT_AUTH_TRANSPORT_UNAVAILABLE: &str = "LIST_ACCOUNT_AUTH_TRANSPORT_UNAVAILABLE";
/// The provider's sign-in does not finish by exchanging a code.
pub const ACCOUNT_AUTH_UNSUPPORTED_EXCHANGE: &str = "LIST_ACCOUNT_AUTH_UNSUPPORTED_EXCHANGE";
/// The provider's sign-in does not finish by polling.
pub const ACCOUNT_AUTH_UNSUPPORTED_POLL: &str = "LIST_ACCOUNT_AUTH_UNSUPPORTED_POLL";
/// This instance has no sign-in for the provider.
pub const ACCOUNT_AUTH_UNSUPPORTED_PROVIDER: &str = "LIST_ACCOUNT_AUTH_UNSUPPORTED_PROVIDER";
/// The provider's sign-in does not take an instance-owned app.
pub const ACCOUNT_AUTH_UNSUPPORTED_PROVIDER_APP: &str =
    "LIST_ACCOUNT_AUTH_UNSUPPORTED_PROVIDER_APP";
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
    ROUTE_AUDIO_LANGUAGE_UNKNOWN,
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
    ACCOUNT_KEY_UNSUPPORTED,
    ACCOUNT_KEY_REQUIRED,
    ACCOUNT_KEY_INVALID,
    ACCOUNT_KEY_REJECTED,
    ACCOUNT_LINK_ORIGIN_INVALID,
    ACCOUNT_LINKS_TOO_MANY,
    ACCOUNT_LINK_RESULT_INVALID,
    ACCOUNT_LINK_ISSUER_INVALID,
    ACCOUNT_LINK_NOT_POLLED,
    ACCOUNT_AUTH_UNAVAILABLE,
    ACCOUNT_AUTH_ACCESS_DENIED,
    ACCOUNT_AUTH_AUTHORIZATION_FAILED,
    ACCOUNT_AUTH_BUSY,
    ACCOUNT_AUTH_INSTANCE_AUTH_REQUIRED,
    ACCOUNT_AUTH_INSTANCE_AUTH_UNAVAILABLE,
    ACCOUNT_AUTH_INVALID_PROVIDER_APP,
    ACCOUNT_AUTH_INVALID_PROVIDER_RESPONSE,
    ACCOUNT_AUTH_INVALID_REQUEST,
    ACCOUNT_AUTH_PROVIDER_NOT_CONFIGURED,
    ACCOUNT_AUTH_PROVIDER_REJECTED,
    ACCOUNT_AUTH_PROVIDER_UNAVAILABLE,
    ACCOUNT_AUTH_RATE_LIMITED,
    ACCOUNT_AUTH_RECONNECT_REQUIRED,
    ACCOUNT_AUTH_RELAY_UNAVAILABLE,
    ACCOUNT_AUTH_TRANSPORT_UNAVAILABLE,
    ACCOUNT_AUTH_UNSUPPORTED_EXCHANGE,
    ACCOUNT_AUTH_UNSUPPORTED_POLL,
    ACCOUNT_AUTH_UNSUPPORTED_PROVIDER,
    ACCOUNT_AUTH_UNSUPPORTED_PROVIDER_APP,
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

/// [`refused`] carrying the values a client needs, each as its own extension.
pub fn refused_with_facts(
    reason: &'static str,
    message: impl Into<String>,
    facts: Vec<crate::RefusalFact>,
) -> AppError {
    AppError::validation_refused_with_facts(reason, message, facts)
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
