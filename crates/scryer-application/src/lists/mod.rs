//! Lists: public and personal list subscriptions.
//!
//! This module owns the repository ports the sync engine and the GraphQL layer
//! read through, the null implementations an assembly without a list store
//! falls back to, and the sync engine itself: fetch → resolve → evaluate →
//! act → leave, driven by `sync`. `runtime` binds the engine's action and
//! resolver ports to the application's use cases.

pub mod act;
pub mod catalog;
pub mod evaluate;
pub mod fetch;
pub mod gateway;
pub mod leave;
pub mod null_repositories;
pub mod plugin;
pub mod ports;
pub mod privacy;
pub mod provider_settings;
pub mod public;
pub mod rejection;
pub mod resolve;
mod runtime;
#[cfg(test)]
pub(crate) use runtime::AppListActions;
pub mod sync;
#[cfg(test)]
pub(crate) mod test_support;
pub mod title_provenance;

pub use plugin::{ListPluginProvider, ListProviderClient, NullListPluginProvider};
pub use ports::{
    ListExclusionRepository, ListMembershipRepository, ListSubscriptionQuery,
    ListSubscriptionRepository, UserListAccountRepository, UserListPolicyRepository,
};
pub use provider_settings::{ListProviderConfigs, ListProviderSettingField, ListProviderSettings};
pub use public::{
    ListExclusionView, ListMembershipPage, ListPreview, ListPreviewItem, ListSourceDraft,
    MemberListPolicy, NewListExclusionInput, PublicListInput, PublicListPatch,
};
pub use sync::{ListSyncReport, list_sync_summary};
pub use title_provenance::TitleListMembership;
