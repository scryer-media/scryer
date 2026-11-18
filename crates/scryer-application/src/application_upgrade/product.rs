//! Scryer's identity as seen by the shared application-upgrade core.
//!
//! Every value here is part of a frozen contract: it appears in a signed
//! manifest, on disk, in the registry, or in a process argument, and a helper
//! from one release must still read a plan or journal written by another.

use application_updater::ProductDescriptor;

/// The schema identifier accepted for signed upgrade manifests.
pub const UPGRADE_MANIFEST_SCHEMA_VERSION: &str = "scryer.upgrade.manifest.v1";

/// The schema identifier accepted for signed v2 upgrade manifests.
///
/// v1 is frozen: its parser denies unknown fields and unknown enum values, so
/// every shipped client rejects the whole document on anything new. v2 exists
/// to carry everything added from now on, and is published beside v1.
pub const UPGRADE_MANIFEST_V2_SCHEMA_VERSION: &str = "scryer.upgrade.manifest.v2";

/// The schema identifier written into the durable upgrade journal.
pub const JOURNAL_SCHEMA: &str = "scryer.upgrade.journal.v1";

/// The schema identifier for the Windows upgrade helper plan.
pub const APPLICATION_UPGRADE_HELPER_PLAN_SCHEMA: &str = "scryer.upgrade.helper-plan.v1";

/// Scryer's product identity for the shared upgrade core.
pub const SCRYER_PRODUCT: ProductDescriptor = ProductDescriptor {
    display_name: "Scryer",
    release_repository: "scryer-media/scryer",
    release_workflow: ".github/workflows/scryer.yml",
    manifest_asset_name: "scryer-upgrade-manifest.json",
    manifest_signature_asset_name: "scryer-upgrade-manifest.json.sigstore.json",
    manifest_schema: UPGRADE_MANIFEST_SCHEMA_VERSION,
    manifest_v2_asset_name: "scryer-upgrade-manifest.v2.json",
    manifest_v2_signature_asset_name: "scryer-upgrade-manifest.v2.json.sigstore.json",
    manifest_v2_schema: UPGRADE_MANIFEST_V2_SCHEMA_VERSION,
    macos_bundle_name: "Scryer.app",
    journal_schema: JOURNAL_SCHEMA,
    helper_plan_schema: APPLICATION_UPGRADE_HELPER_PLAN_SCHEMA,
    windows_server_executable: "scryer.exe",
    windows_tray_executable: "scryer-tray.exe",
    windows_helper_executable: "scryer-upgrade-helper.exe",
    windows_product_process_names: &["scryer.exe", "scryer-tray.exe"],
    // Keeps its long name: the journal of the release that staged a bundle
    // names this directory, and the release that finishes the upgrade removes
    // it only when it carries this one prefix.
    staged_replacement_prefix: ".scryer-upgrade-new-",
    disable_self_upgrade_env: "SCRYER_DISABLE_SELF_UPGRADE",
    package_env: "SCRYER_PACKAGE",
    tray_supervised_env: "SCRYER_TRAY_SUPERVISED",
    windows_registry_key: "Software\\Scryer Media\\Scryer",
    // Not a cross-release contract: each probe is created and removed by the
    // same call.
    write_probe_prefix: ".scry-wp-",
    http_operation_timeout: scryer_outbound_http::LONG_RUNNING_HTTP_OPERATION_TIMEOUT,
};

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire identities the shipped releases already use. Changing one of
    /// these breaks cross-version upgrades, so they are asserted literally.
    #[test]
    fn product_identity_matches_the_shipped_wire_contract() {
        assert_eq!(SCRYER_PRODUCT.manifest_schema, "scryer.upgrade.manifest.v1");
        assert_eq!(
            SCRYER_PRODUCT.manifest_asset_name,
            "scryer-upgrade-manifest.json"
        );
        assert_eq!(
            SCRYER_PRODUCT.manifest_signature_asset_name,
            "scryer-upgrade-manifest.json.sigstore.json"
        );
        assert_eq!(
            SCRYER_PRODUCT.manifest_v2_schema,
            "scryer.upgrade.manifest.v2"
        );
        assert_eq!(
            SCRYER_PRODUCT.manifest_v2_asset_name,
            "scryer-upgrade-manifest.v2.json"
        );
        assert_eq!(
            SCRYER_PRODUCT.manifest_v2_signature_asset_name,
            "scryer-upgrade-manifest.v2.json.sigstore.json"
        );
        assert_eq!(SCRYER_PRODUCT.macos_bundle_name, "Scryer.app");
        assert_eq!(SCRYER_PRODUCT.journal_schema, "scryer.upgrade.journal.v1");
        assert_eq!(
            SCRYER_PRODUCT.helper_plan_schema,
            "scryer.upgrade.helper-plan.v1"
        );
        assert_eq!(SCRYER_PRODUCT.release_repository, "scryer-media/scryer");
        assert_eq!(
            SCRYER_PRODUCT.release_workflow,
            ".github/workflows/scryer.yml"
        );
        assert_eq!(SCRYER_PRODUCT.windows_server_executable, "scryer.exe");
        assert_eq!(SCRYER_PRODUCT.windows_tray_executable, "scryer-tray.exe");
        assert_eq!(
            SCRYER_PRODUCT.windows_registry_key,
            "Software\\Scryer Media\\Scryer"
        );
        assert_eq!(
            SCRYER_PRODUCT.disable_self_upgrade_env,
            "SCRYER_DISABLE_SELF_UPGRADE"
        );
        assert_eq!(SCRYER_PRODUCT.package_env, "SCRYER_PACKAGE");
        assert_eq!(SCRYER_PRODUCT.tray_supervised_env, "SCRYER_TRAY_SUPERVISED");
        assert_eq!(SCRYER_PRODUCT.write_probe_prefix, ".scry-wp-");
        assert_eq!(
            SCRYER_PRODUCT.staged_replacement_prefix,
            ".scryer-upgrade-new-"
        );
    }

    #[test]
    fn scryer_release_signer_is_pinned_to_the_release_workflow() {
        let signer = SCRYER_PRODUCT.release_required_signer("scryer-v0.19.4");
        assert_eq!(signer.github_repository, "scryer-media/scryer");
        assert_eq!(
            signer.github_workflow.as_deref(),
            Some(".github/workflows/scryer.yml")
        );
        assert_eq!(
            signer.github_ref.as_deref(),
            Some("refs/tags/scryer-v0.19.4")
        );
    }
}
