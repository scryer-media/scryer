use std::io::Cursor;
use std::sync::Arc;

use super::{normalize_title_image_source_url, title_image_blob_digest};
use async_trait::async_trait;
use fast_image_resize as fir;
use image::codecs::avif::AvifEncoder;
use image::{DynamicImage, ImageEncoder, ImageFormat, RgbaImage};
use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE, ETAG, LAST_MODIFIED};
use scryer_application::{
    AppError, AppResult, TitleImageKind, TitleImageProcessor, TitleImageSourceResult,
    TitleImageVariantRecord, TitleImageVariantSpec,
};
use scryer_outbound_http::{
    OutboundHttpClient, OutboundHttpError, RateLimitRegistry, RequestPolicy,
    no_redirect_reqwest_client,
};

const MAX_SOURCE_BYTES: usize = 20 * 1024 * 1024;
const AVIF_SPEED: u8 = 4;
const AVIF_QUALITY: u8 = 85;
const PROXY_AVIF_SPEED: u8 = 10;
const PROXY_AVIF_QUALITY: u8 = 60;

fn artwork_pool(workers: usize) -> AppResult<rayon::ThreadPool> {
    if !matches!(workers, 2 | 4) {
        return Err(AppError::Validation(
            "artwork worker count must be two or four".into(),
        ));
    }
    let (ready, results) = std::sync::mpsc::channel();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .thread_name(|index| format!("scryer-artwork-{index}"))
        .start_handler(move |index| {
            let result = scryer_application::background_worker_priority();
            match &result {
                Ok(()) => tracing::debug!(index, "artwork worker priority initialized"),
                Err(error) => tracing::error!(index, %error, "artwork worker priority failed"),
            }
            let _ = ready.send(result);
        })
        .build()
        .map_err(|error| AppError::Repository(format!("artwork pool creation failed: {error}")))?;
    for _ in 0..workers {
        results
            .recv()
            .map_err(|error| {
                AppError::Repository(format!("artwork worker startup failed: {error}"))
            })?
            .map_err(|error| {
                AppError::Repository(format!("artwork worker priority failed: {error}"))
            })?;
    }
    tracing::info!(workers, "dedicated artwork pool ready");
    Ok(pool)
}

#[derive(Clone)]
pub struct HttpTitleImageProcessor {
    outbound_http: OutboundHttpClient,
    max_source_bytes: usize,
    avif_enabled: bool,
    encoding_pool: Arc<tokio::sync::RwLock<Option<Arc<rayon::ThreadPool>>>>,
}

impl HttpTitleImageProcessor {
    pub fn new() -> Self {
        Self {
            outbound_http: OutboundHttpClient::new(
                no_redirect_reqwest_client(),
                RateLimitRegistry::new(),
            ),
            max_source_bytes: MAX_SOURCE_BYTES,
            avif_enabled: true,
            encoding_pool: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }

    #[cfg(test)]
    pub fn new_for_tests(avif_enabled: bool) -> Self {
        Self {
            outbound_http: OutboundHttpClient::new(
                no_redirect_reqwest_client(),
                RateLimitRegistry::new(),
            ),
            max_source_bytes: MAX_SOURCE_BYTES,
            avif_enabled,
            encoding_pool: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }

    async fn fetch_source(
        &self,
        source_url: &str,
    ) -> AppResult<(String, Vec<u8>, Option<String>, Option<String>)> {
        let source_url = normalize_title_image_source_url(source_url)?;
        let target =
            scryer_outbound_http::prepare_untrusted_public_http_target(&source_url, "title image")
                .await
                .map_err(|error| AppError::Validation(error.to_string()))?;
        let scope = title_image_scope(&source_url);
        let response = self
            .outbound_http
            .send(
                // Redirect hops would escape the DNS-pinned target client, so
                // this untrusted fetch must never auto-follow.
                RequestPolicy::safe_read(scope, "title_image_fetch")
                    .without_redirects()
                    .with_max_retries(2)
                    .with_backoff(
                        std::time::Duration::from_millis(500),
                        std::time::Duration::from_secs(10),
                    ),
                || target.client().get(target.url().clone()),
            )
            .await
            .map_err(|error| match error {
                OutboundHttpError::DispatchRejected => {
                    AppError::canceled("outbound dispatch is closed")
                }
                OutboundHttpError::RateLimited(rate_limited) => AppError::Repository(
                    match rate_limited.retry_after.filter(|delay| !delay.is_zero()) {
                        Some(delay) => format!(
                            "title image fetch was rate limited; retry after {}s",
                            delay.as_secs()
                        ),
                        None => "title image fetch was rate limited".to_string(),
                    },
                ),
                OutboundHttpError::Transport { source, .. } => {
                    AppError::Repository(format!("failed to fetch title image: {source}"))
                }
            })?;

        if response.status().is_redirection() {
            return Err(AppError::Validation(
                "title image redirects are not allowed".into(),
            ));
        }

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
            && length > self.max_source_bytes
        {
            return Err(AppError::Validation(format!(
                "title image exceeds max size of {} bytes",
                self.max_source_bytes
            )));
        }

        if let Some(content_type) = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            && !content_type.starts_with("image/")
        {
            return Err(AppError::Validation(format!(
                "unsupported title image content type: {content_type}"
            )));
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

        Ok((source_url, bytes.to_vec(), etag, last_modified))
    }

    fn process_bytes(
        &self,
        kind: TitleImageKind,
        source_url: &str,
        bytes: &[u8],
        source_etag: Option<String>,
        source_last_modified: Option<String>,
        variant_specs: Vec<TitleImageVariantSpec>,
    ) -> AppResult<TitleImageSourceResult> {
        let guessed_format = image::guess_format(bytes)
            .map_err(|err| AppError::Validation(format!("failed to detect image format: {err}")))?;
        let source_format = SupportedImageFormat::from_image_format(guessed_format)
            .ok_or_else(|| AppError::Validation("unsupported image format".to_string()))?;
        let decoded = image::load_from_memory_with_format(bytes, guessed_format)
            .map_err(|err| AppError::Validation(format!("failed to decode image: {err}")))?;
        let oriented = apply_orientation(decoded, read_exif_orientation(bytes).unwrap_or(1));
        let rgba = oriented.to_rgba8();
        let (source_width, source_height) = rgba.dimensions();

        if source_width == 0 || source_height == 0 {
            return Err(AppError::Validation(
                "image dimensions must be non-zero".to_string(),
            ));
        }

        let variants = if self.avif_enabled {
            build_requested_variants(&rgba, &variant_specs)?
        } else {
            Vec::new()
        };

        Ok(TitleImageSourceResult {
            kind,
            requested_source_url: source_url.to_string(),
            source_url: source_url.to_string(),
            source_etag,
            source_last_modified,
            source_format: source_format.as_str().to_string(),
            source_width: source_width as i32,
            source_height: source_height as i32,
            variants,
        })
    }

    #[cfg(test)]
    pub fn process_bytes_for_tests(
        &self,
        kind: TitleImageKind,
        source_url: &str,
        bytes: &[u8],
        variant_specs: Vec<TitleImageVariantSpec>,
    ) -> AppResult<TitleImageSourceResult> {
        artwork_pool(2)?
            .install(|| self.process_bytes(kind, source_url, bytes, None, None, variant_specs))
    }
}

fn title_image_scope(source_url: &str) -> String {
    match reqwest::Url::parse(source_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
    {
        Some(host) => format!("title_image:{host}"),
        None => "title_image:unknown".to_string(),
    }
}

impl Default for HttpTitleImageProcessor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl TitleImageProcessor for HttpTitleImageProcessor {
    async fn encode_cached_jpeg(&self, bytes: Vec<u8>, width: u32) -> AppResult<Vec<u8>> {
        let pool = self.encoding_pool.read().await.clone().ok_or_else(|| {
            AppError::Repository("artwork workers have not been configured".into())
        })?;
        let (send, receive) = tokio::sync::oneshot::channel();
        pool.spawn(move || {
            if send.is_closed() {
                return;
            }
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                encode_proxy_jpeg(&bytes, width)
            }))
            .unwrap_or_else(|_| Err(AppError::Repository("cached JPEG task panicked".into())));
            let _ = send.send(result);
        });
        receive
            .await
            .map_err(|error| AppError::Repository(format!("cached JPEG task failed: {error}")))?
    }

    async fn configure_encoding_workers(&self, workers: usize) -> AppResult<()> {
        let pool = tokio::task::spawn_blocking(move || artwork_pool(workers))
            .await
            .map_err(|error| {
                AppError::Repository(format!("artwork pool startup failed: {error}"))
            })??;
        *self.encoding_pool.write().await = Some(Arc::new(pool));
        Ok(())
    }

    async fn fetch_and_process_image(
        &self,
        kind: TitleImageKind,
        source_url: &str,
        variants: Vec<TitleImageVariantSpec>,
    ) -> AppResult<TitleImageSourceResult> {
        let requested_source_url = source_url.to_string();
        let (source_url, bytes, etag, last_modified) = self.fetch_source(source_url).await?;
        let this = self.clone();
        let pool = self.encoding_pool.read().await.clone().ok_or_else(|| {
            AppError::Repository("artwork workers have not been configured".into())
        })?;
        let (send, receive) = tokio::sync::oneshot::channel();
        pool.spawn(move || {
            // Keep a panic confined to this image instead of Rayon aborting the process.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                this.process_bytes(kind, &source_url, &bytes, etag, last_modified, variants)
            }))
            .unwrap_or_else(|_| Err(AppError::Repository("image encode task panicked".into())));
            let _ = send.send(result);
        });
        let mut result = receive
            .await
            .map_err(|err| AppError::Repository(format!("image encode task failed: {err}")))??;
        result.requested_source_url = requested_source_url;
        Ok(result)
    }
}

fn encode_proxy_jpeg(bytes: &[u8], width: u32) -> AppResult<Vec<u8>> {
    if bytes.len() > MAX_SOURCE_BYTES || width == 0 || width > 1280 {
        return Err(AppError::Validation(
            "invalid cached JPEG size or target width".into(),
        ));
    }
    if image::guess_format(bytes).ok() != Some(ImageFormat::Jpeg) {
        return Err(AppError::Validation("cached image is not a JPEG".into()));
    }
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), ImageFormat::Jpeg);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|error| AppError::Validation(format!("failed to decode cached JPEG: {error}")))?;
    let rgba = apply_orientation(decoded, read_exif_orientation(bytes).unwrap_or(1)).to_rgba8();
    let width = width.min(rgba.width());
    let height = scaled_height(rgba.width(), rgba.height(), width);
    let resized = if width == rgba.width() {
        rgba
    } else {
        image::imageops::resize(&rgba, width, height, image::imageops::FilterType::Lanczos3)
    };
    encode_avif(&resized, PROXY_AVIF_SPEED, PROXY_AVIF_QUALITY)
}

fn build_requested_variants(
    rgba: &RgbaImage,
    variant_specs: &[TitleImageVariantSpec],
) -> AppResult<Vec<TitleImageVariantRecord>> {
    let mut variants = Vec::with_capacity(variant_specs.len());
    for spec in variant_specs {
        variants.push(build_width_variant(rgba, &spec.variant_key, spec.width)?);
    }
    Ok(variants)
}

fn build_width_variant(
    rgba: &RgbaImage,
    variant_key: &str,
    target_width: u32,
) -> AppResult<TitleImageVariantRecord> {
    let (source_width, source_height) = rgba.dimensions();
    let actual_width = source_width.min(target_width);
    let actual_height = scaled_height(source_width, source_height, actual_width);
    let variant_image = if actual_width == source_width {
        rgba.clone()
    } else {
        resize_rgba_linear_box(rgba, actual_width, actual_height)?
    };
    let bytes = encode_avif(&variant_image, AVIF_SPEED, AVIF_QUALITY)?;
    Ok(TitleImageVariantRecord {
        variant_key: variant_key.to_string(),
        format: SupportedImageFormat::Avif.as_str().to_string(),
        width: actual_width as i32,
        height: actual_height as i32,
        digest: title_image_blob_digest(&bytes),
        bytes,
    })
}

fn resize_rgba_linear_box(image: &RgbaImage, width: u32, height: u32) -> AppResult<RgbaImage> {
    let linear = rgba_to_premultiplied_linear(image);
    let src = fir::images::ImageRef::from_pixels(image.width(), image.height(), &linear)
        .map_err(|err| AppError::Repository(format!("failed to prepare resize source: {err}")))?;
    let mut dst_pixels = vec![fir::pixels::F32x4::default(); width as usize * height as usize];
    let mut resizer = fir::Resizer::new();

    {
        let (head, dst_bytes, tail) = unsafe { dst_pixels.align_to_mut::<u8>() };
        debug_assert!(head.is_empty());
        debug_assert!(tail.is_empty());
        let mut dst =
            fir::images::Image::from_slice_u8(width, height, dst_bytes, fir::PixelType::F32x4)
                .map_err(|err| {
                    AppError::Repository(format!("failed to prepare resize destination: {err}"))
                })?;
        resizer
            .resize(
                &src,
                &mut dst,
                &fir::ResizeOptions::new()
                    .resize_alg(fir::ResizeAlg::Convolution(fir::FilterType::Box)),
            )
            .map_err(|err| AppError::Repository(format!("failed to resize image: {err}")))?;
    }

    Ok(linear_to_rgba(width, height, &dst_pixels))
}

fn rgba_to_premultiplied_linear(image: &RgbaImage) -> Vec<fir::pixels::F32x4> {
    let mut out = Vec::with_capacity(image.width() as usize * image.height() as usize);
    for pixel in image.as_raw().chunks_exact(4) {
        let alpha = pixel[3] as f32 / 255.0;
        out.push(fir::pixels::F32x4::new([
            srgb_to_linear(pixel[0]) * alpha,
            srgb_to_linear(pixel[1]) * alpha,
            srgb_to_linear(pixel[2]) * alpha,
            alpha,
        ]));
    }
    out
}

fn linear_to_rgba(width: u32, height: u32, pixels: &[fir::pixels::F32x4]) -> RgbaImage {
    let mut out = Vec::with_capacity(width as usize * height as usize * 4);
    for pixel in pixels {
        let [r, g, b, a] = pixel.0;
        let alpha = a.clamp(0.0, 1.0);
        let scale = if alpha > 0.00001 { 1.0 / alpha } else { 0.0 };
        out.push(linear_to_srgb_u8((r * scale).clamp(0.0, 1.0)));
        out.push(linear_to_srgb_u8((g * scale).clamp(0.0, 1.0)));
        out.push(linear_to_srgb_u8((b * scale).clamp(0.0, 1.0)));
        out.push((alpha * 255.0).round() as u8);
    }
    RgbaImage::from_raw(width, height, out)
        .expect("linear resize conversion should preserve dimensions")
}

fn srgb_to_linear(value: u8) -> f32 {
    let channel = value as f32 / 255.0;
    if channel <= 0.04045 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb_u8(value: f32) -> u8 {
    let channel = if value <= 0.0031308 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    };
    (channel.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn encode_avif(image: &RgbaImage, speed: u8, quality: u8) -> AppResult<Vec<u8>> {
    if !std::thread::current()
        .name()
        .is_some_and(|name| name.starts_with("scryer-artwork-"))
    {
        return Err(AppError::Repository(
            "AVIF encoding requires an artwork worker".into(),
        ));
    }
    let mut bytes = Vec::new();
    AvifEncoder::new_with_speed_quality(&mut bytes, speed, quality)
        // None keeps ravif/rav1e's nested Rayon work in our current pool.
        .with_num_threads(None)
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
        ((source_height as u64 * target_width as u64) / source_width as u64).max(1) as u32
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
    use crate::media::images::synthesize_local_title_image_url;
    use std::collections::HashMap;

    #[tokio::test]
    async fn avif_nested_work_stays_on_background_pool_when_global_pool_is_busy() {
        // Nextest runs this test in its own process. A blocked, normal-priority global
        // worker detects accidental fallback anywhere in image -> ravif -> rav1e.
        rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build_global()
            .unwrap();
        let (release, wait) = std::sync::mpsc::channel();
        let (started, ready) = std::sync::mpsc::channel();
        rayon::spawn(move || {
            started.send(()).unwrap();
            let _ = wait.recv();
        });
        ready.recv().unwrap();
        let encode = tokio::task::spawn_blocking(|| {
            for workers in [2, 4] {
                let pool = artwork_pool(workers).unwrap();
                let names = pool.broadcast(|_| {
                    // Reapplying and reading back QoS is checked on macOS by this helper.
                    scryer_application::background_worker_priority().unwrap();
                    std::thread::current().name().unwrap().to_string()
                });
                assert_eq!(names.len(), workers);
                assert!(names.iter().all(|name| name.starts_with("scryer-artwork-")));
                pool.install(|| {
                    for alpha in [255, 96] {
                        let mut pixels = RgbaImage::new(64, 96);
                        for (x, y, pixel) in pixels.enumerate_pixels_mut() {
                            *pixel = image::Rgba([x as u8, y as u8, 180, alpha]);
                        }
                        let bytes = encode_avif(&pixels, AVIF_SPEED, AVIF_QUALITY).unwrap();
                        assert!(!bytes.is_empty());
                        assert_eq!(image::guess_format(&bytes).unwrap(), ImageFormat::Avif);
                        // ravif's color/alpha join and all nested parallel operations
                        // share this pool's fixed total worker budget.
                        assert_eq!(rayon::current_num_threads(), workers);
                    }
                });
            }
        });
        let outcome = tokio::time::timeout(std::time::Duration::from_secs(30), encode).await;
        let _ = release.send(());
        outcome
            .expect("AVIF must finish without the global pool")
            .unwrap();
    }

    #[tokio::test]
    async fn cached_jpeg_uses_fast_avif_and_bounds_dimensions_without_upscaling() {
        let processor = HttpTitleImageProcessor::new();
        processor.configure_encoding_workers(2).await.unwrap();
        for (w, h, target, expected) in [
            (600, 900, 300, (300, 450)),
            (300, 450, 300, (300, 450)),
            (40, 60, 300, (40, 60)),
            (1600, 900, 1280, (1280, 720)),
        ] {
            let rgb = image::RgbImage::from_pixel(w, h, image::Rgb([45, 89, 120]));
            let mut jpeg = Vec::new();
            image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
                .write_image(rgb.as_raw(), w, h, image::ExtendedColorType::Rgb8)
                .unwrap();
            let encoded = processor.encode_cached_jpeg(jpeg, target).await.unwrap();
            assert_eq!(image::guess_format(&encoded).unwrap(), ImageFormat::Avif);
            let ispe = encoded.windows(4).position(|v| v == b"ispe").unwrap();
            let width = u32::from_be_bytes(encoded[ispe + 8..ispe + 12].try_into().unwrap());
            let height = u32::from_be_bytes(encoded[ispe + 12..ispe + 16].try_into().unwrap());
            assert_eq!((width, height), expected);
        }
        assert!(
            processor
                .encode_cached_jpeg(b"not a JPEG".to_vec(), 300)
                .await
                .is_err()
        );
        assert_eq!((AVIF_SPEED, AVIF_QUALITY), (4, 85));
        assert_eq!((PROXY_AVIF_SPEED, PROXY_AVIF_QUALITY), (10, 60));
    }

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
        let processor = HttpTitleImageProcessor::new_for_tests(true);
        let bytes = encode_test_image(ImageFormat::Png);
        let processed = processor
            .process_bytes_for_tests(
                TitleImageKind::Poster,
                "https://example.com/poster.png",
                &bytes,
                vec![
                    TitleImageVariantSpec {
                        variant_key: "w250".to_string(),
                        width: 250,
                    },
                    TitleImageVariantSpec {
                        variant_key: "w70".to_string(),
                        width: 70,
                    },
                ],
            )
            .expect("processing should succeed");

        assert_eq!(processed.source_format, "png");
        assert_eq!(processed.source_width, 800);
        assert_eq!(processed.source_height, 1200);

        let widths = processed
            .variants
            .iter()
            .map(|variant| (variant.variant_key.clone(), (variant.width, variant.height)))
            .collect::<HashMap<_, _>>();
        assert_eq!(widths.get("w250"), Some(&(250, 375)));
        assert_eq!(widths.get("w70"), Some(&(70, 105)));
        assert!(
            processed
                .variants
                .iter()
                .all(|variant| variant.digest.starts_with("blake3:"))
        );
    }

    #[test]
    fn fanart_avif_pipeline_generates_w1280_variant_without_upscaling() {
        let processor = HttpTitleImageProcessor::new_for_tests(true);
        let bytes = encode_test_image(ImageFormat::Png);

        let processed = processor
            .process_bytes_for_tests(
                TitleImageKind::Fanart,
                "https://example.com/fanart.png",
                &bytes,
                vec![TitleImageVariantSpec {
                    variant_key: "w1280".to_string(),
                    width: 1280,
                }],
            )
            .expect("processing should succeed");

        assert_eq!(processed.source_format, "png");

        let widths = processed
            .variants
            .iter()
            .map(|variant| (variant.variant_key.clone(), (variant.width, variant.height)))
            .collect::<HashMap<_, _>>();
        assert_eq!(widths.get("w1280"), Some(&(800, 1200)));
    }

    #[test]
    fn no_variants_are_stored_when_avif_disabled() {
        let processor = HttpTitleImageProcessor::new_for_tests(false);
        let bytes = encode_test_image(ImageFormat::Jpeg);
        let processed = processor
            .process_bytes_for_tests(
                TitleImageKind::Poster,
                "https://example.com/poster.jpg",
                &bytes,
                vec![TitleImageVariantSpec {
                    variant_key: "w250".to_string(),
                    width: 250,
                }],
            )
            .expect("processing should succeed");

        assert_eq!(processed.source_format, "jpeg");
        assert!(processed.variants.is_empty());
    }

    #[test]
    fn poster_variants_do_not_upscale_small_images() {
        let processor = HttpTitleImageProcessor::new_for_tests(true);
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
                vec![
                    TitleImageVariantSpec {
                        variant_key: "w250".to_string(),
                        width: 250,
                    },
                    TitleImageVariantSpec {
                        variant_key: "w70".to_string(),
                        width: 70,
                    },
                ],
            )
            .expect("processing should succeed");

        let widths = processed
            .variants
            .iter()
            .map(|variant| (variant.variant_key.clone(), (variant.width, variant.height)))
            .collect::<HashMap<_, _>>();
        assert_eq!(widths.get("w250"), Some(&(120, 180)));
        assert_eq!(widths.get("w70"), Some(&(70, 105)));
    }

    #[test]
    fn normalize_title_image_source_url_expands_relative_tvdb_paths() {
        assert_eq!(
            normalize_title_image_source_url(
                "/banners/movies/147325/backgrounds//5vyMUvxy6W0xU9Unnh5M7WXkh4l.jpg"
            )
            .expect("relative TVDB path should normalize"),
            "https://artworks.thetvdb.com/banners/movies/147325/backgrounds/5vyMUvxy6W0xU9Unnh5M7WXkh4l.jpg"
        );
    }

    #[test]
    fn normalize_title_image_source_url_collapses_duplicate_path_separators() {
        assert_eq!(
            normalize_title_image_source_url(
                "https://artworks.thetvdb.com/banners/posters//example.jpg"
            )
            .expect("absolute TVDB URL should normalize"),
            "https://artworks.thetvdb.com/banners/posters/example.jpg"
        );
    }

    #[tokio::test]
    async fn title_image_untrusted_fetch_blocks_local_and_private_addresses() {
        for raw in [
            "http://127.0.0.1/poster.jpg",
            "http://10.0.0.5/poster.jpg",
            "http://172.16.0.5/poster.jpg",
            "http://192.168.1.5/poster.jpg",
            "http://169.254.1.1/poster.jpg",
            "http://224.0.0.1/poster.jpg",
            "http://[::1]/poster.jpg",
            "http://[fd00::1]/poster.jpg",
            "http://[fe80::1]/poster.jpg",
            "http://[ff02::1]/poster.jpg",
        ] {
            assert!(
                scryer_outbound_http::prepare_untrusted_public_http_target(raw, "title image")
                    .await
                    .is_err(),
                "{raw} should be blocked"
            );
        }
    }

    #[tokio::test]
    async fn title_image_untrusted_fetch_allows_public_addresses() {
        scryer_outbound_http::prepare_untrusted_public_http_target(
            "http://93.184.216.34/poster.jpg",
            "title image",
        )
        .await
        .expect("literal public IP should be allowed");
    }

    #[test]
    fn pipeline_decodes_supported_formats() {
        let processor = HttpTitleImageProcessor::new_for_tests(false);
        for format in [ImageFormat::Png, ImageFormat::Jpeg, ImageFormat::WebP] {
            let bytes = encode_test_image(format);
            let processed = processor
                .process_bytes_for_tests(
                    TitleImageKind::Poster,
                    "https://example.com/poster",
                    &bytes,
                    Vec::new(),
                )
                .expect("supported image should decode");
            assert_eq!(processed.source_width, 800);
            assert_eq!(processed.source_height, 1200);
        }
    }
}
