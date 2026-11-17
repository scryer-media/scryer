//! Explicit recovery entrypoint, usable from a fresh portable binary even when
//! the installed server cannot start. No application database is opened.

// Compile the orchestration on non-Windows test hosts as well as Windows.
#![cfg_attr(not(windows), allow(dead_code))]

use application_updater::manifest::{UpgradeArchive, UpgradePlatform};
use application_updater::pipeline::{
    DiscardProgress, application_upgrade_http_client, download_artifact, fetch_upgrade_manifest,
    verify_artifact_hash,
};
use scryer_application::application_upgrade::{SCRYER_PRODUCT, msi_source::retain_installer};
use scryer_application::disk::{Disk, Parents};
use serde::Deserialize;
use std::path::Path;

async fn create_dir(path: &Path) -> Result<(), String> {
    Disk::process()
        .create_dir(path, Parents::Create)
        .await
        .map_err(|e| e.words().to_string())
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct InstallerIdentity {
    product_code: String,
    package_code: String,
    version: String,
}

impl InstallerIdentity {
    fn matches(&self, other: &Self) -> bool {
        self.product_code.eq_ignore_ascii_case(&other.product_code)
            && self.package_code.eq_ignore_ascii_case(&other.package_code)
            && self.version == other.version
    }
}

#[derive(Deserialize)]
struct Discovery {
    cache: std::path::PathBuf,
    products: Vec<InstalledProduct>,
}

#[derive(Deserialize)]
struct InstalledProduct {
    #[serde(flatten)]
    identity: InstallerIdentity,
    arch: application_updater::manifest::UpgradeArchitecture,
}

async fn powershell(
    operation: &str,
    path: Option<&Path>,
    identity: Option<&InstallerIdentity>,
) -> Result<Vec<u8>, String> {
    let mut command = tokio::process::Command::new("powershell.exe");
    // Keep the double-click setup graphical while the helper runs.
    #[cfg(windows)]
    command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    command.args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        include_str!("windows_installer_recovery.ps1"),
    ]);
    command
        .env("SCRYER_MSI_OPERATION", operation)
        .kill_on_drop(true);
    if let Some(path) = path {
        command.env("SCRYER_MSI_PACKAGE", path);
    }
    if let Some(identity) = identity {
        command
            .env("SCRYER_MSI_PRODUCT_CODE", &identity.product_code)
            .env("SCRYER_MSI_PACKAGE_CODE", &identity.package_code)
            .env("SCRYER_MSI_VERSION", &identity.version);
    }
    let budget = if operation == "install" { 1830 } else { 330 };
    let output = tokio::time::timeout(std::time::Duration::from_secs(budget), command.output())
        .await
        .map_err(|_| {
            "Installer recovery timed out; inspect the repair log before retrying".to_string()
        })?
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    Ok(output.stdout)
}

pub async fn repair() -> Result<(), String> {
    scryer_outbound_http::install_default_rustls_provider();
    let discovery: Discovery = serde_json::from_slice(&powershell("discover", None, None).await?)
        .map_err(|e| e.to_string())?;
    let client = application_upgrade_http_client(&SCRYER_PRODUCT).map_err(|e| e.to_string())?;
    for product in discovery.products {
        let installed = product.identity;
        let parts: Vec<_> = installed.version.split('.').collect();
        if parts.len() != 3
            || parts.iter().any(|part| {
                part.parse::<u16>()
                    .ok()
                    .is_none_or(|n| n.to_string() != *part)
            })
        {
            return Err("Installed MSI has an unsupported version".into());
        }
        let tag = format!("scryer-v{}", installed.version);
        println!("Recovering the original installer for {tag}...");
        let manifest = fetch_upgrade_manifest(&SCRYER_PRODUCT, &client, &tag)
            .await
            .map_err(|e| e.to_string())?
            .manifest;
        let mut restored = false;
        let disk = Disk::process();
        for artifact in manifest.artifacts.iter().filter(|a| {
            a.platform == UpgradePlatform::Windows
                && a.archive == UpgradeArchive::Msi
                && a.arch == product.arch
        }) {
            let source = discovery.cache.join(&artifact.blake3).join("installer.msi");
            if disk.follow(&source).await.exists() != Some(true) {
                // Do not overwrite a download or source another invocation may
                // be using. Completed sources are reused only after revalidation.
                let download_dir = discovery
                    .cache
                    .join("downloads")
                    .join(uuid::Uuid::new_v4().to_string());
                create_dir(&download_dir).await?;
                let download = download_dir.join("installer.msi");
                download_artifact(&client, artifact, None, &download, &mut DiscardProgress)
                    .await
                    .map_err(|e| e.to_string())?;
                retain_installer(&download, &discovery.cache, artifact)
                    .map_err(|e| e.to_string())?;
            }
            verify_artifact_hash(&source, artifact).map_err(|e| e.to_string())?;
            let identity: InstallerIdentity =
                serde_json::from_slice(&powershell("inspect", Some(&source), None).await?)
                    .map_err(|e| e.to_string())?;
            if !installed.matches(&identity) {
                continue;
            }
            let result = powershell("recache", Some(&source), Some(&installed)).await?;
            let result: serde_json::Value =
                serde_json::from_slice(&result).map_err(|e| e.to_string())?;
            println!(
                "Installer source restored for {tag}. Reboot required: {}",
                result["reboot_required"]
            );
            restored = true;
            break;
        }
        if !restored {
            return Err(format!(
                "No signed original MSI matches the registered product and package for {tag}; registration was preserved"
            ));
        }
    }
    println!("Installer recovery complete. You can now run the new Scryer MSI.");
    Ok(())
}

/// The setup executable pins its destination to its own compiled release.
/// A failed repair prevents the new MSI from starting.
#[allow(
    dead_code,
    reason = "This module is shared by scryer (repair only) and scryer-setup (install)."
)]
pub async fn install(version: &str) -> Result<bool, String> {
    repair().await?;
    let discovery: Discovery = serde_json::from_slice(&powershell("discover", None, None).await?)
        .map_err(|e| e.to_string())?;
    let client = application_upgrade_http_client(&SCRYER_PRODUCT).map_err(|e| e.to_string())?;
    let manifest = fetch_upgrade_manifest(&SCRYER_PRODUCT, &client, &format!("scryer-v{version}"))
        .await
        .map_err(|e| e.to_string())?
        .manifest;
    let arch = match std::env::consts::ARCH {
        "x86_64" => application_updater::manifest::UpgradeArchitecture::X86_64,
        "aarch64" => application_updater::manifest::UpgradeArchitecture::Arm64,
        _ => return Err("This setup download does not support this Windows architecture.".into()),
    };
    let artifact = manifest
        .artifacts
        .iter()
        .find(|a| {
            a.platform == UpgradePlatform::Windows
                && a.archive == UpgradeArchive::Msi
                && a.arch == arch
        })
        .ok_or("The signed release does not contain this Windows installer.")?;
    let source = discovery.cache.join(&artifact.blake3).join("installer.msi");
    if Disk::process().follow(&source).await.exists() != Some(true) {
        let directory = discovery
            .cache
            .join("downloads")
            .join(uuid::Uuid::new_v4().to_string());
        create_dir(&directory).await?;
        let download = directory.join("installer.msi");
        download_artifact(&client, artifact, None, &download, &mut DiscardProgress)
            .await
            .map_err(|e| e.to_string())?;
        retain_installer(&download, &discovery.cache, artifact).map_err(|e| e.to_string())?;
    }
    verify_artifact_hash(&source, artifact).map_err(|e| e.to_string())?;
    let result = powershell("install", Some(&source), None).await?;
    #[derive(Deserialize)]
    struct Installed {
        reboot_required: bool,
    }
    let result: Installed = serde_json::from_slice(&result).map_err(|e| e.to_string())?;
    Ok(result.reboot_required)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_requires_original_package_not_just_same_version() {
        let original = InstallerIdentity {
            product_code: "A".into(),
            package_code: "B".into(),
            version: "0.21.16".into(),
        };
        let mut candidate = InstallerIdentity {
            product_code: "a".into(),
            package_code: "b".into(),
            version: "0.21.16".into(),
        };
        assert!(original.matches(&candidate));
        candidate.package_code = "different".into();
        assert!(!original.matches(&candidate));
        candidate.package_code = "b".into();
        candidate.product_code = "different".into();
        assert!(!original.matches(&candidate));
    }
}
