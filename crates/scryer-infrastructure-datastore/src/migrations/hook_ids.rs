pub fn is_known_migration_hook_id(hook_id: &str) -> bool {
    match hook_id {
        "migrate_jellyfin_notification_channels_to_media_server_targets" => true,
        "migrate_title_root_folder_ids" => true,
        "migrate_title_catalog_sort_keys" => true,
        "migrate_title_folder_ownership" => true,
        "migrate_title_folder_ownership_safe" => true,
        "migrate_title_image_blobs" => true,
        "converge_post_0_16_6_prerelease_schema" => true,
        "backfill_canonical_download_identity" => true,
        "disable_invalid_user_rule_runtime_wrappers" => true,
        "backfill_blake3_identities" => true,
        "compact_event_storage" => true,
        "migrate_synthetic_root_ids" => true,
        "adopt_existing_title_tag_definitions" => true,
        #[cfg(test)]
        "test_insert_hook_marker" => true,
        _ => false,
    }
}

pub fn validate_migration_hook_id(hook_id: &str) -> Result<(), String> {
    if is_known_migration_hook_id(hook_id) {
        Ok(())
    } else {
        Err(format!("unknown migration hook id '{hook_id}'"))
    }
}
