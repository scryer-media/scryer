//! Durable, content-addressed Windows Installer sources, outside upgrade staging.

use std::path::{Path, PathBuf};

use application_updater::{manifest::UpgradeArtifact, pipeline::verify_artifact_hash};

use crate::disk::{Access, Disk, Open, Parents, Replace, reason};

/// Publish only verified bytes. An existing source is never overwritten, even
/// if damaged: it may still be registered to an installed product.
pub fn retain_installer(
    source: &Path,
    cache: &Path,
    artifact: &UpgradeArtifact,
) -> crate::AppResult<PathBuf> {
    verify_artifact_hash(source, artifact).map_err(super::map_updater_error)?;
    // The hash is both an identity and a path component. Check it even when
    // callers already validated the signed manifest.
    if artifact.blake3.len() != 64 || !artifact.blake3.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(crate::AppError::validation_refused(
            crate::error_reasons::application_upgrade::INSTALLER_DIGEST_INVALID,
            "invalid installer digest",
        ));
    }
    let directory = cache.join(&artifact.blake3);
    let disk = Disk::process();
    disk.create_dir_blocking(&directory, Parents::Create)
        .map_err(|e| {
            crate::AppError::Repository(format!("failed to create installer cache: {e}"))
        })?;
    let destination = directory.join("installer.msi");
    if disk.follow_blocking(&destination).exists() != Some(true) {
        // Keep partial copies in a staging file that is removed unless
        // published. Publish without replacing an existing source, including
        // when two recovery attempts race. A copy also supports Windows data
        // volumes without hard-link support.
        let io_error = |e: std::io::Error| {
            crate::AppError::Repository(format!("failed to retain installer: {e}"))
        };
        let mut staged = disk
            .stage_blocking(&destination, Parents::MustExist, Access::OwnerOnly)
            .map_err(|e| io_error(e.into()))?;
        let mut input = disk
            .open_blocking(source, Open::read())
            .map_err(|e| io_error(e.into()))?;
        std::io::copy(&mut input, staged.file()).map_err(io_error)?;
        verify_artifact_hash(staged.path(), artifact).map_err(super::map_updater_error)?;
        if let Err(error) = staged.commit_blocking(Replace::Never)
            && error.reason() != reason::DISK_DESTINATION_EXISTS
        {
            return Err(io_error(error.into()));
        }
    }
    verify_artifact_hash(&destination, artifact).map_err(super::map_updater_error)?;
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    use application_updater::manifest::{
        UpgradeArchitecture, UpgradeArchive, UpgradeChannel, UpgradePlatform,
    };

    fn artifact(bytes: &[u8]) -> UpgradeArtifact {
        UpgradeArtifact {
            platform: UpgradePlatform::Windows,
            arch: UpgradeArchitecture::X86_64,
            channel: UpgradeChannel::Msi,
            asset_name: "fixture.msi".into(),
            url: "https://example.invalid/fixture.msi".into(),
            size: bytes.len() as u64,
            blake3: blake3::hash(bytes).to_hex().to_string(),
            archive: UpgradeArchive::Msi,
            members: vec![],
        }
    }

    #[test]
    fn msi_sources_survive_staging_removal_and_later_versions() {
        let temp = tempfile::tempdir().unwrap();
        let staging = temp.path().join("staging");
        let cache = temp.path().join("installer-cache");
        std::fs::create_dir(&staging).unwrap();
        let source = staging.join("artifact");
        std::fs::write(&source, b"old installer").unwrap();
        let old = retain_installer(&source, &cache, &artifact(b"old installer")).unwrap();
        std::fs::remove_file(&source).unwrap();
        std::fs::write(&source, b"new installer").unwrap();
        let new = retain_installer(&source, &cache, &artifact(b"new installer")).unwrap();
        std::fs::remove_dir_all(&staging).unwrap();
        assert_ne!(old, new);
        assert_eq!(std::fs::read(old).unwrap(), b"old installer");
        assert_eq!(std::fs::read(new).unwrap(), b"new installer");
    }

    #[test]
    fn damaged_registered_source_is_not_overwritten() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("artifact");
        std::fs::write(&source, b"installer").unwrap();
        let artifact = artifact(b"installer");
        let destination = retain_installer(&source, temp.path(), &artifact).unwrap();
        std::fs::write(&destination, b"damaged").unwrap();
        assert!(retain_installer(&source, temp.path(), &artifact).is_err());
        assert_eq!(std::fs::read(destination).unwrap(), b"damaged");
    }
}
