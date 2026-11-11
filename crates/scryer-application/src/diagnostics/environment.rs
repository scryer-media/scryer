//! Environment facts the application layer owns: datastore, setup state,
//! instance settings that are enums, numbers or booleans, and filesystem facts
//! per library root and download root, each identified only by pseudonym.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use super::Omission;
use super::gather::{download_roots, error_kind};
use crate::AppUseCase;

/// Filesystem facts for one root, identified by pseudonym.
#[derive(Debug, Default)]
struct RootFacts {
    exists: bool,
    filesystem_type: Option<String>,
    total_bytes: Option<u64>,
    available_bytes: Option<u64>,
    device: Option<u64>,
}

impl AppUseCase {
    pub(super) async fn diagnostics_environment(
        &self,
        actor: &crate::User,
        environment: &mut Map<String, Value>,
        omissions: &mut Vec<Omission>,
    ) {
        match self.services.config.system_info.datastore_info().await {
            Ok(info) => {
                environment.insert(
                    "datastore".to_string(),
                    json!({
                        "engine": info.engine,
                        "current_migration_key": info.current_migration_key,
                    }),
                );
            }
            Err(error) => omissions.push(Omission::new(
                "environment.datastore",
                format!("read_failed:{}", error_kind(&error)),
            )),
        }

        match self.setup_complete().await {
            Ok(complete) => {
                environment.insert("setup_complete".to_string(), json!(complete));
            }
            Err(error) => omissions.push(Omission::new(
                "environment.setup_complete",
                format!("read_failed:{}", error_kind(&error)),
            )),
        }

        match self.smg_version_compatibility_notice().await {
            Ok(notice) => {
                environment.insert(
                    "smg_compatibility_notice".to_string(),
                    match notice {
                        Some(notice) => json!({
                            "present": true,
                            "status": notice.status,
                            "minimum_version": notice.minimum_version,
                            "your_version": notice.your_version,
                            "has_upgrade_deadline": notice.upgrade_deadline.is_some(),
                        }),
                        None => json!({ "present": false }),
                    },
                );
            }
            Err(error) => {
                environment.insert(
                    "smg_compatibility_notice".to_string(),
                    json!({
                        "present": true,
                        "reason": error.validation_reason().unwrap_or("unavailable"),
                    }),
                );
            }
        }

        let mut settings = Map::new();
        match self.general_settings().await {
            Ok(general) => {
                settings.insert(
                    "general".to_string(),
                    json!({
                        "api_explorer_enabled": general.api_explorer_enabled,
                        "experimental_features_enabled": general.experimental_features_enabled,
                        "personalized_discovery_enabled": general.personalized_discovery_enabled,
                        "srrdb_filename_recovery_enabled": general.srrdb_filename_recovery_enabled,
                        "keep_history_forever": general.keep_history_forever,
                        "history_retention_days": general.history_retention_days,
                        "image_cache_max_size_mb": general.image_cache_max_size_mb,
                        "image_cache_max_size_env_override_active": general.image_cache_max_size_env_override_active,
                        "plugin_http_ca_bundle_configured": !general.plugin_http_ca_bundle_pem.trim().is_empty(),
                        "plugin_http_trusted_certificates": general.plugin_http_trusted_certificates.len(),
                    }),
                );
            }
            Err(error) => omissions.push(Omission::new(
                "environment.settings.general",
                format!("read_failed:{}", error_kind(&error)),
            )),
        }
        match self.acquisition_settings().await {
            Ok(acquisition) => {
                settings.insert(
                    "acquisition".to_string(),
                    json!({
                        "enabled": acquisition.enabled,
                        "poll_interval_seconds": acquisition.poll_interval_seconds,
                        "walk_interval_seconds": acquisition.walk_interval_seconds,
                        "walk_intensity": format!("{:?}", acquisition.walk_intensity).to_ascii_lowercase(),
                        "long_tail_backfill_max_scopes_per_cycle": acquisition.long_tail_backfill_max_scopes_per_cycle,
                        "long_tail_reconverge_days": acquisition.long_tail_reconverge_days,
                    }),
                );
            }
            Err(error) => omissions.push(Omission::new(
                "environment.settings.acquisition",
                format!("read_failed:{}", error_kind(&error)),
            )),
        }
        match self.subtitle_settings().await {
            Ok(subtitles) => {
                settings.insert(
                    "subtitles".to_string(),
                    json!({
                        "enabled": subtitles.enabled,
                        "languages": subtitles.languages.len(),
                        "auto_download_on_import": subtitles.auto_download_on_import,
                        "minimum_score_series": subtitles.minimum_score_series,
                        "minimum_score_movie": subtitles.minimum_score_movie,
                        "search_interval_hours": subtitles.search_interval_hours,
                        "include_ai_translated": subtitles.include_ai_translated,
                        "include_machine_translated": subtitles.include_machine_translated,
                        "sync_enabled": subtitles.sync_enabled,
                        "sync_threshold_series": subtitles.sync_threshold_series,
                        "sync_threshold_movie": subtitles.sync_threshold_movie,
                        "sync_max_offset_seconds": subtitles.sync_max_offset_seconds,
                    }),
                );
            }
            Err(error) => omissions.push(Omission::new(
                "environment.settings.subtitles",
                format!("read_failed:{}", error_kind(&error)),
            )),
        }
        match self.get_recycle_bin_settings(actor).await {
            Ok(recycle) => {
                settings.insert(
                    "recycle_bin".to_string(),
                    json!({
                        "enabled": recycle.enabled,
                        "custom_path_configured": recycle.path.is_some(),
                        "retention_days": recycle.retention_days,
                        "effective_paths": recycle.effective_paths.len(),
                        "has_validation_error": recycle.validation_error.is_some(),
                    }),
                );
            }
            Err(error) => omissions.push(Omission::new(
                "environment.settings.recycle_bin",
                format!("read_failed:{}", error_kind(&error)),
            )),
        }
        environment.insert("settings".to_string(), Value::Object(settings));

        let public_url = self.public_url_runtime().snapshot();
        let trusted_proxies = self.trusted_proxy_runtime().snapshot();
        environment.insert(
            "network_configuration".to_string(),
            json!({
                "public_url_configured": public_url.configured_value().is_some(),
                "trusted_proxies_configured": !trusted_proxies.addresses.is_empty(),
                "trusted_proxy_count": trusted_proxies.addresses.len(),
            }),
        );

        match self.services.identity.users.list_all().await {
            Ok(users) => {
                environment.insert("user_count".to_string(), json!(users.len()));
            }
            Err(error) => omissions.push(Omission::new(
                "environment.user_count",
                format!("read_failed:{}", error_kind(&error)),
            )),
        }

        self.filesystem_facts(environment, omissions).await;
    }

    async fn filesystem_facts(
        &self,
        environment: &mut Map<String, Value>,
        omissions: &mut Vec<Omission>,
    ) {
        // Pseudonyms follow the known-value harvest's order, so `root-N` and
        // `download-root-N` here are the same roots the logs name.
        let mut roots: Vec<(String, &'static str, String)> = Vec::new();
        match self.services.catalog.libraries.list(None).await {
            Ok(libraries) => {
                let mut index = 0;
                for library in &libraries {
                    for root in &library.roots {
                        index += 1;
                        roots.push((format!("root-{index}"), "library_root", root.path.clone()));
                    }
                }
            }
            Err(error) => omissions.push(Omission::new(
                "environment.filesystems.library_roots",
                format!("read_failed:{}", error_kind(&error)),
            )),
        }
        match self
            .services
            .integrations
            .download_client_configs
            .list(None)
            .await
        {
            Ok(clients) => {
                let mut index = 0;
                for client in &clients {
                    for root in download_roots(&client.config_json) {
                        index += 1;
                        roots.push((format!("download-root-{index}"), "download_root", root));
                    }
                }
            }
            Err(error) => omissions.push(Omission::new(
                "environment.filesystems.download_roots",
                format!("read_failed:{}", error_kind(&error)),
            )),
        }

        let probed = tokio::task::spawn_blocking(move || {
            roots
                .into_iter()
                .map(|(pseudonym, role, path)| (pseudonym, role, probe_root(&path)))
                .collect::<Vec<_>>()
        })
        .await;
        let Ok(probed) = probed else {
            omissions.push(Omission::new(
                "environment.filesystems",
                "probe_task_failed",
            ));
            return;
        };

        // Devices become small group numbers so the raw device ids, which can
        // be stable host identifiers, are never written.
        let mut groups: BTreeMap<u64, usize> = BTreeMap::new();
        for (_, _, facts) in &probed {
            if let Some(device) = facts.device {
                let next = groups.len() + 1;
                groups.entry(device).or_insert(next);
            }
        }
        let mut hardlink_pairs = Vec::new();
        for (library_pseudonym, role, facts) in &probed {
            if *role != "library_root" {
                continue;
            }
            for (download_pseudonym, other_role, other) in &probed {
                if *other_role == "download_root"
                    && facts.device.is_some()
                    && facts.device == other.device
                {
                    hardlink_pairs.push(json!([library_pseudonym, download_pseudonym]));
                }
            }
        }
        let rows = probed
            .iter()
            .map(|(pseudonym, role, facts)| {
                json!({
                    "root": pseudonym,
                    "role": role,
                    "exists": facts.exists,
                    "filesystem_type": facts.filesystem_type,
                    "total_bytes": facts.total_bytes,
                    "available_bytes": facts.available_bytes,
                    "device_group": facts.device.and_then(|device| groups.get(&device).copied()),
                })
            })
            .collect::<Vec<_>>();
        environment.insert(
            "filesystems".to_string(),
            json!({
                "roots": rows,
                "same_filesystem_pairs": hardlink_pairs,
            }),
        );
        if cfg!(windows) {
            omissions.push(Omission::new(
                "environment.filesystems.type_and_device",
                "not_portable_on_windows",
            ));
        }
    }
}

fn probe_root(path: &str) -> RootFacts {
    let path = std::path::Path::new(path);
    let Ok(metadata) = std::fs::metadata(path) else {
        return RootFacts::default();
    };
    let space = crate::helpers::filesystem_space_raw(path).ok();
    RootFacts {
        exists: true,
        filesystem_type: filesystem_type(path),
        total_bytes: space.as_ref().map(|space| space.total_bytes),
        available_bytes: space.as_ref().map(|space| space.available_bytes),
        device: device_id(&metadata),
    }
}

#[cfg(unix)]
fn device_id(metadata: &std::fs::Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    Some(metadata.dev())
}

#[cfg(not(unix))]
fn device_id(_metadata: &std::fs::Metadata) -> Option<u64> {
    None
}

#[cfg(target_os = "macos")]
fn filesystem_type(path: &std::path::Path) -> Option<String> {
    use std::os::unix::ffi::OsStrExt;
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: `statfs` writes into a zeroed struct we own; the path is a valid
    // NUL-terminated string for the duration of the call.
    unsafe {
        let mut buf: libc::statfs = std::mem::zeroed();
        if libc::statfs(c_path.as_ptr(), &mut buf) != 0 {
            return None;
        }
        let name = std::ffi::CStr::from_ptr(buf.f_fstypename.as_ptr());
        Some(name.to_string_lossy().into_owned())
    }
}

#[cfg(target_os = "linux")]
fn filesystem_type(path: &std::path::Path) -> Option<String> {
    use std::os::unix::ffi::OsStrExt;
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: as above.
    let magic = unsafe {
        let mut buf: libc::statfs = std::mem::zeroed();
        if libc::statfs(c_path.as_ptr(), &mut buf) != 0 {
            return None;
        }
        buf.f_type as u64
    };
    Some(linux_filesystem_name(magic))
}

#[cfg(target_os = "linux")]
fn linux_filesystem_name(magic: u64) -> String {
    match magic {
        0xEF53 => "ext4",
        0x9123_683E => "btrfs",
        0x5846_5342 => "xfs",
        0x2FC1_2FC1 => "zfs",
        0x0102_1994 => "tmpfs",
        0x794C_7630 => "overlayfs",
        0x6969 => "nfs",
        0xFF53_4D42 => "cifs",
        0xFE53_4D42 => "smb2",
        0x517B => "smb",
        0x6573_5546 => "fuse",
        0x4D44 => "vfat",
        0x5346_544E => "ntfs",
        0x2011_BAB0 => "exfat",
        0xF2F5_2010 => "f2fs",
        0x0102_1997 => "9p",
        0x7366_7362 => "virtiofs",
        0xCA45_1A4E => "bcachefs",
        _ => return format!("0x{magic:x}"),
    }
    .to_string()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn filesystem_type(_path: &std::path::Path) -> Option<String> {
    None
}
