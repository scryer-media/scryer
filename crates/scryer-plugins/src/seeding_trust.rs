//! Version floor for the seeding verdicts a download-client plugin reports.
//!
//! `can_remove` and `can_move_files` are the two fields the seeding gate acts
//! on, and the audit that gave them their current meaning — `None` is a
//! legitimate steady state, `can_move_files` is a statement about the *data*
//! rather than permission to move it — landed *after* the published torrent
//! clients were built. Registry qBittorrent 1.0.5, for one, hardcodes
//! `can_remove: Some(true)`, which tells the gate that every imported torrent
//! is free to remove on the first post-import poll.
//!
//! Sonarr never has this problem: its clients ship in-tree, so `CanBeRemoved`
//! cannot be older than the code reading it (and defaults to `false`). Scryer's
//! plugin ABI can skew, so the host checks the plugin's own version against the
//! first release that reports under the audited semantics and, below it, drops
//! the verdicts it cannot trust to `None`.
//!
//! **Only positive verdicts are dropped**, because the two fields are not read
//! symmetrically. `can_remove: None` is `CLIENT_VERDICT_UNKNOWN` and the gate
//! holds — strictly safer than the `Some(true)` it replaces. `can_move_files`
//! is read as `observation.can_move_files != Some(false)`
//! (`import/seeding_gate.rs`), so `None` is *stable*: erasing a stale
//! `Some(false)` would upgrade a hardlink-or-copy import into a `Move` and take
//! the payload out from under a torrent that may still be seeding. A stale
//! "not stable" claim can only ever downgrade a `Move`, so it is kept
//! verbatim; Sonarr agrees, defaulting `CanMoveFiles` to `false` and importing
//! copy-only when it is not set.
//!
//! Scope: this is an anti-staleness measure, not a trust boundary. The table is
//! an allowlist of the first-party clients whose reporting semantics changed,
//! and the version it checks is self-reported by the plugin, so a plugin that
//! is not listed — or that misstates its version — is believed. Installing a
//! plugin is an admin action and plugin code already runs with host-granted
//! capabilities (see `SECURITY.md`); the floor exists so an *honest* published
//! build from before the audit cannot silently release torrents.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use semver::Version;
use tracing::warn;

use crate::types::{PluginDescriptor, PluginDownloadItem};

/// `plugin id → the first version that reports the audited seeding semantics`.
///
/// These are the versions the seeding sweep published (scryer-plugins
/// `c926bca`: every torrent client minor-bumped, rqbit already on 1.1.x and so
/// on 1.2.0). The 13 torrent clients are the only plugins with a seeding
/// obligation to report, so anything not listed — a usenet client, or a
/// third-party client the host knows nothing about — is left alone.
const SEEDING_VERDICT_FLOORS: &[(&str, &str)] = &[
    ("aria2", "1.1.0"),
    ("deluge", "1.1.0"),
    ("downloadstation", "1.1.0"),
    ("flood", "1.1.0"),
    ("freebox", "1.1.0"),
    ("hadouken", "1.1.0"),
    ("qbittorrent", "1.1.0"),
    ("rqbit", "1.2.0"),
    ("rtorrent", "1.1.0"),
    ("torrent-blackhole", "1.1.0"),
    ("transmission", "1.1.0"),
    ("tribler", "1.1.0"),
    ("utorrent", "1.1.0"),
];

/// Apply the floor to every item of one plugin listing.
pub(crate) fn apply_seeding_trust_floor(
    descriptor: &PluginDescriptor,
    items: &mut [PluginDownloadItem],
) {
    for item in items.iter_mut() {
        coerce_seeding_verdicts(descriptor, item);
    }
}

/// Drop the untrustworthy verdicts of a plugin that predates the seeding audit.
///
/// Applied on the way in from the plugin, before any observation is taken, so
/// there is one place where an untrusted verdict can enter the host and it is
/// upstream of every reader.
///
/// `can_remove` goes to `None` whatever it said: both signs were computed under
/// semantics the audit changed, and `None` holds. `can_move_files` loses only a
/// `Some(true)` — see the module doc: `None` reads as *stable*, so dropping a
/// `Some(false)` would permit a `Move` the plugin was refusing.
pub(crate) fn coerce_seeding_verdicts(
    descriptor: &PluginDescriptor,
    item: &mut PluginDownloadItem,
) {
    let Some(floor) = floor_for(&descriptor.id) else {
        return;
    };
    if reports_audited_verdicts(&descriptor.version, floor) {
        return;
    }
    item.can_remove = None;
    if item.can_move_files == Some(true) {
        item.can_move_files = None;
    }
    warn_once(descriptor.id.trim(), descriptor.version.trim(), floor);
}

fn floor_for(plugin_id: &str) -> Option<&'static str> {
    let plugin_id = plugin_id.trim();
    SEEDING_VERDICT_FLOORS
        .iter()
        .find(|(id, _)| id.eq_ignore_ascii_case(plugin_id))
        .map(|(_, floor)| *floor)
}

/// Whether this version is at or above the floor.
///
/// A version the host cannot parse is treated as below it: the plugin cannot
/// be *shown* to report the audited semantics, and "cannot be shown" is exactly
/// what the coerced `None` says. A floor the host cannot parse is a typo in the
/// table above rather than anything the operator's plugin did, so it trusts the
/// plugin instead of punishing it; `every_floor_is_valid_semver` keeps that
/// branch unreachable.
fn reports_audited_verdicts(version: &str, floor: &str) -> bool {
    let Ok(floor) = Version::parse(floor) else {
        return true;
    };
    Version::parse(version.trim()).is_ok_and(|version| version >= floor)
}

/// One line per stale plugin per process: the poll that trips this repeats
/// every cycle for every item the client holds.
fn warn_once(plugin_id: &str, plugin_version: &str, minimum_version: &str) {
    static WARNED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let Ok(mut warned) = WARNED.get_or_init(|| Mutex::new(HashSet::new())).lock() else {
        return;
    };
    if !warned.insert(format!("{plugin_id}@{plugin_version}")) {
        return;
    }
    warn!(
        plugin_id,
        plugin_version,
        minimum_version,
        "download-client plugin predates the seeding audit; ignoring its can_remove / can_move_files verdicts, so imported torrents are held until the plugin is updated"
    );
}

/// A download-client descriptor for one plugin id at one version.
#[cfg(test)]
pub(crate) fn descriptor(id: &str, version: &str) -> PluginDescriptor {
    use crate::types::{DownloadClientCapabilities, DownloadClientDescriptor, ProviderDescriptor};

    PluginDescriptor {
        settings: Vec::new(),
        id: id.to_string(),
        name: id.to_string(),
        version: version.to_string(),
        sdk_version: crate::types::SDK_VERSION.to_string(),
        sdk_constraint: String::new(),
        socket_permissions: Vec::new(),
        provider: ProviderDescriptor::DownloadClient(DownloadClientDescriptor {
            provider_type: id.to_string(),
            provider_aliases: Vec::new(),
            config_fields: Vec::new(),
            default_base_url: None,
            allowed_hosts: Vec::new(),
            accepted_inputs: Vec::new(),
            isolation_modes: Vec::new(),
            capabilities: DownloadClientCapabilities::default(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::DownloadItemState;

    fn item(can_remove: Option<bool>, can_move_files: Option<bool>) -> PluginDownloadItem {
        PluginDownloadItem {
            client_item_id: "item-1".to_string(),
            download_id: None,
            info_hash: None,
            title: "Item".to_string(),
            state: DownloadItemState::Seeding,
            message: None,
            category: None,
            remote_output_path: None,
            torrent: None,
            total_size_bytes: None,
            remaining_size_bytes: None,
            eta_seconds: None,
            progress_percent: None,
            can_move_files,
            can_remove,
            removed: None,
            raw_state: None,
            completed_at: None,
        }
    }

    fn coerced(plugin_id: &str, version: &str) -> PluginDownloadItem {
        let mut item = item(Some(true), Some(true));
        coerce_seeding_verdicts(&descriptor(plugin_id, version), &mut item);
        item
    }

    #[test]
    fn every_floor_is_valid_semver() {
        for (plugin_id, floor) in SEEDING_VERDICT_FLOORS {
            assert!(
                Version::parse(floor).is_ok(),
                "floor for {plugin_id} must be semver: {floor}"
            );
        }
    }

    #[test]
    fn a_plugin_below_the_floor_loses_its_positive_verdicts() {
        // The published registry build: it hardcodes `can_remove: Some(true)`
        // for every item, which would release every imported torrent.
        let item = coerced("qbittorrent", "1.0.5");
        assert_eq!(item.can_remove, None);
        assert_eq!(item.can_move_files, None);
    }

    #[test]
    fn a_plugin_at_the_floor_is_believed() {
        let item = coerced("qbittorrent", "1.1.0");
        assert_eq!(item.can_remove, Some(true));
        assert_eq!(item.can_move_files, Some(true));
    }

    #[test]
    fn a_plugin_above_the_floor_is_believed() {
        let item = coerced("qbittorrent", "1.4.2");
        assert_eq!(item.can_remove, Some(true));
        assert_eq!(item.can_move_files, Some(true));
    }

    #[test]
    fn rqbit_carries_its_own_higher_floor() {
        // rqbit was already on 1.1.x before the sweep, so 1.1.0 is a *pre*-audit
        // build there while it is the audited one everywhere else.
        assert_eq!(coerced("rqbit", "1.1.0").can_remove, None);
        assert_eq!(coerced("rqbit", "1.2.0").can_remove, Some(true));
    }

    #[test]
    fn an_unparsable_version_is_treated_as_below_the_floor() {
        for version in ["", "not-a-version", "1.1", "v1.1.0"] {
            let item = coerced("transmission", version);
            assert_eq!(item.can_remove, None, "version {version:?}");
            assert_eq!(item.can_move_files, None, "version {version:?}");
        }
    }

    #[test]
    fn a_plugin_with_no_floor_is_left_alone() {
        // Usenet clients have no seeding obligation to report, and a
        // third-party client the host has never heard of has no floor to
        // measure against; neither is coerced.
        for plugin_id in ["nzbvortex", "usenet-blackhole", "some-third-party-client"] {
            let item = coerced(plugin_id, "0.0.1");
            assert_eq!(item.can_remove, Some(true), "plugin {plugin_id}");
            assert_eq!(item.can_move_files, Some(true), "plugin {plugin_id}");
        }
    }

    /// The asymmetry the gate forces on us.
    ///
    /// `can_move_files: None` is read as *stable* (`!= Some(false)`), so
    /// erasing a stale "the data is not stable" would hand the importer a
    /// `Move` the plugin was refusing — pre-audit deluge reports
    /// `Some(ratio_goal_met)` and aria2 hardcodes `Some(false)`, so this is the
    /// common case, not a corner. `can_remove` has no such asymmetry: `None`
    /// holds.
    #[test]
    fn a_below_floor_refusal_to_move_survives_while_its_removal_verdict_does_not() {
        let mut item = item(Some(false), Some(false));
        coerce_seeding_verdicts(&descriptor("deluge", "1.0.2"), &mut item);
        assert_eq!(item.can_remove, None);
        assert_eq!(
            item.can_move_files,
            Some(false),
            "a stale refusal can only downgrade a Move, so it is kept"
        );
    }

    #[test]
    fn the_floor_applies_to_every_item_in_a_listing() {
        let mut items = vec![item(Some(true), Some(true)), item(Some(true), None)];
        apply_seeding_trust_floor(&descriptor("qbittorrent", "1.0.5"), &mut items);
        assert!(
            items
                .iter()
                .all(|item| item.can_remove.is_none() && item.can_move_files.is_none())
        );
    }
}
