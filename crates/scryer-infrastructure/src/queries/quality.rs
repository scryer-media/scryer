use std::collections::HashSet;

use chrono::Utc;
use scryer_application::{AppError, AppResult, QualityProfile, QualityProfileCriteria, ScoringConfig};
use sqlx::{Row, Sqlite, SqlitePool, Transaction};

pub(crate) async fn list_quality_profiles_query(
    pool: &SqlitePool,
    scope: &str,
    scope_id: Option<String>,
) -> AppResult<Vec<QualityProfile>> {
    let scope = scope.trim().to_string();
    if scope.is_empty() {
        return Err(AppError::Validation(
            "scope is required to list quality profiles".into(),
        ));
    }

    let normalized_scope_id = scope_id
        .and_then(|value| {
            let value = value.trim().to_string();
            if value.is_empty() {
                None
            } else {
                Some(value)
            }
        });

    let statement = if normalized_scope_id.is_some() {
        "SELECT id, name, scope, scope_id, archival_quality,
                allow_unknown_quality, atmos_preferred, dolby_vision_allowed,
                detected_hdr_allowed, prefer_remux, allow_bd_disk, allow_upgrades,
                prefer_dual_audio, required_audio_languages, scoring_config
           FROM quality_profiles
          WHERE scope = ?
            AND scope_id = ?
          ORDER BY name"
    } else {
        "SELECT id, name, scope, scope_id, archival_quality,
                allow_unknown_quality, atmos_preferred, dolby_vision_allowed,
                detected_hdr_allowed, prefer_remux, allow_bd_disk, allow_upgrades,
                prefer_dual_audio, required_audio_languages, scoring_config
           FROM quality_profiles
          WHERE scope = ?
            AND scope_id IS NULL
          ORDER BY name"
    };

    let mut query = sqlx::query(statement).bind(&scope);
    if let Some(scope_id) = normalized_scope_id.as_ref() {
        query = query.bind(scope_id);
    }

    let rows = query
        .fetch_all(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let id: String = row
            .try_get("id")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let name: String = row
            .try_get("name")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let archival_quality: Option<String> = row
            .try_get("archival_quality")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let allow_unknown_quality: i64 = row
            .try_get("allow_unknown_quality")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let atmos_preferred: i64 = row
            .try_get("atmos_preferred")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let dolby_vision_allowed: i64 = row
            .try_get("dolby_vision_allowed")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let detected_hdr_allowed: i64 = row
            .try_get("detected_hdr_allowed")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let prefer_remux: i64 = row
            .try_get("prefer_remux")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let allow_bd_disk: i64 = row
            .try_get("allow_bd_disk")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let allow_upgrades: i64 = row
            .try_get("allow_upgrades")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let prefer_dual_audio: i64 = row
            .try_get("prefer_dual_audio")
            .unwrap_or(0);
        let required_audio_languages_json: String = row
            .try_get("required_audio_languages")
            .unwrap_or_else(|_| "[]".to_string());
        let required_audio_languages: Vec<String> =
            serde_json::from_str(&required_audio_languages_json).unwrap_or_default();
        let scoring_config_json: String = row
            .try_get("scoring_config")
            .unwrap_or_else(|_| "{}".to_string());
        let scoring_config: ScoringConfig =
            serde_json::from_str(&scoring_config_json).unwrap_or_default();

        let quality_tiers = list_quality_profile_quality_tiers_query(pool, &id).await?;
        let source_allowlist = list_quality_profile_source_allowlist_query(pool, &id).await?;
        let source_blocklist = list_quality_profile_source_blocklist_query(pool, &id).await?;
        let video_codec_allowlist =
            list_quality_profile_video_codec_allowlist_query(pool, &id).await?;
        let video_codec_blocklist =
            list_quality_profile_video_codec_blocklist_query(pool, &id).await?;
        let audio_codec_allowlist =
            list_quality_profile_audio_codec_allowlist_query(pool, &id).await?;
        let audio_codec_blocklist =
            list_quality_profile_audio_codec_blocklist_query(pool, &id).await?;

        let profile = QualityProfile {
            id,
            name,
            criteria: QualityProfileCriteria {
                quality_tiers,
                archival_quality: archival_quality.and_then(|value| {
                    let value = value.trim().to_string();
                    if value.is_empty() {
                        None
                    } else {
                        Some(value)
                    }
                }),
                allow_unknown_quality: allow_unknown_quality != 0,
                source_allowlist,
                source_blocklist,
                video_codec_allowlist,
                video_codec_blocklist,
                audio_codec_allowlist,
                audio_codec_blocklist,
                atmos_preferred: atmos_preferred != 0,
                dolby_vision_allowed: dolby_vision_allowed != 0,
                detected_hdr_allowed: detected_hdr_allowed != 0,
                prefer_remux: prefer_remux != 0,
                allow_bd_disk: allow_bd_disk != 0,
                allow_upgrades: allow_upgrades != 0,
                prefer_dual_audio: prefer_dual_audio != 0,
                required_audio_languages,
                scoring_persona: scoring_config.scoring_persona,
                scoring_overrides: scoring_config.scoring_overrides,
                cutoff_tier: scoring_config.cutoff_tier,
                min_score_to_grab: scoring_config.min_score_to_grab,
                facet_persona_overrides: scoring_config.facet_persona_overrides,
            },
        };
        out.push(profile);
    }

    Ok(out)
}

pub(crate) async fn delete_quality_profile_query(
    pool: &SqlitePool,
    profile_id: &str,
) -> AppResult<()> {
    let profile_id = profile_id.trim().to_string();
    if profile_id.is_empty() {
        return Err(AppError::Validation(
            "profile_id is required to delete a quality profile".into(),
        ));
    }

    let mut tx = pool
        .begin()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    clear_quality_profile_value_rows(&mut tx, &profile_id).await?;

    sqlx::query("DELETE FROM quality_profiles WHERE id = ?")
        .bind(&profile_id)
        .execute(&mut *tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    tx.commit()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))
}

pub(crate) async fn replace_quality_profiles_query(
    pool: &SqlitePool,
    scope: &str,
    scope_id: Option<String>,
    profiles: Vec<QualityProfile>,
) -> AppResult<()> {
    let scope = scope.trim().to_string();
    if scope.is_empty() {
        return Err(AppError::Validation(
            "scope is required to replace quality profiles".into(),
        ));
    }

    let normalized_scope_id = scope_id
        .and_then(|value| {
            let value = value.trim().to_string();
            if value.is_empty() {
                None
            } else {
                Some(value)
            }
        });

    let now = Utc::now().to_rfc3339();
    let mut tx = pool
        .begin()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    if let Some(scope_id) = normalized_scope_id.as_ref() {
        sqlx::query("DELETE FROM quality_profiles WHERE scope = ? AND scope_id = ?")
            .bind(&scope)
            .bind(scope_id)
            .execute(&mut *tx)
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
    } else {
        sqlx::query("DELETE FROM quality_profiles WHERE scope = ? AND scope_id IS NULL")
            .bind(&scope)
            .execute(&mut *tx)
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
    }

    for profile in profiles {
        upsert_quality_profile_query(
            &mut tx,
            scope.as_str(),
            normalized_scope_id.as_ref(),
            &now,
            profile,
        )
        .await?;
    }

    tx.commit()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))
}

pub(crate) async fn upsert_quality_profiles_query(
    pool: &SqlitePool,
    scope: &str,
    scope_id: Option<String>,
    profiles: Vec<QualityProfile>,
) -> AppResult<()> {
    let scope = scope.trim().to_string();
    if scope.is_empty() {
        return Err(AppError::Validation(
            "scope is required to upsert quality profiles".into(),
        ));
    }

    let normalized_scope_id = scope_id
        .and_then(|value| {
            let value = value.trim().to_string();
            if value.is_empty() {
                None
            } else {
                Some(value)
            }
        });

    let now = Utc::now().to_rfc3339();
    let mut tx = pool
        .begin()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    for profile in profiles {
        upsert_quality_profile_query(
            &mut tx,
            scope.as_str(),
            normalized_scope_id.as_ref(),
            &now,
            profile,
        )
        .await?;
    }

    tx.commit()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))
}

async fn upsert_quality_profile_query(
    tx: &mut Transaction<'_, Sqlite>,
    scope: &str,
    scope_id: Option<&String>,
    now: &str,
    profile: QualityProfile,
) -> AppResult<()> {
    let id = profile.id.trim().to_string();
    if id.is_empty() {
        return Ok(());
    }

    let name = profile.name.trim().to_string();
    let criteria = profile.criteria;
    let archival_quality = criteria
        .archival_quality
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let quality_tiers = normalize_profile_string_values(criteria.quality_tiers);
    let source_allowlist = normalize_profile_string_values(criteria.source_allowlist);
    let source_blocklist = normalize_profile_string_values(criteria.source_blocklist);
    let video_codec_allowlist = normalize_profile_string_values(criteria.video_codec_allowlist);
    let video_codec_blocklist = normalize_profile_string_values(criteria.video_codec_blocklist);
    let audio_codec_allowlist = normalize_profile_string_values(criteria.audio_codec_allowlist);
    let audio_codec_blocklist = normalize_profile_string_values(criteria.audio_codec_blocklist);

    clear_quality_profile_value_rows(tx, &id).await?;

    let required_audio_languages_json =
        serde_json::to_string(&criteria.required_audio_languages).unwrap_or_else(|_| "[]".to_string());

    let scoring_config = ScoringConfig {
        scoring_persona: criteria.scoring_persona.clone(),
        scoring_overrides: criteria.scoring_overrides.clone(),
        cutoff_tier: criteria.cutoff_tier.clone(),
        min_score_to_grab: criteria.min_score_to_grab,
        facet_persona_overrides: criteria.facet_persona_overrides.clone(),
    };
    let scoring_config_json =
        serde_json::to_string(&scoring_config).unwrap_or_else(|_| "{}".to_string());

    sqlx::query(
        "INSERT INTO quality_profiles
            (id, name, scope, scope_id, archival_quality, allow_unknown_quality,
             atmos_preferred, dolby_vision_allowed, detected_hdr_allowed, prefer_remux,
             allow_bd_disk, allow_upgrades, prefer_dual_audio, required_audio_languages,
             scoring_config, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            scope = excluded.scope,
            scope_id = excluded.scope_id,
            archival_quality = excluded.archival_quality,
            allow_unknown_quality = excluded.allow_unknown_quality,
            atmos_preferred = excluded.atmos_preferred,
            dolby_vision_allowed = excluded.dolby_vision_allowed,
            detected_hdr_allowed = excluded.detected_hdr_allowed,
            prefer_remux = excluded.prefer_remux,
            allow_bd_disk = excluded.allow_bd_disk,
            allow_upgrades = excluded.allow_upgrades,
            prefer_dual_audio = excluded.prefer_dual_audio,
            required_audio_languages = excluded.required_audio_languages,
            scoring_config = excluded.scoring_config",
    )
    .bind(id.as_str())
    .bind(name)
    .bind(scope)
    .bind(scope_id)
    .bind(archival_quality)
    .bind(if criteria.allow_unknown_quality {
        1_i64
    } else {
        0_i64
    })
    .bind(if criteria.atmos_preferred {
        1_i64
    } else {
        0_i64
    })
    .bind(if criteria.dolby_vision_allowed {
        1_i64
    } else {
        0_i64
    })
    .bind(if criteria.detected_hdr_allowed {
        1_i64
    } else {
        0_i64
    })
    .bind(if criteria.prefer_remux { 1_i64 } else { 0_i64 })
    .bind(if criteria.allow_bd_disk { 1_i64 } else { 0_i64 })
    .bind(if criteria.allow_upgrades {
        1_i64
    } else {
        0_i64
    })
    .bind(if criteria.prefer_dual_audio {
        1_i64
    } else {
        0_i64
    })
    .bind(&required_audio_languages_json)
    .bind(&scoring_config_json)
    .bind(now)
    .execute(&mut **tx)
    .await
    .map_err(|error| AppError::Repository(error.to_string()))?;

    replace_quality_profile_quality_tiers_query(tx, &id, &quality_tiers).await?;
    replace_quality_profile_source_allowlist_query(tx, &id, &source_allowlist).await?;
    replace_quality_profile_source_blocklist_query(tx, &id, &source_blocklist).await?;
    replace_quality_profile_video_codec_allowlist_query(tx, &id, &video_codec_allowlist).await?;
    replace_quality_profile_video_codec_blocklist_query(tx, &id, &video_codec_blocklist).await?;
    replace_quality_profile_audio_codec_allowlist_query(tx, &id, &audio_codec_allowlist).await?;
    replace_quality_profile_audio_codec_blocklist_query(tx, &id, &audio_codec_blocklist).await?;

    Ok(())
}

fn normalize_profile_string_values(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::with_capacity(values.len());
    for value in values {
        let value = value.trim().to_string();
        if value.is_empty() {
            continue;
        }
        if seen.insert(value.clone()) {
            normalized.push(value);
        }
    }

    normalized
}

pub(crate) async fn clear_quality_profile_value_rows(
    tx: &mut Transaction<'_, Sqlite>,
    profile_id: &str,
) -> AppResult<()> {
    sqlx::query("DELETE FROM quality_profile_quality_tiers WHERE profile_id = ?")
        .bind(profile_id)
        .execute(&mut **tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    sqlx::query("DELETE FROM quality_profile_source_allowlist WHERE profile_id = ?")
        .bind(profile_id)
        .execute(&mut **tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    sqlx::query("DELETE FROM quality_profile_source_blocklist WHERE profile_id = ?")
        .bind(profile_id)
        .execute(&mut **tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    sqlx::query("DELETE FROM quality_profile_video_codec_allowlist WHERE profile_id = ?")
        .bind(profile_id)
        .execute(&mut **tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    sqlx::query("DELETE FROM quality_profile_video_codec_blocklist WHERE profile_id = ?")
        .bind(profile_id)
        .execute(&mut **tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    sqlx::query("DELETE FROM quality_profile_audio_codec_allowlist WHERE profile_id = ?")
        .bind(profile_id)
        .execute(&mut **tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    sqlx::query("DELETE FROM quality_profile_audio_codec_blocklist WHERE profile_id = ?")
        .bind(profile_id)
        .execute(&mut **tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}

pub(crate) async fn replace_quality_profile_quality_tiers_query(
    tx: &mut Transaction<'_, Sqlite>,
    profile_id: &str,
    values: &[String],
) -> AppResult<()> {
    for (index, value) in values.iter().enumerate() {
        sqlx::query(
            "INSERT INTO quality_profile_quality_tiers(profile_id, quality_tier, sort_order)
             VALUES (?, ?, ?)",
        )
        .bind(profile_id)
        .bind(value)
        .bind(index as i64)
        .execute(&mut **tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    }

    Ok(())
}

pub(crate) async fn replace_quality_profile_source_allowlist_query(
    tx: &mut Transaction<'_, Sqlite>,
    profile_id: &str,
    values: &[String],
) -> AppResult<()> {
    for value in values {
        sqlx::query(
            "INSERT INTO quality_profile_source_allowlist(profile_id, source)
             VALUES (?, ?)",
        )
        .bind(profile_id)
        .bind(value)
        .execute(&mut **tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    }

    Ok(())
}

pub(crate) async fn replace_quality_profile_source_blocklist_query(
    tx: &mut Transaction<'_, Sqlite>,
    profile_id: &str,
    values: &[String],
) -> AppResult<()> {
    for value in values {
        sqlx::query(
            "INSERT INTO quality_profile_source_blocklist(profile_id, source)
             VALUES (?, ?)",
        )
        .bind(profile_id)
        .bind(value)
        .execute(&mut **tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    }

    Ok(())
}

pub(crate) async fn replace_quality_profile_video_codec_allowlist_query(
    tx: &mut Transaction<'_, Sqlite>,
    profile_id: &str,
    values: &[String],
) -> AppResult<()> {
    for value in values {
        sqlx::query(
            "INSERT INTO quality_profile_video_codec_allowlist(profile_id, codec)
             VALUES (?, ?)",
        )
        .bind(profile_id)
        .bind(value)
        .execute(&mut **tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    }

    Ok(())
}

pub(crate) async fn replace_quality_profile_video_codec_blocklist_query(
    tx: &mut Transaction<'_, Sqlite>,
    profile_id: &str,
    values: &[String],
) -> AppResult<()> {
    for value in values {
        sqlx::query(
            "INSERT INTO quality_profile_video_codec_blocklist(profile_id, codec)
             VALUES (?, ?)",
        )
        .bind(profile_id)
        .bind(value)
        .execute(&mut **tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    }

    Ok(())
}

pub(crate) async fn replace_quality_profile_audio_codec_allowlist_query(
    tx: &mut Transaction<'_, Sqlite>,
    profile_id: &str,
    values: &[String],
) -> AppResult<()> {
    for value in values {
        sqlx::query(
            "INSERT INTO quality_profile_audio_codec_allowlist(profile_id, codec)
             VALUES (?, ?)",
        )
        .bind(profile_id)
        .bind(value)
        .execute(&mut **tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    }

    Ok(())
}

pub(crate) async fn replace_quality_profile_audio_codec_blocklist_query(
    tx: &mut Transaction<'_, Sqlite>,
    profile_id: &str,
    values: &[String],
) -> AppResult<()> {
    for value in values {
        sqlx::query(
            "INSERT INTO quality_profile_audio_codec_blocklist(profile_id, codec)
             VALUES (?, ?)",
        )
        .bind(profile_id)
        .bind(value)
        .execute(&mut **tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    }

    Ok(())
}

pub(crate) async fn list_quality_profile_quality_tiers_query(
    pool: &SqlitePool,
    profile_id: &str,
) -> AppResult<Vec<String>> {
    let rows = sqlx::query(
        "SELECT quality_tier
           FROM quality_profile_quality_tiers
          WHERE profile_id = ?
          ORDER BY sort_order ASC",
    )
    .bind(profile_id)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut values = Vec::with_capacity(rows.len());
    for row in rows {
        let value: String = row
            .try_get("quality_tier")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        values.push(value);
    }

    Ok(values)
}

pub(crate) async fn list_quality_profile_source_allowlist_query(
    pool: &SqlitePool,
    profile_id: &str,
) -> AppResult<Vec<String>> {
    let rows = sqlx::query(
        "SELECT source
           FROM quality_profile_source_allowlist
          WHERE profile_id = ?
          ORDER BY source ASC",
    )
    .bind(profile_id)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut values = Vec::with_capacity(rows.len());
    for row in rows {
        let value: String = row
            .try_get("source")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        values.push(value);
    }

    Ok(values)
}

pub(crate) async fn list_quality_profile_source_blocklist_query(
    pool: &SqlitePool,
    profile_id: &str,
) -> AppResult<Vec<String>> {
    let rows = sqlx::query(
        "SELECT source
           FROM quality_profile_source_blocklist
          WHERE profile_id = ?
          ORDER BY source ASC",
    )
    .bind(profile_id)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut values = Vec::with_capacity(rows.len());
    for row in rows {
        let value: String = row
            .try_get("source")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        values.push(value);
    }

    Ok(values)
}

pub(crate) async fn list_quality_profile_video_codec_allowlist_query(
    pool: &SqlitePool,
    profile_id: &str,
) -> AppResult<Vec<String>> {
    let rows = sqlx::query(
        "SELECT codec
           FROM quality_profile_video_codec_allowlist
          WHERE profile_id = ?
          ORDER BY codec ASC",
    )
    .bind(profile_id)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut values = Vec::with_capacity(rows.len());
    for row in rows {
        let value: String = row
            .try_get("codec")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        values.push(value);
    }

    Ok(values)
}

pub(crate) async fn list_quality_profile_video_codec_blocklist_query(
    pool: &SqlitePool,
    profile_id: &str,
) -> AppResult<Vec<String>> {
    let rows = sqlx::query(
        "SELECT codec
           FROM quality_profile_video_codec_blocklist
          WHERE profile_id = ?
          ORDER BY codec ASC",
    )
    .bind(profile_id)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut values = Vec::with_capacity(rows.len());
    for row in rows {
        let value: String = row
            .try_get("codec")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        values.push(value);
    }

    Ok(values)
}

pub(crate) async fn list_quality_profile_audio_codec_allowlist_query(
    pool: &SqlitePool,
    profile_id: &str,
) -> AppResult<Vec<String>> {
    let rows = sqlx::query(
        "SELECT codec
           FROM quality_profile_audio_codec_allowlist
          WHERE profile_id = ?
          ORDER BY codec ASC",
    )
    .bind(profile_id)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut values = Vec::with_capacity(rows.len());
    for row in rows {
        let value: String = row
            .try_get("codec")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        values.push(value);
    }

    Ok(values)
}

pub(crate) async fn list_quality_profile_audio_codec_blocklist_query(
    pool: &SqlitePool,
    profile_id: &str,
) -> AppResult<Vec<String>> {
    let rows = sqlx::query(
        "SELECT codec
           FROM quality_profile_audio_codec_blocklist
          WHERE profile_id = ?
          ORDER BY codec ASC",
    )
    .bind(profile_id)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut values = Vec::with_capacity(rows.len());
    for row in rows {
        let value: String = row
            .try_get("codec")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        values.push(value);
    }

    Ok(values)
}
