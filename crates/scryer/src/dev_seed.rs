use std::collections::HashMap;
use std::path::{Path, PathBuf};

use scryer_application::{AppUseCase, InsertMediaFileInput};
use scryer_domain::{
    ExternalId, MediaFacet, NewDownloadClientConfig, NewIndexerConfig, NewTitle, Title,
};
use scryer_infrastructure::SqliteServices;
use serde::Deserialize;

use crate::admin_routes::normalize_base_url;
use crate::settings_bootstrap::MOVIES_PATH_KEY;

#[derive(Deserialize)]
struct SeedConfig {
    admin: Option<SeedAdmin>,
    #[serde(default)]
    indexers: Vec<SeedIndexer>,
    #[serde(default)]
    download_clients: Vec<SeedDownloadClient>,
    #[serde(default)]
    settings: Vec<SeedSetting>,
}

#[derive(Deserialize)]
struct SeedTitlesConfig {
    #[serde(default)]
    movies: Vec<SeedTitle>,
    #[serde(default)]
    series: Vec<SeedTitle>,
    #[serde(default)]
    anime: Vec<SeedTitle>,
}

#[derive(Deserialize)]
struct SeedTitle {
    tvdb_id: i64,
    /// Optional name hint — used only for logging. SMG hydrates the real name.
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    quality_tier: Option<String>,
}

#[derive(Deserialize)]
struct SeedAdmin {
    password: String,
}

#[derive(Deserialize)]
struct SeedIndexer {
    name: String,
    provider_type: String,
    base_url: String,
    api_key: Option<String>,
    rate_limit_seconds: Option<i64>,
    rate_limit_burst: Option<i64>,
    #[serde(default = "default_true")]
    enabled: bool,
}

#[derive(Deserialize)]
struct SeedDownloadClient {
    name: String,
    client_type: String,
    base_url: Option<String>,
    config: toml::Value,
    #[serde(default = "default_true")]
    enabled: bool,
}

#[derive(Deserialize)]
struct SeedSetting {
    scope: String,
    key: String,
    value: toml::Value,
    scope_id: Option<String>,
}

fn default_true() -> bool {
    true
}

/// Convert a TOML value to a JSON string for storage.
fn toml_value_to_json_string(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => serde_json::to_string(s).unwrap(),
        toml::Value::Integer(n) => serde_json::to_string(n).unwrap(),
        toml::Value::Float(f) => serde_json::to_string(f).unwrap(),
        toml::Value::Boolean(b) => serde_json::to_string(b).unwrap(),
        toml::Value::Array(arr) => {
            let json_values: Vec<serde_json::Value> = arr
                .iter()
                .map(|v| serde_json::from_str(&toml_value_to_json_string(v)).unwrap())
                .collect();
            serde_json::to_string(&json_values).unwrap()
        }
        toml::Value::Table(table) => {
            let mut map = serde_json::Map::new();
            for (k, v) in table {
                let json_val: serde_json::Value =
                    serde_json::from_str(&toml_value_to_json_string(v)).unwrap();
                map.insert(k.clone(), json_val);
            }
            serde_json::to_string(&serde_json::Value::Object(map)).unwrap()
        }
        toml::Value::Datetime(dt) => serde_json::to_string(&dt.to_string()).unwrap(),
    }
}

const DEFAULT_MOVIE_MEDIA_ROOT: &str = "/media/movies";
const DEV_SEED_MOVIE_COLLECTION_INDEX: &str = "1";

fn read_seed_media_root(settings: &[SeedSetting], key: &str, fallback: &str) -> PathBuf {
    settings
        .iter()
        .rev()
        .find(|setting| setting.scope == "media" && setting.key == key)
        .and_then(|setting| setting.value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(fallback))
}

fn sanitize_path_component(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut last_was_sep = false;
    for ch in raw.chars() {
        let normalized = match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' => Some(ch),
            ' ' | '-' | '_' | '.' => Some(ch),
            '&' => Some('+'),
            _ => None,
        };
        if let Some(ch) = normalized {
            let is_sep = matches!(ch, ' ' | '-' | '_' | '.');
            if is_sep && last_was_sep {
                continue;
            }
            out.push(ch);
            last_was_sep = is_sep;
        } else if !last_was_sep {
            out.push(' ');
            last_was_sep = true;
        }
    }
    let trimmed = out.trim_matches(|ch: char| matches!(ch, ' ' | '-' | '_' | '.'));
    if trimmed.is_empty() {
        "untitled".to_string()
    } else {
        trimmed.to_string()
    }
}

fn normalize_seed_quality_tier(tvdb_id: i64, configured: Option<&str>) -> String {
    if let Some(value) = configured.map(str::trim).filter(|value| !value.is_empty()) {
        return value.to_ascii_uppercase();
    }
    match tvdb_id.rem_euclid(3) {
        0 => "2160P".to_string(),
        1 => "1080P".to_string(),
        _ => "720P".to_string(),
    }
}

fn seeded_movie_size_bytes(quality_tier: &str) -> u64 {
    match quality_tier.trim().to_ascii_uppercase().as_str() {
        "2160P" => 30 * 1024 * 1024 * 1024,
        "1080P" => 10 * 1024 * 1024 * 1024,
        "720P" => 4 * 1024 * 1024 * 1024,
        _ => 2 * 1024 * 1024 * 1024,
    }
}

fn build_seeded_movie_path(media_root: &Path, title_name: &str, quality_tier: &str) -> PathBuf {
    let folder_name = sanitize_path_component(title_name);
    let file_stem = sanitize_path_component(title_name).replace(' ', ".");
    media_root
        .join(&folder_name)
        .join(format!("{file_stem}.{quality_tier}.mkv"))
}

fn ensure_placeholder_media_file(path: &Path, size_bytes: u64) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(path)?;
    if file.metadata()?.len() < size_bytes {
        file.set_len(size_bytes)?;
    }
    Ok(())
}

async fn ensure_seeded_movie_import(
    app: &AppUseCase,
    actor: &scryer_domain::User,
    title: &Title,
    seed: &SeedTitle,
    movie_media_root: &Path,
) -> Result<bool, String> {
    let existing_collections = app.list_collections(actor, &title.id).await.map_err(|e| {
        format!(
            "dev seed: failed to list collections for '{}': {e}",
            title.name
        )
    })?;

    let quality_tier = normalize_seed_quality_tier(seed.tvdb_id, seed.quality_tier.as_deref());
    let file_path = build_seeded_movie_path(movie_media_root, &title.name, &quality_tier);
    let file_path_string = file_path.to_string_lossy().into_owned();
    let has_movie_collection = existing_collections
        .iter()
        .any(|collection| collection.collection_type.eq_ignore_ascii_case("movie"));
    let existing_media_files = app
        .services
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .map_err(|e| {
            format!(
                "dev seed: failed to list media files for '{}': {e}",
                title.name
            )
        })?;
    let has_media_file = existing_media_files
        .iter()
        .any(|media_file| media_file.file_path == file_path_string);

    if has_movie_collection && has_media_file {
        return Ok(false);
    }

    if let Err(error) =
        ensure_placeholder_media_file(&file_path, seeded_movie_size_bytes(&quality_tier))
    {
        tracing::warn!(
            title_id = %title.id,
            path = %file_path.display(),
            error = %error,
            "dev seed: failed to create placeholder movie file"
        );
    }

    let mut changed = false;

    if !has_media_file {
        let file_size_bytes = std::fs::metadata(&file_path)
            .map(|metadata| metadata.len() as i64)
            .unwrap_or_else(|_| seeded_movie_size_bytes(&quality_tier) as i64);
        app.services
            .media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: file_path_string.clone(),
                size_bytes: file_size_bytes,
                quality_label: Some(quality_tier.clone()),
                resolution: Some(quality_tier.clone()),
                ..Default::default()
            })
            .await
            .map_err(|e| {
                format!(
                    "dev seed: failed to insert media file for '{}': {e}",
                    title.name
                )
            })?;
        changed = true;
    }

    if !has_movie_collection {
        app.create_collection(
            actor,
            title.id.clone(),
            "movie".to_string(),
            DEV_SEED_MOVIE_COLLECTION_INDEX.to_string(),
            Some(quality_tier.clone()),
            Some(file_path_string.clone()),
            None,
            None,
        )
        .await
        .map_err(|e| {
            format!(
                "dev seed: failed to create movie collection for '{}': {e}",
                title.name
            )
        })?;
        changed = true;
    }

    tracing::debug!(
        title_id = %title.id,
        quality_tier = %quality_tier,
        path = %file_path.display(),
        changed = changed,
        "dev seed: ensured synthetic movie import"
    );

    Ok(changed)
}

pub(crate) async fn apply_dev_seed(app: &AppUseCase, db: &SqliteServices) -> Result<(), String> {
    let seed_path = match std::env::var("SCRYER_DEV_SEED_FILE") {
        Ok(path) if !path.trim().is_empty() => path.trim().to_string(),
        _ => return Ok(()),
    };

    let contents = std::fs::read_to_string(&seed_path)
        .map_err(|e| format!("failed to read seed file {seed_path}: {e}"))?;

    let config: SeedConfig = toml::from_str(&contents)
        .map_err(|e| format!("failed to parse seed file {seed_path}: {e}"))?;

    let actor = app
        .find_or_create_default_user()
        .await
        .map_err(|e| format!("failed to get admin user for seeding: {e}"))?;

    // Admin password
    if let Some(admin) = &config.admin
        && !admin.password.is_empty()
    {
        match app
            .bootstrap_user_password(&actor.id, &admin.password)
            .await
        {
            Ok(_) => tracing::info!("dev seed: admin password set"),
            Err(e) => tracing::warn!(error = %e, "dev seed: failed to set admin password"),
        }
    }

    // Indexers
    for seed_indexer in &config.indexers {
        let provider_type = seed_indexer.provider_type.to_lowercase();
        let normalized_base = normalize_base_url(&seed_indexer.base_url);

        let existing = app
            .list_indexer_configs(&actor, Some(provider_type.clone()))
            .await
            .map_err(|e| format!("dev seed: failed to list indexer configs: {e}"))?;

        let existing_config = existing.iter().find(|c| {
            c.provider_type.eq_ignore_ascii_case(&provider_type)
                && normalize_base_url(&c.base_url) == normalized_base
        });

        if let Some(existing_config) = existing_config {
            app.update_indexer_config(
                &actor,
                &existing_config.id,
                Some(seed_indexer.name.clone()),
                None,
                Some(seed_indexer.base_url.trim_end_matches('/').to_string()),
                seed_indexer.api_key.clone(),
                seed_indexer.rate_limit_seconds,
                seed_indexer.rate_limit_burst,
                Some(seed_indexer.enabled),
                None,
                None,
                None,
            )
            .await
            .map_err(|e| {
                format!(
                    "dev seed: failed to update indexer '{}': {e}",
                    seed_indexer.name
                )
            })?;

            tracing::info!(
                name = %seed_indexer.name,
                provider = %provider_type,
                "dev seed: updated existing indexer"
            );
            continue;
        }

        let input = NewIndexerConfig {
            name: seed_indexer.name.clone(),
            provider_type,
            base_url: seed_indexer.base_url.trim_end_matches('/').to_string(),
            api_key_encrypted: seed_indexer.api_key.clone(),
            rate_limit_seconds: seed_indexer.rate_limit_seconds,
            rate_limit_burst: seed_indexer.rate_limit_burst,
            is_enabled: seed_indexer.enabled,
            enable_interactive_search: true,
            enable_auto_search: true,
            config_json: None,
        };

        app.create_indexer_config(&actor, input)
            .await
            .map_err(|e| {
                format!(
                    "dev seed: failed to create indexer '{}': {e}",
                    seed_indexer.name
                )
            })?;

        tracing::info!(name = %seed_indexer.name, "dev seed: created indexer");
    }

    // Download clients
    for seed_client in &config.download_clients {
        let existing = app
            .list_download_client_configs(&actor, None)
            .await
            .map_err(|e| format!("dev seed: failed to list download clients: {e}"))?;

        let already_exists = existing
            .iter()
            .any(|c| c.name.eq_ignore_ascii_case(&seed_client.name));

        if already_exists {
            tracing::info!(
                name = %seed_client.name,
                "dev seed: download client already exists, skipping"
            );
            continue;
        }

        let config_json = toml_value_to_json_string(&seed_client.config);

        let input = NewDownloadClientConfig {
            name: seed_client.name.clone(),
            client_type: seed_client.client_type.clone(),
            base_url: seed_client.base_url.clone(),
            config_json,
            client_priority: 0, // auto-calculated by create_download_client_config
            is_enabled: seed_client.enabled,
        };

        app.create_download_client_config(&actor, input)
            .await
            .map_err(|e| {
                format!(
                    "dev seed: failed to create download client '{}': {e}",
                    seed_client.name
                )
            })?;

        tracing::info!(name = %seed_client.name, "dev seed: created download client");
    }

    // Settings
    for seed_setting in &config.settings {
        let value_json = toml_value_to_json_string(&seed_setting.value);

        db.upsert_setting_value(
            &seed_setting.scope,
            &seed_setting.key,
            seed_setting.scope_id.clone(),
            value_json,
            "dev-seed",
            None,
        )
        .await
        .map_err(|e| {
            format!(
                "dev seed: failed to upsert setting {}.{}: {e}",
                seed_setting.scope, seed_setting.key
            )
        })?;

        tracing::info!(
            scope = %seed_setting.scope,
            key = %seed_setting.key,
            scope_id = ?seed_setting.scope_id,
            "dev seed: upserted setting"
        );
    }

    tracing::info!(seed_file = %seed_path, "dev seed: completed");

    // Title seeding (separate file, separate env var)
    if let Err(e) = apply_title_seed(app, &actor, &config.settings).await {
        tracing::warn!(error = %e, "dev seed: title seeding failed");
    }

    Ok(())
}

async fn apply_title_seed(
    app: &AppUseCase,
    actor: &scryer_domain::User,
    settings: &[SeedSetting],
) -> Result<(), String> {
    let seed_path = match std::env::var("SCRYER_DEV_SEED_TITLES_FILE") {
        Ok(path) if !path.trim().is_empty() => path.trim().to_string(),
        _ => return Ok(()),
    };

    let contents = std::fs::read_to_string(&seed_path)
        .map_err(|e| format!("failed to read title seed file {seed_path}: {e}"))?;

    let config: SeedTitlesConfig = toml::from_str(&contents)
        .map_err(|e| format!("failed to parse title seed file {seed_path}: {e}"))?;

    let total = config.movies.len() + config.series.len() + config.anime.len();
    if total == 0 {
        return Ok(());
    }

    tracing::info!(
        movies = config.movies.len(),
        series = config.series.len(),
        anime = config.anime.len(),
        "dev seed: seeding titles"
    );

    // Collect existing TVDB IDs to skip duplicates
    let existing_titles = app
        .list_titles(actor, None, None)
        .await
        .map_err(|e| format!("dev seed: failed to list titles: {e}"))?;

    let existing_titles_by_tvdb: HashMap<i64, Title> = existing_titles
        .iter()
        .filter_map(|t| {
            t.external_ids
                .iter()
                .find(|eid| eid.source == "tvdb")
                .and_then(|eid| eid.value.parse::<i64>().ok())
                .map(|tvdb_id| (tvdb_id, t.clone()))
        })
        .collect();

    let mut created = 0u32;
    let mut skipped = 0u32;
    let mut imported_movies = 0u32;
    let movie_media_root =
        read_seed_media_root(settings, MOVIES_PATH_KEY, DEFAULT_MOVIE_MEDIA_ROOT);

    let sections: Vec<(MediaFacet, &[SeedTitle])> = vec![
        (MediaFacet::Movie, &config.movies),
        (MediaFacet::Tv, &config.series),
        (MediaFacet::Anime, &config.anime),
    ];

    for (facet, titles) in sections {
        for seed in titles {
            if let Some(existing_title) = existing_titles_by_tvdb.get(&seed.tvdb_id) {
                skipped += 1;
                if facet == MediaFacet::Movie {
                    match ensure_seeded_movie_import(
                        app,
                        actor,
                        existing_title,
                        seed,
                        &movie_media_root,
                    )
                    .await
                    {
                        Ok(true) => imported_movies += 1,
                        Ok(false) => {}
                        Err(error) => tracing::warn!(
                            tvdb_id = seed.tvdb_id,
                            name = %existing_title.name,
                            error = %error,
                            "dev seed: failed to backfill synthetic movie import"
                        ),
                    }
                }
                continue;
            }

            let label = seed.name.as_deref().unwrap_or("unknown");

            let request = NewTitle {
                name: label.to_string(),
                facet: facet.clone(),
                monitored: false,
                tags: vec!["dev-seed".to_string()],
                external_ids: vec![ExternalId {
                    source: "tvdb".to_string(),
                    value: seed.tvdb_id.to_string(),
                }],
                min_availability: None,
                poster_url: None,
                year: None,
                overview: None,
                sort_title: None,
                slug: None,
                runtime_minutes: None,
                language: None,
                content_status: None,
            };

            match app.add_title(actor, request).await {
                Ok(title) => {
                    created += 1;
                    tracing::debug!(
                        tvdb_id = seed.tvdb_id,
                        name = %title.name,
                        facet = ?facet,
                        "dev seed: created title"
                    );
                    if facet == MediaFacet::Movie {
                        match ensure_seeded_movie_import(
                            app,
                            actor,
                            &title,
                            seed,
                            &movie_media_root,
                        )
                        .await
                        {
                            Ok(true) => imported_movies += 1,
                            Ok(false) => {}
                            Err(error) => tracing::warn!(
                                tvdb_id = seed.tvdb_id,
                                name = %title.name,
                                error = %error,
                                "dev seed: failed to create synthetic movie import"
                            ),
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        tvdb_id = seed.tvdb_id,
                        name = label,
                        error = %e,
                        "dev seed: failed to create title"
                    );
                }
            }

            tokio::task::yield_now().await;
        }
    }

    tracing::info!(
        created = created,
        skipped = skipped,
        imported_movies = imported_movies,
        "dev seed: title seeding completed"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_example_seed_file() {
        let contents = include_str!("../../../dev-seed.example.toml");
        let config: SeedConfig = toml::from_str(contents).expect("example seed file should parse");
        assert!(config.admin.is_some());
        assert_eq!(config.admin.unwrap().password, "admin");
        assert_eq!(config.indexers.len(), 1);
        assert_eq!(config.indexers[0].provider_type, "nzbgeek");
        assert_eq!(config.download_clients.len(), 1);
        assert_eq!(config.download_clients[0].client_type, "nzbget");
        assert!(config.settings.len() >= 2);
    }

    #[test]
    fn parse_nested_table_settings() {
        let toml_str = r#"
[[settings]]
scope = "media"
key = "movies.path"
value = "/media/movies"

[[settings]]
scope = "system"
key = "download_client.routing"
scope_id = "movie"

[settings.value.home-nzbget]
category = "Movies"
recentQueuePriority = "normal"
"#;
        let config: SeedConfig = toml::from_str(toml_str).expect("nested tables should parse");
        assert_eq!(config.settings.len(), 2);

        // First: simple string value
        let json0 = toml_value_to_json_string(&config.settings[0].value);
        assert_eq!(json0, r#""/media/movies""#);

        // Second: nested table value
        let json1 = toml_value_to_json_string(&config.settings[1].value);
        let parsed: serde_json::Value = serde_json::from_str(&json1).unwrap();
        assert_eq!(parsed["home-nzbget"]["category"], "Movies");
    }

    #[test]
    fn parse_title_seed_file() {
        let toml_str = r#"
[[movies]]
tvdb_id = 12345
name = "The Dark Knight"

[[movies]]
tvdb_id = 67890

[[series]]
tvdb_id = 81189
name = "Breaking Bad"

[[anime]]
tvdb_id = 267440
name = "Attack on Titan"
"#;
        let config: SeedTitlesConfig = toml::from_str(toml_str).expect("title seed should parse");
        assert_eq!(config.movies.len(), 2);
        assert_eq!(config.series.len(), 1);
        assert_eq!(config.anime.len(), 1);
        assert_eq!(config.movies[0].tvdb_id, 12345);
        assert_eq!(config.movies[0].name.as_deref(), Some("The Dark Knight"));
        assert!(config.movies[0].quality_tier.is_none());
        assert_eq!(config.movies[1].tvdb_id, 67890);
        assert!(config.movies[1].name.is_none());
        assert_eq!(config.anime[0].tvdb_id, 267440);
    }

    #[test]
    fn normalize_seed_quality_uses_explicit_or_deterministic_default() {
        assert_eq!(normalize_seed_quality_tier(10, Some("1080p")), "1080P");
        assert_eq!(normalize_seed_quality_tier(9, None), "2160P");
        assert_eq!(normalize_seed_quality_tier(10, None), "1080P");
        assert_eq!(normalize_seed_quality_tier(11, None), "720P");
    }

    #[test]
    fn build_seeded_movie_path_sanitizes_title() {
        let path = build_seeded_movie_path(
            Path::new("/media/movies"),
            "Spider-Man: No Way/Home",
            "2160P",
        );
        assert_eq!(
            path,
            PathBuf::from("/media/movies/Spider-Man No Way Home/Spider-Man.No.Way.Home.2160P.mkv")
        );
    }

    #[test]
    fn toml_value_converts_to_json() {
        let toml_str = r#"
value = { host = "127.0.0.1", port = "6789", use_ssl = false, tags = [] }
"#;
        let table: toml::Value = toml::from_str(toml_str).unwrap();
        let value = table.get("value").unwrap();
        let json = toml_value_to_json_string(value);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["host"], "127.0.0.1");
        assert_eq!(parsed["use_ssl"], false);
        assert!(parsed["tags"].is_array());
    }
}
