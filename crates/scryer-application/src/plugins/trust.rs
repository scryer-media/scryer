//! Application adapter for the shared artifact verifier.
use std::sync::OnceLock;

use super::catalog::RequiredSigner;
use crate::{AppError, AppResult};

/// The trusted root release tooling materialized for this build, with its
/// receipt. It replaces the older copy embedded in `artifact-trust`.
const EMBEDDED_SIGSTORE_TRUST_ROOT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../scryer-plugins/builtins/sigstore-trusted-root.json"
));
const EMBEDDED_SIGSTORE_TRUST_ROOT_PROVENANCE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../scryer-plugins/builtins/sigstore-trusted-root.provenance.json"
));

static TRUST_SNAPSHOT_INSTALLED: OnceLock<Result<(), String>> = OnceLock::new();

fn map_trust_error(error: artifact_trust::Error) -> AppError {
    match error {
        artifact_trust::Error::Validation(message) => {
            AppError::validation_refused(crate::error_reasons::plugins::ARTIFACT_UNTRUSTED, message)
        }
        artifact_trust::Error::Repository(message) => AppError::Repository(message),
    }
}

/// Idempotent. Every path that verifies a signature calls this first, including
/// the upgrade engine, whose verification runs inside `application-updater`.
pub(crate) fn install_embedded_trust_snapshot() -> AppResult<()> {
    TRUST_SNAPSHOT_INSTALLED
        .get_or_init(|| {
            artifact_trust::install_sigstore_trust_snapshot(
                EMBEDDED_SIGSTORE_TRUST_ROOT,
                EMBEDDED_SIGSTORE_TRUST_ROOT_PROVENANCE,
            )
            .map_err(|error| error.to_string())
        })
        .clone()
        .map_err(AppError::Repository)
}

pub async fn verify_signed_blob(
    raw: Vec<u8>,
    bundle_raw: Vec<u8>,
    required_signer: RequiredSigner,
) -> AppResult<()> {
    install_embedded_trust_snapshot()?;
    artifact_trust::verify_signed_blob(raw, bundle_raw, required_signer)
        .await
        .map_err(map_trust_error)
}

pub async fn prime_sigstore_trust_roots() -> AppResult<()> {
    install_embedded_trust_snapshot()?;
    scryer_outbound_http::install_default_rustls_provider();
    artifact_trust::prime_sigstore_trust_roots()
        .await
        .map_err(map_trust_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_trust_snapshot_matches_its_receipt() {
        install_embedded_trust_snapshot().expect("the build's trust snapshot must install");
    }

    #[tokio::test]
    async fn shared_verifier_accepts_the_cosign_v2_upgrade_manifest() {
        let raw =
            include_bytes!("../../test-fixtures/sigstore/scryer-upgrade-manifest-v0.19.3.json");
        let bundle = include_bytes!(
            "../../test-fixtures/sigstore/scryer-upgrade-manifest-v0.19.3.sigstore.json"
        );
        let signer = RequiredSigner {
            github_repository: "scryer-media/scryer".into(),
            github_workflow: Some(".github/workflows/scryer.yml".into()),
            github_ref: Some("refs/tags/scryer-v0.19.3".into()),
        };
        verify_signed_blob(raw.to_vec(), bundle.to_vec(), signer.clone())
            .await
            .unwrap();
        let mut altered = raw.to_vec();
        altered[0] ^= 1;
        assert!(matches!(
            verify_signed_blob(altered, bundle.to_vec(), signer).await,
            Err(AppError::Validation(_)
                | AppError::ValidationRefused { .. }
                | AppError::ValidationRefusedWithCount { .. })
        ));
    }

    #[tokio::test]
    async fn shared_verifier_accepts_signed_plugin_and_preserves_tampering_errors() {
        let raw = include_bytes!("../../test-fixtures/sigstore/fanzub-catalog-v2.1.0.json");
        let bundle =
            include_bytes!("../../test-fixtures/sigstore/fanzub-catalog-v2.1.0.sigstore.json");
        let signer = RequiredSigner {
            github_repository: "scryer-media/scryer-plugins".into(),
            github_workflow: Some(".github/workflows/release-plugin-v3.yml".into()),
            github_ref: Some("refs/tags/plugins-v3/release/1787972354-109745e51603".into()),
        };
        verify_signed_blob(raw.to_vec(), bundle.to_vec(), signer.clone())
            .await
            .unwrap();
        let mut altered = raw.to_vec();
        altered[0] ^= 1;
        assert!(matches!(
            verify_signed_blob(altered, bundle.to_vec(), signer).await,
            Err(AppError::Validation(_)
                | AppError::ValidationRefused { .. }
                | AppError::ValidationRefusedWithCount { .. })
        ));
    }
}
