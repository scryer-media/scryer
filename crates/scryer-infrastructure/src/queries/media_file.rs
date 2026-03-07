use chrono::Utc;
use scryer_application::{AppError, AppResult, MediaFileAnalysis, TitleMediaFile};
use scryer_domain::Id;
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};

pub(crate) async fn insert_media_file_query(
    pool: &SqlitePool,
    title_id: &str,
    file_path: &str,
    size_bytes: i64,
    quality_label: Option<String>,
) -> AppResult<String> {
    let id = Id::new().0;
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO media_files
         (id, title_id, file_path, size_bytes, quality_id, scan_status, created_at)
         VALUES (?, ?, ?, ?, ?, 'imported', ?)
         ON CONFLICT(file_path) DO UPDATE SET
            title_id = excluded.title_id,
            size_bytes = excluded.size_bytes,
            quality_id = excluded.quality_id,
            scan_status = excluded.scan_status",
    )
    .bind(&id)
    .bind(title_id)
    .bind(file_path)
    .bind(size_bytes)
    .bind(&quality_label)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(id)
}

pub(crate) async fn link_file_to_episode_query(
    pool: &SqlitePool,
    file_id: &str,
    episode_id: &str,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO file_episode_map (file_id, episode_id)
         VALUES (?, ?)
         ON CONFLICT(file_id, episode_id) DO NOTHING",
    )
    .bind(file_id)
    .bind(episode_id)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}

pub(crate) async fn list_media_files_for_title_query(
    pool: &SqlitePool,
    title_id: &str,
) -> AppResult<Vec<TitleMediaFile>> {
    let rows: Vec<SqliteRow> = sqlx::query(
        "SELECT mf.id, mf.title_id, fem.episode_id, mf.file_path,
                mf.size_bytes, mf.quality_id, mf.scan_status, mf.created_at,
                mf.video_codec, mf.video_width, mf.video_height,
                mf.video_bitrate_kbps, mf.video_bit_depth,
                mf.video_hdr_format, mf.video_frame_rate, mf.video_profile,
                mf.audio_codec, mf.audio_channels, mf.audio_bitrate_kbps,
                mf.duration_seconds, mf.container_format,
                mf.audio_languages_json, mf.audio_streams_json,
                mf.subtitle_languages_json,
                mf.subtitle_codecs_json, mf.has_multiaudio
         FROM media_files mf
         LEFT JOIN file_episode_map fem ON fem.file_id = mf.id
         WHERE mf.title_id = ?
         ORDER BY mf.created_at DESC",
    )
    .bind(title_id)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        out.push(row_to_title_media_file(row)?);
    }
    Ok(out)
}

fn row_to_title_media_file(row: &SqliteRow) -> AppResult<TitleMediaFile> {
    let id: String = row
        .try_get("id")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let title_id: String = row
        .try_get("title_id")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let episode_id: Option<String> = row.try_get("episode_id").unwrap_or(None);
    let file_path: String = row
        .try_get("file_path")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let size_bytes: i64 = row
        .try_get("size_bytes")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let quality_label: Option<String> = row.try_get("quality_id").unwrap_or(None);
    let scan_status: String = row
        .try_get("scan_status")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let created_at: String = row
        .try_get("created_at")
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let video_codec: Option<String> = row.try_get("video_codec").unwrap_or(None);
    let video_width: Option<i32> = row.try_get("video_width").unwrap_or(None);
    let video_height: Option<i32> = row.try_get("video_height").unwrap_or(None);
    let video_bitrate_kbps: Option<i32> = row.try_get("video_bitrate_kbps").unwrap_or(None);
    let video_bit_depth: Option<i32> = row.try_get("video_bit_depth").unwrap_or(None);
    let video_hdr_format: Option<String> = row.try_get("video_hdr_format").unwrap_or(None);
    let video_frame_rate: Option<String> = row.try_get("video_frame_rate").unwrap_or(None);
    let video_profile: Option<String> = row.try_get("video_profile").unwrap_or(None);
    let audio_codec: Option<String> = row.try_get("audio_codec").unwrap_or(None);
    let audio_channels: Option<i32> = row.try_get("audio_channels").unwrap_or(None);
    let audio_bitrate_kbps: Option<i32> = row.try_get("audio_bitrate_kbps").unwrap_or(None);
    let duration_seconds: Option<i32> = row.try_get("duration_seconds").unwrap_or(None);
    let container_format: Option<String> = row.try_get("container_format").unwrap_or(None);
    let has_multiaudio: i64 = row.try_get("has_multiaudio").unwrap_or(0i64);

    let audio_languages: Vec<String> = row
        .try_get::<Option<String>, _>("audio_languages_json")
        .unwrap_or(None)
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default();
    let audio_streams: Vec<scryer_application::AudioStreamDetail> = row
        .try_get::<Option<String>, _>("audio_streams_json")
        .unwrap_or(None)
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default();
    let subtitle_languages: Vec<String> = row
        .try_get::<Option<String>, _>("subtitle_languages_json")
        .unwrap_or(None)
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default();
    let subtitle_codecs: Vec<String> = row
        .try_get::<Option<String>, _>("subtitle_codecs_json")
        .unwrap_or(None)
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default();

    Ok(TitleMediaFile {
        id,
        title_id,
        episode_id,
        file_path,
        size_bytes,
        quality_label,
        scan_status,
        created_at,
        video_codec,
        video_width,
        video_height,
        video_bitrate_kbps,
        video_bit_depth,
        video_hdr_format,
        video_frame_rate,
        video_profile,
        audio_codec,
        audio_channels,
        audio_bitrate_kbps,
        audio_languages,
        audio_streams,
        subtitle_languages,
        subtitle_codecs,
        has_multiaudio: has_multiaudio != 0,
        duration_seconds,
        container_format,
    })
}

pub(crate) async fn update_media_file_analysis_query(
    pool: &SqlitePool,
    file_id: &str,
    analysis: &MediaFileAnalysis,
) -> AppResult<()> {
    let audio_languages_json = serde_json::to_string(&analysis.audio_languages)
        .unwrap_or_else(|_| "[]".to_string());
    let audio_streams_json = serde_json::to_string(&analysis.audio_streams)
        .unwrap_or_else(|_| "[]".to_string());
    let subtitle_languages_json = serde_json::to_string(&analysis.subtitle_languages)
        .unwrap_or_else(|_| "[]".to_string());
    let subtitle_codecs_json = serde_json::to_string(&analysis.subtitle_codecs)
        .unwrap_or_else(|_| "[]".to_string());

    sqlx::query(
        "UPDATE media_files SET
            video_codec = ?,
            video_width = ?,
            video_height = ?,
            video_bitrate_kbps = ?,
            video_bit_depth = ?,
            video_hdr_format = ?,
            video_frame_rate = ?,
            video_profile = ?,
            audio_codec = ?,
            audio_channels = ?,
            audio_bitrate_kbps = ?,
            duration_seconds = ?,
            container_format = ?,
            ffprobe_json = ?,
            audio_languages_json = ?,
            audio_streams_json = ?,
            subtitle_languages_json = ?,
            subtitle_codecs_json = ?,
            has_multiaudio = ?,
            scan_status = 'scanned'
         WHERE id = ?",
    )
    .bind(&analysis.video_codec)
    .bind(analysis.video_width)
    .bind(analysis.video_height)
    .bind(analysis.video_bitrate_kbps)
    .bind(analysis.video_bit_depth)
    .bind(&analysis.video_hdr_format)
    .bind(&analysis.video_frame_rate)
    .bind(&analysis.video_profile)
    .bind(&analysis.audio_codec)
    .bind(analysis.audio_channels)
    .bind(analysis.audio_bitrate_kbps)
    .bind(analysis.duration_seconds)
    .bind(&analysis.container_format)
    .bind(&analysis.raw_json)
    .bind(&audio_languages_json)
    .bind(&audio_streams_json)
    .bind(&subtitle_languages_json)
    .bind(&subtitle_codecs_json)
    .bind(if analysis.has_multiaudio { 1i64 } else { 0i64 })
    .bind(file_id)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}

pub(crate) async fn mark_scan_failed_query(
    pool: &SqlitePool,
    file_id: &str,
    error: &str,
) -> AppResult<()> {
    sqlx::query(
        "UPDATE media_files SET scan_status = 'scan_failed', scan_error = ? WHERE id = ?",
    )
    .bind(error)
    .bind(file_id)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}

pub(crate) async fn delete_media_file_query(pool: &SqlitePool, file_id: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM media_files WHERE id = ?")
        .bind(file_id)
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}
