use std::collections::HashMap;
use std::io::Cursor;

use async_trait::async_trait;
use fast_image_resize as fir;
use image::codecs::avif::AvifEncoder;
use image::{DynamicImage, ImageEncoder, ImageFormat, RgbaImage};
use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE, ETAG, LAST_MODIFIED};
use ring::digest;
use scryer_application::{
    AppError, AppResult, TitleImageBlob, TitleImageKind, TitleImageProcessor,
    TitleImageReplacement, TitleImageRepository, TitleImageStorageMode, TitleImageSyncTask,
    TitleImageVariantRecord,
};
use scryer_domain::Title;
use sqlx::{Row, SqlitePool};
use tracing::warn;
use uuid::Uuid;

const MAX_SOURCE_BYTES: usize = 20 * 1024 * 1024;
const POSTER_VARIANT_WIDTHS: [u32; 3] = [500, 250, 70];
const TITLE_IMAGE_CONNECT_TIMEOUT_SECS: u64 = 5;
const TITLE_IMAGE_REQUEST_TIMEOUT_SECS: u64 = 20;
const AVIF_SPEED: u8 = if cfg!(debug_assertions) { 10 } else { 6 };
const AVIF_VARIANT_SPEED: u8 = 9;
const AVIF_QUALITY: u8 = if cfg!(debug_assertions) { 60 } else { 85 };

#[derive(Clone)]
pub struct SqliteTitleImageProcessor {
    client: reqwest::Client,
    max_source_bytes: usize,
    avif_enabled: bool,
}

impl SqliteTitleImageProcessor {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent(format!("scryer/{}", env!("CARGO_PKG_VERSION")))
            .connect_timeout(std::time::Duration::from_secs(
                TITLE_IMAGE_CONNECT_TIMEOUT_SECS,
            ))
            .timeout(std::time::Duration::from_secs(
                TITLE_IMAGE_REQUEST_TIMEOUT_SECS,
            ))
            .build()
            .expect("title image reqwest client should build");
        Self {
            client,
            max_source_bytes: MAX_SOURCE_BYTES,
            avif_enabled: true,
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_tests(avif_enabled: bool) -> Self {
        let client = reqwest::Client::builder()
            .user_agent("scryer-tests")
            .connect_timeout(std::time::Duration::from_secs(
                TITLE_IMAGE_CONNECT_TIMEOUT_SECS,
            ))
            .timeout(std::time::Duration::from_secs(
                TITLE_IMAGE_REQUEST_TIMEOUT_SECS,
            ))
            .build()
            .expect("title image test reqwest client should build");
        Self {
            client,
            max_source_bytes: MAX_SOURCE_BYTES,
            avif_enabled,
        }
    }

    async fn fetch_source(
        &self,
        source_url: &str,
    ) -> AppResult<(Vec<u8>, Option<String>, Option<String>)> {
        let response =
            self.client.get(source_url).send().await.map_err(|err| {
                AppError::Repository(format!("failed to fetch title image: {err}"))
            })?;

        if !response.status().is_success() {
            return Err(AppError::Repository(format!(
                "title image fetch failed with status {}",
                response.status()
            )));
        }

        if let Some(length) = response
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<usize>().ok())
        {
            if length > self.max_source_bytes {
                return Err(AppError::Validation(format!(
                    "title image exceeds max size of {} bytes",
                    self.max_source_bytes
                )));
            }
        }

        if let Some(content_type) = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
        {
            if !content_type.starts_with("image/") {
                return Err(AppError::Validation(format!(
                    "unsupported title image content type: {content_type}"
                )));
            }
        }

        let etag = response
            .headers()
            .get(ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let last_modified = response
            .headers()
            .get(LAST_MODIFIED)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);

        let bytes = response.bytes().await.map_err(|err| {
            AppError::Repository(format!("failed to read title image bytes: {err}"))
        })?;
        if bytes.len() > self.max_source_bytes {
            return Err(AppError::Validation(format!(
                "title image exceeds max size of {} bytes",
                self.max_source_bytes
            )));
        }

        Ok((bytes.to_vec(), etag, last_modified))
    }

    fn process_bytes(
        &self,
        kind: TitleImageKind,
        source_url: &str,
        bytes: &[u8],
        source_etag: Option<String>,
        source_last_modified: Option<String>,
    ) -> AppResult<TitleImageReplacement> {
        let guessed_format = image::guess_format(bytes)
            .map_err(|err| AppError::Validation(format!("failed to detect image format: {err}")))?;
        let source_format = SupportedImageFormat::from_image_format(guessed_format)
            .ok_or_else(|| AppError::Validation("unsupported image format".to_string()))?;
        let decoded = image::load_from_memory_with_format(bytes, guessed_format)
            .map_err(|err| AppError::Validation(format!("failed to decode image: {err}")))?;
        let oriented = apply_orientation(decoded, read_exif_orientation(bytes).unwrap_or(1));
        let rgba = oriented.to_rgba8();
        let (width, height) = rgba.dimensions();

        if width == 0 || height == 0 {
            return Err(AppError::Validation(
                "image dimensions must be non-zero".to_string(),
            ));
        }

        if self.avif_enabled {
            match encode_avif(&rgba, AVIF_SPEED, AVIF_QUALITY) {
                Ok(master_bytes) => {
                    let master_sha256 = sha256_hex(&master_bytes);
                    let variants = build_image_variants(kind, &rgba, &master_bytes)?;
                    return Ok(TitleImageReplacement {
                        kind,
                        source_url: source_url.to_string(),
                        source_etag,
                        source_last_modified,
                        source_format: source_format.as_str().to_string(),
                        source_width: width as i32,
                        source_height: height as i32,
                        storage_mode: TitleImageStorageMode::AvifMaster,
                        master_format: SupportedImageFormat::Avif.as_str().to_string(),
                        master_sha256,
                        master_width: width as i32,
                        master_height: height as i32,
                        master_bytes,
                        variants,
                    });
                }
                Err(error) => {
                    warn!(error = %error, source_url = %source_url, "title image AVIF encode failed; falling back to original bytes");
                }
            }
        }

        Ok(TitleImageReplacement {
            kind,
            source_url: source_url.to_string(),
            source_etag,
            source_last_modified,
            source_format: source_format.as_str().to_string(),
            source_width: width as i32,
            source_height: height as i32,
            storage_mode: TitleImageStorageMode::Original,
            master_format: source_format.as_str().to_string(),
            master_sha256: sha256_hex(bytes),
            master_width: width as i32,
            master_height: height as i32,
            master_bytes: bytes.to_vec(),
            variants: Vec::new(),
        })
    }

    #[cfg(test)]
    pub(crate) fn process_bytes_for_tests(
        &self,
        kind: TitleImageKind,
        source_url: &str,
        bytes: &[u8],
    ) -> AppResult<TitleImageReplacement> {
        self.process_bytes(kind, source_url, bytes, None, None)
    }
}

impl Default for SqliteTitleImageProcessor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl TitleImageProcessor for SqliteTitleImageProcessor {
    async fn fetch_and_process_image(
        &self,
        kind: TitleImageKind,
        source_url: &str,
    ) -> AppResult<TitleImageReplacement> {
        let (bytes, etag, last_modified) = self.fetch_source(source_url).await?;
        let this = self.clone();
        let source_url = source_url.to_string();
        tokio::task::spawn_blocking(move || {
            this.process_bytes(kind, &source_url, &bytes, etag, last_modified)
        })
        .await
        .map_err(|err| AppError::Repository(format!("image encode task failed: {err}")))?
    }
}

#[async_trait]
impl TitleImageRepository for crate::sqlite_services::SqliteServices {
    async fn list_titles_requiring_image_refresh(
        &self,
        kind: TitleImageKind,
        limit: usize,
    ) -> AppResult<Vec<TitleImageSyncTask>> {
        list_titles_requiring_image_refresh_query(&self.pool, kind, limit).await
    }

    async fn replace_title_image(
        &self,
        title_id: &str,
        replacement: TitleImageReplacement,
    ) -> AppResult<()> {
        replace_title_image_query(&self.pool, title_id, replacement).await
    }

    async fn get_title_image_blob(
        &self,
        title_id: &str,
        kind: TitleImageKind,
        variant_key: &str,
    ) -> AppResult<Option<TitleImageBlob>> {
        get_title_image_blob_query(&self.pool, title_id, kind, variant_key).await
    }
}

pub(crate) async fn apply_local_poster_urls(
    pool: &SqlitePool,
    titles: &mut [Title],
) -> AppResult<()> {
    apply_local_image_urls(pool, TitleImageKind::Poster, "w500", titles).await
}

pub(crate) async fn apply_local_image_urls(
    pool: &SqlitePool,
    kind: TitleImageKind,
    preferred_variant: &str,
    titles: &mut [Title],
) -> AppResult<()> {
    if titles.is_empty() {
        return Ok(());
    }

    let title_ids = titles
        .iter()
        .map(|title| title.id.clone())
        .collect::<Vec<_>>();
    let local_urls = load_local_image_url_map(pool, kind, preferred_variant, &title_ids).await?;
    for title in titles {
        if let Some(url) = local_urls.get(&title.id) {
            match kind {
                TitleImageKind::Poster => title.poster_url = Some(url.clone()),
                TitleImageKind::Banner => title.banner_url = Some(url.clone()),
                TitleImageKind::Fanart => title.background_url = Some(url.clone()),
            }
        }
    }

    Ok(())
}

pub(crate) async fn list_titles_requiring_image_refresh_query(
    pool: &SqlitePool,
    kind: TitleImageKind,
    limit: usize,
) -> AppResult<Vec<TitleImageSyncTask>> {
    let (source_col, preferred_variant) = match kind {
        TitleImageKind::Poster => ("poster_url", "w500"),
        TitleImageKind::Banner => ("banner_url", "master"),
        TitleImageKind::Fanart => ("background_url", "master"),
    };

    let sql = format!(
        "SELECT t.id AS title_id, t.{source_col} AS source_url, ti.source_url AS cached_source_url
         FROM titles t
         LEFT JOIN title_images ti
           ON ti.title_id = t.id
          AND ti.kind = ?
         LEFT JOIN title_image_variants pv
           ON pv.title_image_id = ti.id
          AND pv.variant_key = '{preferred_variant}'
         WHERE NULLIF(TRIM(t.{source_col}), '') IS NOT NULL
           AND (
                ti.id IS NULL
                OR ti.source_url <> t.{source_col}
                OR (
                    ti.storage_mode = ?
                    AND pv.id IS NULL
                )
           )
         ORDER BY t.created_at ASC
         LIMIT ?",
    );

    let rows = sqlx::query(&sql)
        .bind(kind.as_str())
        .bind(TitleImageStorageMode::AvifMaster.as_str())
        .bind(limit as i64)
        .fetch_all(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(rows
        .into_iter()
        .map(|row| TitleImageSyncTask {
            title_id: row.get("title_id"),
            source_url: row.get("source_url"),
            cached_source_url: row.try_get("cached_source_url").unwrap_or(None),
        })
        .collect())
}

pub(crate) async fn replace_title_image_query(
    pool: &SqlitePool,
    title_id: &str,
    replacement: TitleImageReplacement,
) -> AppResult<()> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut tx = pool
        .begin()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let image_id = sqlx::query_scalar::<_, String>(
        "SELECT id FROM title_images WHERE title_id = ? AND kind = ?",
    )
    .bind(title_id)
    .bind(replacement.kind.as_str())
    .fetch_optional(&mut *tx)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?
    .unwrap_or_else(|| Uuid::new_v4().to_string());

    let exists = sqlx::query_scalar::<_, i64>("SELECT COUNT(1) FROM title_images WHERE id = ?")
        .bind(&image_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?
        > 0;

    if exists {
        sqlx::query(
            "UPDATE title_images SET
                source_url = ?,
                source_etag = ?,
                source_last_modified = ?,
                source_format = ?,
                source_width = ?,
                source_height = ?,
                storage_mode = ?,
                master_format = ?,
                master_sha256 = ?,
                master_width = ?,
                master_height = ?,
                bytes = ?,
                updated_at = ?
             WHERE id = ?",
        )
        .bind(&replacement.source_url)
        .bind(&replacement.source_etag)
        .bind(&replacement.source_last_modified)
        .bind(&replacement.source_format)
        .bind(replacement.source_width)
        .bind(replacement.source_height)
        .bind(replacement.storage_mode.as_str())
        .bind(&replacement.master_format)
        .bind(&replacement.master_sha256)
        .bind(replacement.master_width)
        .bind(replacement.master_height)
        .bind(&replacement.master_bytes)
        .bind(&now)
        .bind(&image_id)
        .execute(&mut *tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    } else {
        sqlx::query(
            "INSERT INTO title_images (
                id, title_id, provider, provider_image_id, kind, source_url, source_etag,
                source_last_modified, source_format, source_width, source_height, storage_mode,
                master_path, master_format, master_sha256, master_width, master_height, bytes,
                created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&image_id)
        .bind(title_id)
        .bind("tvdb")
        .bind(Option::<String>::None)
        .bind(replacement.kind.as_str())
        .bind(&replacement.source_url)
        .bind(&replacement.source_etag)
        .bind(&replacement.source_last_modified)
        .bind(&replacement.source_format)
        .bind(replacement.source_width)
        .bind(replacement.source_height)
        .bind(replacement.storage_mode.as_str())
        .bind(Option::<String>::None)
        .bind(&replacement.master_format)
        .bind(&replacement.master_sha256)
        .bind(replacement.master_width)
        .bind(replacement.master_height)
        .bind(&replacement.master_bytes)
        .bind(&now)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    }

    sqlx::query("DELETE FROM title_image_variants WHERE title_image_id = ?")
        .bind(&image_id)
        .execute(&mut *tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    for variant in &replacement.variants {
        sqlx::query(
            "INSERT INTO title_image_variants (
                id, title_image_id, variant_key, path, format, width, height, bytes, sha256, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&image_id)
        .bind(&variant.variant_key)
        .bind(Option::<String>::None)
        .bind(&variant.format)
        .bind(variant.width)
        .bind(variant.height)
        .bind(&variant.bytes)
        .bind(&variant.sha256)
        .bind(&now)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    }

    tx.commit()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}

pub(crate) async fn get_title_image_blob_query(
    pool: &SqlitePool,
    title_id: &str,
    kind: TitleImageKind,
    variant_key: &str,
) -> AppResult<Option<TitleImageBlob>> {
    if variant_key == "original" {
        let row = sqlx::query(
            "SELECT master_format, master_sha256, bytes
             FROM title_images
             WHERE title_id = ? AND kind = ?",
        )
        .bind(title_id)
        .bind(kind.as_str())
        .fetch_optional(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

        return Ok(row.map(|row| TitleImageBlob {
            content_type: content_type_for_format(row.get::<String, _>("master_format")),
            etag: row.get("master_sha256"),
            bytes: row.get("bytes"),
        }));
    }

    let row = sqlx::query(
        "SELECT tiv.format, tiv.sha256, tiv.bytes
         FROM title_image_variants tiv
         INNER JOIN title_images ti ON ti.id = tiv.title_image_id
         WHERE ti.title_id = ? AND ti.kind = ? AND tiv.variant_key = ?",
    )
    .bind(title_id)
    .bind(kind.as_str())
    .bind(variant_key)
    .fetch_optional(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(row.map(|row| TitleImageBlob {
        content_type: content_type_for_format(row.get::<String, _>("format")),
        etag: row.get("sha256"),
        bytes: row.get("bytes"),
    }))
}

async fn load_local_image_url_map(
    pool: &SqlitePool,
    kind: TitleImageKind,
    preferred_variant: &str,
    title_ids: &[String],
) -> AppResult<HashMap<String, String>> {
    if title_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let placeholders = title_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT ti.title_id, ti.storage_mode, ti.master_sha256, pv.sha256 AS pv_sha256
         FROM title_images ti
         LEFT JOIN title_image_variants pv
           ON pv.title_image_id = ti.id
          AND pv.variant_key = '{preferred_variant}'
         WHERE ti.kind = '{kind_str}' AND ti.title_id IN ({placeholders})",
        kind_str = kind.as_str(),
    );

    let mut query = sqlx::query(&sql);
    for title_id in title_ids {
        query = query.bind(title_id);
    }

    let rows = query
        .fetch_all(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let base_path = normalized_base_path_from_env();
    let mut out = HashMap::with_capacity(rows.len());
    for row in rows {
        let title_id: String = row.get("title_id");
        let storage_mode = row.get::<String, _>("storage_mode");
        let master_sha256: String = row.get("master_sha256");
        let pv_sha256: Option<String> = row.try_get("pv_sha256").unwrap_or(None);
        let (variant, version_hash) = if storage_mode == TitleImageStorageMode::Original.as_str() {
            ("original", master_sha256)
        } else if let Some(pv_sha256) = pv_sha256 {
            (preferred_variant, pv_sha256)
        } else {
            warn!(
                title_id = %title_id,
                kind = kind.as_str(),
                storage_mode,
                "image cache missing preferred variant; serving original master until refresh repairs it"
            );
            ("original", master_sha256)
        };
        out.insert(
            title_id.clone(),
            synthesize_local_title_image_url(&base_path, &title_id, kind, variant, &version_hash),
        );
    }

    Ok(out)
}

fn build_image_variants(
    kind: TitleImageKind,
    rgba: &RgbaImage,
    master_bytes: &[u8],
) -> AppResult<Vec<TitleImageVariantRecord>> {
    match kind {
        TitleImageKind::Poster => {
            let (source_width, source_height) = rgba.dimensions();
            let mut variants = Vec::with_capacity(POSTER_VARIANT_WIDTHS.len());
            for target_width in POSTER_VARIANT_WIDTHS {
                let actual_width = source_width.min(target_width);
                let actual_height = scaled_height(source_width, source_height, actual_width);
                let bytes = if actual_width == source_width {
                    master_bytes.to_vec()
                } else {
                    let speed = if target_width >= 500 {
                        AVIF_SPEED
                    } else {
                        AVIF_VARIANT_SPEED
                    };
                    let resized = resize_rgba(rgba, actual_width, actual_height)?;
                    encode_avif(&resized, speed, AVIF_QUALITY)?
                };
                variants.push(TitleImageVariantRecord {
                    variant_key: format!("w{target_width}"),
                    format: SupportedImageFormat::Avif.as_str().to_string(),
                    width: actual_width as i32,
                    height: actual_height as i32,
                    sha256: sha256_hex(&bytes),
                    bytes,
                });
            }
            Ok(variants)
        }
        TitleImageKind::Banner => {
            // Full-resolution AVIF — single "master" variant, no resizing
            let bytes = encode_avif(rgba, AVIF_SPEED, AVIF_QUALITY)?;
            let (width, height) = rgba.dimensions();
            Ok(vec![TitleImageVariantRecord {
                variant_key: "master".to_string(),
                format: SupportedImageFormat::Avif.as_str().to_string(),
                width: width as i32,
                height: height as i32,
                sha256: sha256_hex(&bytes),
                bytes,
            }])
        }
        TitleImageKind::Fanart => {
            // Full-resolution AVIF — single "master" variant, no resizing
            let bytes = encode_avif(rgba, AVIF_SPEED, AVIF_QUALITY)?;
            let (width, height) = rgba.dimensions();
            Ok(vec![TitleImageVariantRecord {
                variant_key: "master".to_string(),
                format: SupportedImageFormat::Avif.as_str().to_string(),
                width: width as i32,
                height: height as i32,
                sha256: sha256_hex(&bytes),
                bytes,
            }])
        }
    }
}

fn resize_rgba(image: &RgbaImage, width: u32, height: u32) -> AppResult<RgbaImage> {
    let src = fir::images::Image::from_vec_u8(
        image.width(),
        image.height(),
        image.clone().into_raw(),
        fir::PixelType::U8x4,
    )
    .map_err(|err| AppError::Repository(format!("failed to prepare resize source: {err}")))?;
    let mut dst = fir::images::Image::new(width, height, fir::PixelType::U8x4);
    let mut resizer = fir::Resizer::new();
    resizer
        .resize(
            &src,
            &mut dst,
            &fir::ResizeOptions::new()
                .resize_alg(fir::ResizeAlg::Convolution(fir::FilterType::Lanczos3)),
        )
        .map_err(|err| AppError::Repository(format!("failed to resize image: {err}")))?;
    let bytes = dst.into_vec();
    RgbaImage::from_raw(width, height, bytes)
        .ok_or_else(|| AppError::Repository("failed to materialize resized image".to_string()))
}

fn encode_avif(image: &RgbaImage, speed: u8, quality: u8) -> AppResult<Vec<u8>> {
    let mut bytes = Vec::new();
    AvifEncoder::new_with_speed_quality(&mut bytes, speed, quality)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ColorType::Rgba8.into(),
        )
        .map_err(|err| AppError::Repository(format!("failed to encode AVIF image: {err}")))?;
    Ok(bytes)
}

fn scaled_height(source_width: u32, source_height: u32, target_width: u32) -> u32 {
    if target_width >= source_width {
        source_height
    } else {
        ((source_height as u64 * target_width as u64) / source_width as u64)
            .max(1)
            .try_into()
            .unwrap_or(source_height)
    }
}

fn read_exif_orientation(bytes: &[u8]) -> Option<u16> {
    let mut cursor = Cursor::new(bytes);
    let reader = exif::Reader::new().read_from_container(&mut cursor).ok()?;
    reader
        .get_field(exif::Tag::Orientation, exif::In::PRIMARY)
        .and_then(|field| field.value.get_uint(0))
        .map(|value| value as u16)
}

fn apply_orientation(image: DynamicImage, orientation: u16) -> DynamicImage {
    match orientation {
        2 => image.fliph(),
        3 => image.rotate180(),
        4 => image.flipv(),
        5 => image.rotate90().fliph(),
        6 => image.rotate90(),
        7 => image.rotate270().fliph(),
        8 => image.rotate270(),
        _ => image,
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let hash = digest::digest(&digest::SHA256, bytes);
    hash.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

fn normalized_base_path_from_env() -> String {
    let Some(raw) = std::env::var("SCRYER_BASE_PATH").ok() else {
        return String::new();
    };

    let segments = raw
        .trim()
        .replace('\\', "/")
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if segments.is_empty() {
        String::new()
    } else {
        format!("/{}", segments.join("/"))
    }
}

pub(crate) fn synthesize_local_title_image_url(
    base_path: &str,
    title_id: &str,
    kind: TitleImageKind,
    variant_key: &str,
    version_hash: &str,
) -> String {
    let version = version_hash.chars().take(16).collect::<String>();
    format!(
        "{base_path}/images/titles/{title_id}/{}/{variant_key}?v={version}",
        kind.as_str()
    )
}

fn content_type_for_format(format: String) -> String {
    match format.trim().to_ascii_lowercase().as_str() {
        "jpeg" | "jpg" => "image/jpeg".to_string(),
        "png" => "image/png".to_string(),
        "webp" => "image/webp".to_string(),
        "avif" => "image/avif".to_string(),
        other => format!("image/{other}"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SupportedImageFormat {
    Jpeg,
    Png,
    Webp,
    Avif,
}

impl SupportedImageFormat {
    fn from_image_format(format: ImageFormat) -> Option<Self> {
        match format {
            ImageFormat::Jpeg => Some(Self::Jpeg),
            ImageFormat::Png => Some(Self::Png),
            ImageFormat::WebP => Some(Self::Webp),
            ImageFormat::Avif => Some(Self::Avif),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Jpeg => "jpeg",
            Self::Png => "png",
            Self::Webp => "webp",
            Self::Avif => "avif",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_image() -> RgbaImage {
        let mut image = RgbaImage::new(800, 1200);
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            let red = (x % 255) as u8;
            let green = (y % 255) as u8;
            *pixel = image::Rgba([red, green, 180, 255]);
        }
        image
    }

    fn encode_test_image(format: ImageFormat) -> Vec<u8> {
        let dynamic = DynamicImage::ImageRgba8(test_image());
        let mut bytes = Vec::new();
        dynamic
            .write_to(&mut Cursor::new(&mut bytes), format)
            .expect("test image should encode");
        bytes
    }

    #[test]
    fn orientation_transform_rotates_image() {
        let mut image = RgbaImage::new(2, 1);
        image.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        image.put_pixel(1, 0, image::Rgba([0, 255, 0, 255]));

        let rotated = apply_orientation(DynamicImage::ImageRgba8(image), 6).to_rgba8();

        assert_eq!(rotated.dimensions(), (1, 2));
        assert_eq!(rotated.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(rotated.get_pixel(0, 1).0, [0, 255, 0, 255]);
    }

    #[test]
    fn synthesize_local_url_honors_base_path() {
        assert_eq!(
            synthesize_local_title_image_url(
                "/scryer",
                "title-1",
                TitleImageKind::Poster,
                "w500",
                "abcdef0123456789"
            ),
            "/scryer/images/titles/title-1/poster/w500?v=abcdef0123456789"
        );
    }

    #[test]
    fn avif_pipeline_generates_expected_variants() {
        let processor = SqliteTitleImageProcessor::new_for_tests(true);
        let bytes = encode_test_image(ImageFormat::Png);
        let processed = processor
            .process_bytes_for_tests(
                TitleImageKind::Poster,
                "https://example.com/poster.png",
                &bytes,
            )
            .expect("processing should succeed");

        assert_eq!(processed.storage_mode, TitleImageStorageMode::AvifMaster);
        assert_eq!(processed.master_format, "avif");
        assert_eq!(processed.master_width, 800);
        assert_eq!(processed.master_height, 1200);

        let widths = processed
            .variants
            .iter()
            .map(|variant| (variant.variant_key.clone(), (variant.width, variant.height)))
            .collect::<HashMap<_, _>>();
        assert_eq!(widths.get("w500"), Some(&(500, 750)));
        assert_eq!(widths.get("w250"), Some(&(250, 375)));
        assert_eq!(widths.get("w70"), Some(&(70, 105)));
    }

    #[test]
    fn original_fallback_stores_source_bytes_when_avif_disabled() {
        let processor = SqliteTitleImageProcessor::new_for_tests(false);
        let bytes = encode_test_image(ImageFormat::Jpeg);
        let processed = processor
            .process_bytes_for_tests(
                TitleImageKind::Poster,
                "https://example.com/poster.jpg",
                &bytes,
            )
            .expect("processing should succeed");

        assert_eq!(processed.storage_mode, TitleImageStorageMode::Original);
        assert_eq!(processed.master_format, "jpeg");
        assert_eq!(processed.master_bytes, bytes);
        assert!(processed.variants.is_empty());
    }

    #[test]
    fn poster_variants_do_not_upscale_small_images() {
        let processor = SqliteTitleImageProcessor::new_for_tests(true);
        let mut image = RgbaImage::new(120, 180);
        for pixel in image.pixels_mut() {
            *pixel = image::Rgba([32, 96, 160, 255]);
        }
        let bytes = {
            let dynamic = DynamicImage::ImageRgba8(image);
            let mut bytes = Vec::new();
            dynamic
                .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
                .expect("test image should encode");
            bytes
        };

        let processed = processor
            .process_bytes_for_tests(
                TitleImageKind::Poster,
                "https://example.com/poster-small.png",
                &bytes,
            )
            .expect("processing should succeed");

        let widths = processed
            .variants
            .iter()
            .map(|variant| (variant.variant_key.clone(), (variant.width, variant.height)))
            .collect::<HashMap<_, _>>();
        assert_eq!(widths.get("w500"), Some(&(120, 180)));
        assert_eq!(widths.get("w250"), Some(&(120, 180)));
        assert_eq!(widths.get("w70"), Some(&(70, 105)));
    }

    #[test]
    fn pipeline_decodes_supported_formats() {
        let processor = SqliteTitleImageProcessor::new_for_tests(true);
        for format in [ImageFormat::Png, ImageFormat::Jpeg, ImageFormat::WebP] {
            let bytes = encode_test_image(format);
            let processed = processor
                .process_bytes_for_tests(
                    TitleImageKind::Poster,
                    "https://example.com/poster",
                    &bytes,
                )
                .expect("supported image should decode");
            assert_eq!(processed.source_width, 800);
            assert_eq!(processed.source_height, 1200);
        }
    }
}
