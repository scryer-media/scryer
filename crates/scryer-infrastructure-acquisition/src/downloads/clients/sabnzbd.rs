use std::path::PathBuf;

use async_compression::tokio::bufread::ZstdDecoder;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::{StatusCode, Url, multipart};
use scryer_application::{
    AppError, AppResult, DownloadClient, DownloadClientAddRequest, DownloadClientStatus,
    DownloadGrabResult, NullStagedNzbStore, RateLimitCooldownAction, StagedNzbRef, StagedNzbStore,
};
use scryer_domain::{CompletedDownload, DownloadQueueItem, DownloadQueueState};
use scryer_outbound_http::{
    OutboundHttpClient, OutboundHttpError, RateLimitRegistry, RequestPolicy, generic_reqwest_client,
};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::sync::{OnceCell, Semaphore};
use tokio_util::io::ReaderStream;
use tracing::{debug, info, warn};

use super::{
    extract_f64_value, extract_i64_value, parse_duration_seconds, resolve_staged_nzb_for_request,
};

#[derive(Clone)]
pub struct SabnzbdDownloadClient {
    base_url: String,
    api_key: Option<String>,
    username: Option<String>,
    password: Option<String>,
    outbound_http: OutboundHttpClient,
    staged_nzb_store: Arc<dyn StagedNzbStore>,
    staged_nzb_pipeline_limit: Arc<Semaphore>,
    /// The SAB API path that answered a read for this backend, cached so
    /// mutations (addfile) can be pinned to it instead of guessing between
    /// `<base>/api` and `<base>/sabnzbd/api` on every submit. Real SABnzbd and
    /// nzbdav answer at `/api`; altmount serves the SAB-compat API at
    /// `/sabnzbd/api` (its `/api` is a different application). Discovering the
    /// path with an idempotent GET and pinning the POST there is what lets a
    /// landed-but-lost addfile be reconciled instead of blindly re-POSTed.
    resolved_api_url: Arc<OnceCell<String>>,
}

#[derive(Debug, Deserialize)]
struct SabnzbdConfigEnvelope {
    config: SabnzbdConfig,
}

#[derive(Debug, Default, Deserialize)]
struct SabnzbdConfig {
    #[serde(default)]
    misc: SabnzbdConfigMisc,
    #[serde(default)]
    categories: Vec<SabnzbdCategory>,
    #[serde(default)]
    sorters: Vec<SabnzbdSorter>,
}

#[derive(Debug, Default, Deserialize)]
struct SabnzbdConfigMisc {
    #[serde(default)]
    complete_dir: String,
    #[serde(default, deserialize_with = "deserialize_sab_string_list")]
    tv_categories: Vec<String>,
    #[serde(default)]
    enable_tv_sorting: bool,
    #[serde(default, deserialize_with = "deserialize_sab_string_list")]
    movie_categories: Vec<String>,
    #[serde(default)]
    enable_movie_sorting: bool,
    #[serde(default, deserialize_with = "deserialize_sab_string_list")]
    date_categories: Vec<String>,
    #[serde(default)]
    enable_date_sorting: bool,
    #[serde(default)]
    history_retention: String,
    #[serde(default)]
    history_retention_option: String,
    #[serde(default)]
    history_retention_number: i64,
}

#[derive(Debug, Default, Deserialize)]
struct SabnzbdCategory {
    #[serde(default, alias = "Name")]
    _name: String,
    #[serde(default, alias = "Dir")]
    dir: String,
}

#[derive(Debug, Default, Deserialize)]
struct SabnzbdSorter {
    #[serde(default, deserialize_with = "deserialize_sab_string_list")]
    sort_cats: Vec<String>,
    #[serde(default)]
    is_active: bool,
}

#[derive(Debug, Deserialize)]
struct SabnzbdFullStatusEnvelope {
    status: SabnzbdFullStatus,
}

#[derive(Debug, Default, Deserialize)]
struct SabnzbdFullStatus {
    #[serde(default, rename = "completedir")]
    complete_dir: String,
}

#[derive(Clone)]
enum SabApiAuth {
    ApiKey(String),
    Credentials { username: String, password: String },
}

const SAB_ADDFILE_UPLOAD_FIELD: &str = "nzbfile";
// Safe reads may make three attempts. Ninety seconds per attempt plus the
// bounded retry backoff remains below the default 300-second feedback gate.
const SABNZBD_HTTP_REQUEST_TIMEOUT: std::time::Duration =
    scryer_outbound_http::DOWNLOAD_CLIENT_HTTP_TIMEOUT;
const SAB_SECRET_QUERY_KEYS: &[&str] = &["apikey", "api_key", "ma_password", "password"];

#[derive(Clone)]
enum SabAddfilePayload {
    File { path: PathBuf, len: u64 },
}

struct SabAddfileRequest<'a> {
    url: &'a str,
    nzb_name: &'a str,
    queue_priority: &'a str,
    upload_payload: SabAddfilePayload,
    upload_filename: &'a str,
    upload_mime: &'a str,
    cat: Option<&'a str>,
    password: Option<&'a str>,
}

impl SabnzbdDownloadClient {
    pub fn new(base_url: String, api_key: String) -> Self {
        Self::with_auth_and_staged_nzb_store(
            base_url,
            Some(api_key),
            None,
            None,
            Arc::new(NullStagedNzbStore),
            Arc::new(Semaphore::new(4)),
        )
    }

    pub fn with_auth(
        base_url: String,
        api_key: Option<String>,
        username: Option<String>,
        password: Option<String>,
    ) -> Self {
        Self::with_auth_and_staged_nzb_store(
            base_url,
            api_key,
            username,
            password,
            Arc::new(NullStagedNzbStore),
            Arc::new(Semaphore::new(4)),
        )
    }

    pub fn with_staged_nzb_store(
        base_url: String,
        api_key: String,
        staged_nzb_store: Arc<dyn StagedNzbStore>,
        staged_nzb_pipeline_limit: Arc<Semaphore>,
    ) -> Self {
        Self::with_auth_and_staged_nzb_store(
            base_url,
            Some(api_key),
            None,
            None,
            staged_nzb_store,
            staged_nzb_pipeline_limit,
        )
    }

    pub fn with_auth_and_staged_nzb_store(
        base_url: String,
        api_key: Option<String>,
        username: Option<String>,
        password: Option<String>,
        staged_nzb_store: Arc<dyn StagedNzbStore>,
        staged_nzb_pipeline_limit: Arc<Semaphore>,
    ) -> Self {
        let http_client = generic_reqwest_client();
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: normalize_optional_auth_value(api_key),
            username: normalize_optional_auth_value(username),
            password: normalize_optional_auth_value(password),
            outbound_http: OutboundHttpClient::new(http_client.clone(), RateLimitRegistry::new()),
            staged_nzb_store,
            staged_nzb_pipeline_limit,
            resolved_api_url: Arc::new(OnceCell::new()),
        }
    }

    fn sab_nzb_path(staged_nzb: &StagedNzbRef) -> PathBuf {
        staged_nzb
            .compressed_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(format!("{}.sab.nzb.part", staged_nzb.id))
    }

    async fn build_transient_nzb_artifact(
        &self,
        staged_nzb: &StagedNzbRef,
    ) -> AppResult<(PathBuf, u64)> {
        let nzb_path = Self::sab_nzb_path(staged_nzb);
        let input = File::open(&staged_nzb.compressed_path)
            .await
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to open staged nzb {}: {error}",
                    staged_nzb.compressed_path.display()
                ))
            })?;
        let mut output = File::create(&nzb_path).await.map_err(|error| {
            AppError::Repository(format!(
                "failed to create sabnzbd nzb file {}: {error}",
                nzb_path.display()
            ))
        })?;

        let mut decoder = ZstdDecoder::new(BufReader::new(input));
        tokio::io::copy(&mut decoder, &mut output)
            .await
            .map_err(|error| {
                AppError::Repository(format!("sabnzbd nzb decompression failed: {error}"))
            })?;
        output
            .flush()
            .await
            .map_err(|error| AppError::Repository(format!("sabnzbd nzb flush failed: {error}")))?;

        let nzb_len = tokio::fs::metadata(&nzb_path)
            .await
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to stat sabnzbd nzb file {}: {error}",
                    nzb_path.display()
                ))
            })?
            .len();

        Ok((nzb_path, nzb_len))
    }

    fn api_urls(&self) -> Vec<String> {
        build_sab_api_urls(&self.base_url)
    }

    async fn api_get(&self, params: &[(&str, &str)]) -> AppResult<Value> {
        self.api_get_with_policy(params, self.read_policy("sabnzbd_api"))
            .await
    }

    async fn api_get_mutation(
        &self,
        params: &[(&str, &str)],
        request_label: &'static str,
    ) -> AppResult<Value> {
        self.api_get_with_policy(params, self.mutation_policy(request_label))
            .await
    }

    async fn api_get_with_policy(
        &self,
        params: &[(&str, &str)],
        policy: RequestPolicy,
    ) -> AppResult<Value> {
        let urls = self.api_urls();
        let request_mode = params
            .iter()
            .find_map(|(key, value)| (*key == "mode").then_some(*value));
        let mut form_or_query = vec![("output".to_string(), "json".to_string())];
        form_or_query.extend(
            params
                .iter()
                .map(|(key, value)| ((*key).to_string(), (*value).to_string())),
        );

        let auth = self.api_auth()?;
        let mut last_retryable_error = None;
        for (index, url) in urls.iter().enumerate() {
            let response = match &auth {
                SabApiAuth::ApiKey(api_key) => {
                    let mut form_or_query = form_or_query.clone();
                    form_or_query.push(("apikey".to_string(), api_key.clone()));
                    self.outbound_http
                        .send(policy.clone(), || {
                            self.outbound_http
                                .client()
                                .get(url)
                                .query(&form_or_query)
                                .timeout(SABNZBD_HTTP_REQUEST_TIMEOUT)
                        })
                        .await
                }
                SabApiAuth::Credentials { username, password } => {
                    let mut form_or_query = form_or_query.clone();
                    form_or_query.push(("ma_username".to_string(), username.clone()));
                    form_or_query.push(("ma_password".to_string(), password.clone()));
                    let encoded_form = url::form_urlencoded::Serializer::new(String::new())
                        .extend_pairs(
                            form_or_query
                                .iter()
                                .map(|(key, value)| (key.as_str(), value.as_str())),
                        )
                        .finish();
                    self.outbound_http
                        .send(policy.clone(), || {
                            self.outbound_http
                                .client()
                                .post(url)
                                .header("Content-Type", "application/x-www-form-urlencoded")
                                .body(encoded_form.clone())
                                .timeout(SABNZBD_HTTP_REQUEST_TIMEOUT)
                        })
                        .await
                }
            }
            .map_err(|error| map_sabnzbd_outbound_error("sabnzbd api call", error))?;

            let status = response.status();
            let body = response.text().await.map_err(|err| {
                AppError::Repository(format!("sabnzbd response read failed: {err}"))
            })?;

            match evaluate_sab_api_response("sabnzbd api", request_mode, status, &body) {
                SabApiResponseEvaluation::Success(json) => {
                    // Remember which API path this backend answers on so the
                    // addfile mutation can be pinned to it (set-if-empty).
                    let _ = self.resolved_api_url.set(url.clone());
                    return Ok(json);
                }
                SabApiResponseEvaluation::Retry(error) if index + 1 < urls.len() => {
                    debug!(
                        request_mode,
                        url,
                        error = %error,
                        "retrying sab-compatible endpoint with alternate api path"
                    );
                    last_retryable_error = Some(error);
                }
                SabApiResponseEvaluation::Retry(error)
                | SabApiResponseEvaluation::Failure(error) => {
                    return Err(error);
                }
            }
        }
        Err(last_retryable_error.unwrap_or_else(|| {
            AppError::Repository("sabnzbd api call did not return a usable response".to_string())
        }))
    }

    async fn history_slots_page(&self, start: usize, limit: usize) -> AppResult<Vec<Value>> {
        let start_param = start.to_string();
        let limit_param = limit.to_string();
        let json = self
            .api_get(&[
                ("mode", "history"),
                ("start", start_param.as_str()),
                ("limit", limit_param.as_str()),
            ])
            .await?;

        Ok(json
            .get("history")
            .and_then(slots_from_api_section)
            .or_else(|| json.get("slots").and_then(Value::as_array))
            .cloned()
            .unwrap_or_default())
    }

    async fn completed_downloads_page(&self, limit: usize) -> AppResult<Vec<CompletedDownload>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let slots = self.history_slots_page(0, limit).await?;
        Ok(completed_downloads_from_sab_slots(
            &slots,
            Some(Utc::now().timestamp() - (7 * 24 * 60 * 60)),
        ))
    }

    /// Look up one completed download by its SABnzbd job id.
    ///
    /// Tries SABnzbd's server-side `nzo_ids` history filter first: real SAB
    /// answers with just that row, which replaces what used to be an unbounded
    /// 50-at-a-time scan of the entire history for an id the server was willing
    /// to select. This runs once per stuck item per poll tick, so the scan cost
    /// grew with both retained history and the number of stranded downloads.
    ///
    /// The filter is an OPTIMIZATION, never a contract. SAB-compatible backends
    /// (altmount, nzbdav, older SAB) may ignore an unrecognized query parameter
    /// and return an ordinary unfiltered page — Sonarr, the de-facto
    /// compatibility oracle here, only ever sends start/limit/category. So the
    /// id is re-verified on the response and a miss falls back to the full
    /// paged scan; a backend without the filter behaves exactly as it did
    /// before, one cheap request later.
    async fn completed_download_for_source(
        &self,
        download_client_item_id: &str,
    ) -> AppResult<Option<CompletedDownload>> {
        const HISTORY_PAGE_SIZE: usize = 50;

        let targeted = self
            .api_get(&[
                ("mode", "history"),
                ("nzo_ids", download_client_item_id),
                ("start", "0"),
                ("limit", "1"),
            ])
            .await
            .ok()
            .map(|json| {
                json.get("history")
                    .and_then(slots_from_api_section)
                    .or_else(|| json.get("slots").and_then(Value::as_array))
                    .cloned()
                    .unwrap_or_default()
            })
            .and_then(|slots| {
                completed_downloads_from_sab_slots(&slots, None)
                    .into_iter()
                    .find(|download| download.download_client_item_id == download_client_item_id)
            });
        if targeted.is_some() {
            return Ok(targeted);
        }

        let mut start = 0;
        loop {
            let slots = self.history_slots_page(start, HISTORY_PAGE_SIZE).await?;
            if let Some(download) = completed_downloads_from_sab_slots(&slots, None)
                .into_iter()
                .find(|download| download.download_client_item_id == download_client_item_id)
            {
                return Ok(Some(download));
            }
            if slots.len() < HISTORY_PAGE_SIZE {
                return Ok(None);
            }
            start = start.checked_add(slots.len()).ok_or_else(|| {
                AppError::Repository("SABnzbd history offset overflow".to_string())
            })?;
        }
    }

    async fn get_config(&self) -> AppResult<SabnzbdConfig> {
        let json = self.api_get(&[("mode", "get_config")]).await?;
        serde_json::from_value::<SabnzbdConfigEnvelope>(json)
            .map(|response| response.config)
            .map_err(|error| {
                AppError::Repository(format!("sabnzbd config response parse failed: {error}"))
            })
    }

    async fn get_full_status(&self) -> AppResult<SabnzbdFullStatus> {
        let json = self
            .api_get(&[("mode", "fullstatus"), ("skip_dashboard", "1")])
            .await?;
        serde_json::from_value::<SabnzbdFullStatusEnvelope>(json)
            .map(|response| response.status)
            .map_err(|error| {
                AppError::Repository(format!("sabnzbd fullstatus response parse failed: {error}"))
            })
    }

    fn api_auth(&self) -> AppResult<SabApiAuth> {
        if let Some(api_key) = self.api_key.as_ref() {
            return Ok(SabApiAuth::ApiKey(api_key.clone()));
        }

        match (self.username.as_ref(), self.password.as_ref()) {
            (Some(username), Some(password)) => Ok(SabApiAuth::Credentials {
                username: username.clone(),
                password: password.clone(),
            }),
            _ => Err(AppError::Validation(
                "sabnzbd requires an API key or username/password".to_string(),
            )),
        }
    }

    fn api_auth_strategy_label(&self) -> &'static str {
        if self.api_key.is_some() {
            "api_key"
        } else if self.username.is_some() && self.password.is_some() {
            "credentials"
        } else {
            "missing"
        }
    }

    async fn post_addfile_request(
        &self,
        request: SabAddfileRequest<'_>,
    ) -> AppResult<(reqwest::StatusCode, String)> {
        let auth = self.api_auth()?;
        let upload_payload = request.upload_payload.clone();
        let upload_filename = request.upload_filename.to_string();
        let url = request.url.to_string();
        let nzb_name = request.nzb_name.to_string();
        let queue_priority = request.queue_priority.to_string();
        let upload_mime = request.upload_mime.to_string();
        let cat = request.cat.map(str::to_string);
        let password = request.password.map(str::to_string);
        let auth_strategy = self.api_auth_strategy_label();

        debug!(
            request_mode = "addfile",
            auth_strategy,
            has_category = cat.is_some(),
            has_password = password.is_some(),
            "building sabnzbd enqueue request"
        );

        let response = self
            .outbound_http
            .send_async(self.mutation_policy("sabnzbd_addfile"), move || {
                let auth = auth.clone();
                let url = url.clone();
                let nzb_name = nzb_name.clone();
                let queue_priority = queue_priority.clone();
                let upload_payload = upload_payload.clone();
                let upload_filename = upload_filename.clone();
                let upload_mime = upload_mime.clone();
                let cat = cat.clone();
                let password = password.clone();
                async move {
                    let nzb_part = match upload_payload {
                        SabAddfilePayload::File { path, len } => {
                            let upload_file = File::open(&path).await.map_err(|error| {
                                AppError::Repository(format!(
                                    "failed to reopen sabnzbd upload file {}: {error}",
                                    path.display()
                                ))
                            })?;
                            multipart::Part::stream_with_length(
                                reqwest::Body::wrap_stream(ReaderStream::new(upload_file)),
                                len,
                            )
                        }
                    }
                    .file_name(upload_filename)
                    .mime_str(&upload_mime)
                    .map_err(|err| {
                        AppError::Repository(format!("sabnzbd multipart build failed: {err}"))
                    })?;

                    let query_params = sab_addfile_query_params(
                        &auth,
                        &nzb_name,
                        &queue_priority,
                        cat.as_deref(),
                        password.as_deref(),
                    );
                    let form = multipart::Form::new().part(SAB_ADDFILE_UPLOAD_FIELD, nzb_part);
                    let request_builder =
                        self.outbound_http.client().post(&url).query(&query_params);

                    Ok::<_, AppError>(
                        request_builder
                            .multipart(form)
                            .timeout(SABNZBD_HTTP_REQUEST_TIMEOUT),
                    )
                }
            })
            .await
            .map_err(|error| map_sabnzbd_addfile_send_error("sabnzbd addfile call", error))?;

        let status = response.status();
        let body = response.text().await.map_err(|err| {
            // The request was fully sent; a failure while reading the response
            // body means we cannot tell whether SABnzbd enqueued the job.
            AppError::DownloadSubmitAmbiguous(format!(
                "sabnzbd addfile response body was lost after the upload was sent: {}",
                redact_sab_secret_values(&err.to_string())
            ))
        })?;

        Ok((status, body))
    }

    fn derive_sorting_mode(config: &SabnzbdConfig) -> Option<String> {
        if config
            .sorters
            .iter()
            .any(|sorter| sorter.is_active && !sorter.sort_cats.is_empty())
            || (config.misc.enable_tv_sorting && !config.misc.tv_categories.is_empty())
        {
            return Some("TV".to_string());
        }

        if config.misc.enable_movie_sorting && !config.misc.movie_categories.is_empty() {
            return Some("Movie".to_string());
        }

        if config.misc.enable_date_sorting && !config.misc.date_categories.is_empty() {
            return Some("Date".to_string());
        }

        None
    }

    fn removes_completed_downloads(config: &SabnzbdConfig) -> bool {
        match config.misc.history_retention_option.as_str() {
            "all" => false,
            "number-archive" | "number-delete" | "all-archive" | "all-delete" => true,
            "days-archive" | "days-delete" => config.misc.history_retention_number < 14,
            _ => {
                let retention = config.misc.history_retention.trim();
                if retention.is_empty() {
                    return false;
                }

                if let Some(days) = retention.strip_suffix('d') {
                    return days.parse::<i64>().unwrap_or(i64::MAX) < 14;
                }

                retention != "0"
            }
        }
    }

    fn output_roots_from_config(
        &self,
        config: &SabnzbdConfig,
        full_status: Option<&SabnzbdFullStatus>,
    ) -> Vec<String> {
        let complete_dir = resolved_complete_dir(&config.misc.complete_dir, full_status);
        let mut roots = Vec::new();

        if !complete_dir.is_empty() {
            roots.push(complete_dir.clone());
        }

        for category in &config.categories {
            let path = category_output_root(&complete_dir, &category.dir);
            if !path.is_empty() {
                roots.push(path);
            }
        }

        dedupe_strings(roots)
    }

    pub async fn test_connection(&self) -> AppResult<String> {
        let json = self
            .api_get_with_policy(
                &[("mode", "version")],
                self.read_policy("sabnzbd_test_connection"),
            )
            .await?;

        let version = json
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or("sabnzbd")
            .to_string();

        // Check version >= 3.0.0
        let mut warnings = Vec::new();
        let version_parts: Vec<u32> = version.split('.').filter_map(|p| p.parse().ok()).collect();
        if version_parts.len() >= 2 && version_parts[0] < 3 {
            warnings.push(format!(
                "SABnzbd {version} is outdated; version 3.0.0+ is recommended"
            ));
        }

        // Validate the configured auth mode by making an authenticated request.
        self.api_get(&[("mode", "queue"), ("limit", "0")])
            .await
            .map_err(map_sabnzbd_auth_validation_error)?;

        if warnings.is_empty() {
            Ok(version)
        } else {
            Ok(format!("{version} ({})", warnings.join("; ")))
        }
    }
}

#[async_trait]
impl DownloadClient for SabnzbdDownloadClient {
    async fn submit_download(
        &self,
        request: &DownloadClientAddRequest,
    ) -> AppResult<DownloadGrabResult> {
        let title = &request.title;
        let nzb_name = request
            .source_title
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or(title.name.as_str());

        let staged = resolve_staged_nzb_for_request(
            &self.outbound_http,
            &self.staged_nzb_store,
            &self.staged_nzb_pipeline_limit,
            request,
        )
        .await?;
        let mut transient_nzb_path: Option<PathBuf> = None;

        let result: AppResult<DownloadGrabResult> = async {
            let cat = request
                .category
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            let password = request
                .source_password
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty() && *value != "0")
                .map(str::to_string);
            let nzb_name_owned = nzb_name.to_string();
            let queue_priority =
                sabnzbd_queue_priority(request.queue_priority.as_deref()).to_string();
            let (nzb_path, nzb_len) = self
                .build_transient_nzb_artifact(&staged.staged_nzb)
                .await?;
            self.staged_nzb_store.mark_artifact_active(&nzb_path)?;
            transient_nzb_path = Some(nzb_path.clone());

            let plain_nzb_filename = if nzb_name.to_ascii_lowercase().ends_with(".nzb") {
                nzb_name.to_string()
            } else {
                format!("{nzb_name}.nzb")
            };

            // Discover (or reuse) the single API path this backend serves and
            // POST the addfile there ONLY. Probing is an idempotent GET, so it
            // is safe to try both candidate paths; the mutation is not, so it
            // never falls through — a landed-but-lost response is reconciled
            // instead of blindly re-POSTed to the alternate path.
            let addfile_url = self.resolve_addfile_url().await?;

            let (status, body) = match self
                .post_addfile_request(SabAddfileRequest {
                    url: &addfile_url,
                    nzb_name: &nzb_name_owned,
                    queue_priority: &queue_priority,
                    upload_payload: SabAddfilePayload::File {
                        path: nzb_path.clone(),
                        len: nzb_len,
                    },
                    upload_filename: &plain_nzb_filename,
                    upload_mime: "application/x-nzb",
                    cat: cat.as_deref(),
                    password: password.as_deref(),
                })
                .await
            {
                Ok(response) => response,
                Err(error) => {
                    debug!(
                        request_mode = "addfile",
                        auth_strategy = self.api_auth_strategy_label(),
                        title = title.name.as_str(),
                        error = %error,
                        "sabnzbd enqueue request failed before a usable response"
                    );
                    // An ambiguous transport failure may already have enqueued
                    // the job — reconcile before giving up. A connect/build
                    // failure (Unavailable) or a rate-limit refusal
                    // (TemporaryUnavailable) never reached SAB and is safe to
                    // defer as-is.
                    if error.is_download_submit_ambiguous() {
                        return self
                            .reconcile_ambiguous_sab_addfile(&nzb_name_owned, error)
                            .await;
                    }
                    return Err(error);
                }
            };

            match evaluate_sab_addfile_response(status, &body) {
                SabAddfileOutcome::Accepted(nzo_id) => {
                    debug!(
                        nzo_id = nzo_id.as_str(),
                        title = title.name.as_str(),
                        nzb_name = nzb_name,
                        "sabnzbd addfile succeeded"
                    );
                    Ok(DownloadGrabResult {
                        download_id: None,
                        job_id: nzo_id,
                        client_id: None,
                        client_type: "sabnzbd".to_string(),
                        info_hash: None,
                        seed_goals: None,
                    })
                }
                SabAddfileOutcome::Rejected(detail) => {
                    debug!(
                        title = title.name.as_str(),
                        nzb_name = nzb_name,
                        detail = detail.as_str(),
                        "sabnzbd rejected the enqueue request"
                    );
                    Err(AppError::DownloadSubmitRejected(detail))
                }
                SabAddfileOutcome::Auth(detail) => {
                    Err(AppError::download_submit_unavailable(detail))
                }
                SabAddfileOutcome::Ambiguous(error) => {
                    self.reconcile_ambiguous_sab_addfile(&nzb_name_owned, error)
                        .await
                }
            }
        }
        .await;

        if let Some(nzb_path) = transient_nzb_path {
            if let Err(error) = self.staged_nzb_store.mark_artifact_inactive(&nzb_path) {
                warn!(
                    path = %nzb_path.display(),
                    error = %error,
                    "failed to mark transient sabnzbd nzb artifact inactive"
                );
            }
            if let Err(error) = tokio::fs::remove_file(&nzb_path).await
                && error.kind() != std::io::ErrorKind::NotFound
            {
                warn!(
                    path = %nzb_path.display(),
                    error = %error,
                    "failed to delete transient sabnzbd nzb artifact"
                );
            }
        }

        if staged.self_staged
            && let Err(error) = self
                .staged_nzb_store
                .delete_staged_nzb(&staged.staged_nzb)
                .await
        {
            warn!(
                staged_nzb_id = staged.staged_nzb.id.as_str(),
                error = %error,
                "failed to delete self-staged sabnzbd nzb artifact"
            );
        }

        result
    }

    async fn test_connection(&self) -> AppResult<String> {
        SabnzbdDownloadClient::test_connection(self).await
    }

    async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
        let json = self.api_get(&[("mode", "queue")]).await?;

        let slots = json
            .get("queue")
            .and_then(slots_from_api_section)
            .or_else(|| json.get("slots").and_then(Value::as_array));

        let slots = match slots {
            Some(s) => s,
            None => return Ok(Vec::new()),
        };

        Ok(slots
            .iter()
            .filter_map(|slot| {
                let slot = slot.as_object()?;

                let nzo_id = slot.get("nzo_id").and_then(Value::as_str)?.to_string();

                let raw_filename = slot
                    .get("filename")
                    .and_then(Value::as_str)
                    .unwrap_or("Unnamed download");
                let (title_name, is_encrypted) =
                    if let Some(stripped) = raw_filename.strip_prefix("ENCRYPTED / ") {
                        (stripped.to_string(), true)
                    } else {
                        (raw_filename.to_string(), false)
                    };

                let status = slot.get("status").and_then(Value::as_str).unwrap_or("");
                let state = sabnzbd_queue_state(status)?;

                let percentage = slot
                    .get("percentage")
                    .and_then(|v| v.as_str().or_else(|| v.as_u64().map(|_| "")))
                    .and_then(|s| {
                        if s.is_empty() {
                            slot.get("percentage")
                                .and_then(Value::as_u64)
                                .map(|v| v as u8)
                        } else {
                            s.parse::<u8>().ok()
                        }
                    })
                    .unwrap_or(0);

                let size_bytes = extract_f64_value(slot.get("mb")).map(|mb| {
                    if !mb.is_finite() || mb <= 0.0 {
                        0
                    } else {
                        (mb * 1_048_576f64).round() as i64
                    }
                });

                let remaining_seconds = slot
                    .get("timeleft")
                    .and_then(Value::as_str)
                    .and_then(parse_duration_seconds);

                let pp_status = if state == DownloadQueueState::Downloading {
                    sabnzbd_postprocessing_stage(status)
                } else {
                    None
                };

                let attention_required = is_encrypted;
                let attention_reason = if is_encrypted {
                    Some("ENCRYPTED".to_string())
                } else {
                    pp_status
                };
                let category = extract_sabnzbd_category(slot);

                Some(DownloadQueueItem {
                    id: nzo_id.clone(),
                    title_id: None,
                    episode_id: None,
                    title_name,
                    facet: None,
                    category,
                    client_id: String::new(),
                    client_name: String::new(),
                    client_type: "sabnzbd".to_string(),
                    state,
                    progress_percent: percentage,
                    import_transfer_phase: None,
                    import_transfer_bytes: None,
                    import_transfer_total_bytes: None,
                    import_transfer_started_at: None,
                    import_transfer_updated_at: None,
                    size_bytes,
                    remaining_seconds,
                    queued_at: None,
                    last_updated_at: None,
                    attention_required,
                    attention_reason,
                    download_client_item_id: nzo_id.clone(),
                    download_id: Some(nzo_id),
                    import_status: None,
                    import_error_code: None,
                    import_error_message: None,
                    imported_at: None,
                    delete_status: None,
                    delete_error_message: None,
                    source_provider: None,
                    is_scryer_origin: false,
                    tracked_state: None,
                    tracked_status: None,
                    tracked_status_messages: Vec::new(),
                    tracked_match_type: None,
                    seeding: None,
                })
            })
            .collect())
    }

    async fn list_history(&self) -> AppResult<Vec<DownloadQueueItem>> {
        let slots = self.history_slots_page(0, 50).await?;
        let cutoff_ts = Utc::now().timestamp() - (7 * 24 * 60 * 60);

        Ok(slots
            .iter()
            .filter_map(|slot| {
                let slot = slot.as_object()?;

                let nzo_id = slot.get("nzo_id").and_then(Value::as_str)?.to_string();

                let completed_ts = extract_i64_value(slot.get("completed"));
                if let Some(ts) = completed_ts
                    && ts < cutoff_ts
                {
                    return None;
                }

                let title_name = slot
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("Unnamed download")
                    .to_string();

                let status = slot.get("status").and_then(Value::as_str).unwrap_or("");
                let fail_message = slot.get("fail_message").and_then(Value::as_str);
                let (state, attention_reason) = sabnzbd_history_state(status, fail_message)?;
                let category = extract_sabnzbd_category(slot);

                Some(DownloadQueueItem {
                    id: nzo_id.clone(),
                    title_id: None,
                    episode_id: None,
                    title_name,
                    facet: None,
                    category,
                    client_id: String::new(),
                    client_name: String::new(),
                    client_type: "sabnzbd".to_string(),
                    state,
                    progress_percent: if state == DownloadQueueState::Completed {
                        100
                    } else {
                        0
                    },
                    import_transfer_phase: None,
                    import_transfer_bytes: None,
                    import_transfer_total_bytes: None,
                    import_transfer_started_at: None,
                    import_transfer_updated_at: None,
                    size_bytes: extract_i64_value(slot.get("bytes")),
                    remaining_seconds: None,
                    queued_at: extract_i64_value(slot.get("time_added")).map(|v| v.to_string()),
                    last_updated_at: completed_ts.map(|v| v.to_string()),
                    attention_required: matches!(state, DownloadQueueState::Failed),
                    attention_reason,
                    download_client_item_id: nzo_id.clone(),
                    download_id: Some(nzo_id),
                    import_status: None,
                    import_error_code: None,
                    import_error_message: None,
                    imported_at: None,
                    delete_status: None,
                    delete_error_message: None,
                    source_provider: None,
                    is_scryer_origin: false,
                    tracked_state: None,
                    tracked_status: None,
                    tracked_status_messages: Vec::new(),
                    tracked_match_type: None,
                    seeding: None,
                })
            })
            .collect())
    }

    async fn list_history_page(
        &self,
        offset: usize,
        limit: usize,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        let slots = self.history_slots_page(offset, limit).await?;
        let cutoff_ts = Utc::now().timestamp() - (7 * 24 * 60 * 60);

        Ok(slots
            .iter()
            .filter_map(|slot| {
                let slot = slot.as_object()?;

                let nzo_id = slot.get("nzo_id").and_then(Value::as_str)?.to_string();

                let completed_ts = extract_i64_value(slot.get("completed"));
                if let Some(ts) = completed_ts
                    && ts < cutoff_ts
                {
                    return None;
                }

                let title_name = slot
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("Unnamed download")
                    .to_string();

                let status = slot.get("status").and_then(Value::as_str).unwrap_or("");
                let fail_message = slot.get("fail_message").and_then(Value::as_str);
                let (state, attention_reason) = sabnzbd_history_state(status, fail_message)?;
                let category = extract_sabnzbd_category(slot);

                Some(DownloadQueueItem {
                    id: nzo_id.clone(),
                    title_id: None,
                    episode_id: None,
                    title_name,
                    facet: None,
                    category,
                    client_id: String::new(),
                    client_name: String::new(),
                    client_type: "sabnzbd".to_string(),
                    state,
                    progress_percent: if state == DownloadQueueState::Completed {
                        100
                    } else {
                        0
                    },
                    import_transfer_phase: None,
                    import_transfer_bytes: None,
                    import_transfer_total_bytes: None,
                    import_transfer_started_at: None,
                    import_transfer_updated_at: None,
                    size_bytes: extract_i64_value(slot.get("bytes")),
                    remaining_seconds: None,
                    queued_at: extract_i64_value(slot.get("time_added"))
                        .map(|value| value.to_string()),
                    last_updated_at: completed_ts.map(|value| value.to_string()),
                    attention_required: matches!(state, DownloadQueueState::Failed),
                    attention_reason,
                    download_client_item_id: nzo_id.clone(),
                    download_id: Some(nzo_id),
                    import_status: None,
                    import_error_code: None,
                    import_error_message: None,
                    imported_at: None,
                    delete_status: None,
                    delete_error_message: None,
                    source_provider: None,
                    is_scryer_origin: false,
                    tracked_state: None,
                    tracked_status: None,
                    tracked_status_messages: Vec::new(),
                    tracked_match_type: None,
                    seeding: None,
                })
            })
            .collect())
    }

    async fn list_completed_downloads(&self) -> AppResult<Vec<CompletedDownload>> {
        self.completed_downloads_page(50).await
    }

    async fn list_recent_completed_downloads(
        &self,
        limit: usize,
    ) -> AppResult<Vec<CompletedDownload>> {
        self.completed_downloads_page(limit).await
    }

    async fn get_completed_download_for_source(
        &self,
        _client_id: &str,
        _client_type: &str,
        download_client_item_id: &str,
    ) -> AppResult<Option<CompletedDownload>> {
        let download_client_item_id = download_client_item_id.trim();
        if download_client_item_id.is_empty() {
            return Ok(None);
        }
        self.completed_download_for_source(download_client_item_id)
            .await
    }

    async fn get_client_status(&self) -> AppResult<DownloadClientStatus> {
        let config = self.get_config().await?;
        let full_status = self.get_full_status().await.ok();

        Ok(DownloadClientStatus {
            version: None,
            is_localhost: is_localhost_base_url(&self.base_url),
            remote_output_roots: self.output_roots_from_config(&config, full_status.as_ref()),
            removes_completed_downloads: Some(Self::removes_completed_downloads(&config)),
            sorting_mode: Self::derive_sorting_mode(&config),
            warnings: Vec::new(),
        })
    }

    async fn pause_queue_item(&self, id: &str) -> AppResult<()> {
        self.api_get_mutation(
            &[("mode", "queue"), ("name", "pause"), ("value", id)],
            "sabnzbd_pause_queue_item",
        )
        .await?;
        Ok(())
    }

    async fn resume_queue_item(&self, id: &str) -> AppResult<()> {
        self.api_get_mutation(
            &[("mode", "queue"), ("name", "resume"), ("value", id)],
            "sabnzbd_resume_queue_item",
        )
        .await?;
        Ok(())
    }

    /// A history delete includes `del_files=1` only when `remove_data` is
    /// requested. Queue deletes continue to include `del_files=1` regardless.
    ///
    /// `is_history` is only a hint: the caller derives it from the last polled
    /// state, and SAB moves a job into history the moment post-processing
    /// starts. A queue delete for an id that only lives in history answers
    /// `{"status": false, "nzo_ids": []}` — nothing removed, no error text
    /// (`sabnzbd/api.py::_api_queue_delete`) — and a history delete answers a
    /// bare `{"status": true}` whether or not the id existed
    /// (`_api_history_delete`). So a wrong hint in either direction has to
    /// fall through to the other mode or the row silently comes back on the
    /// next poll.
    async fn delete_queue_item(
        &self,
        id: &str,
        is_history: bool,
        remove_data: bool,
    ) -> AppResult<()> {
        let response = self.send_delete(id, is_history, remove_data).await?;

        match sab_delete_removed_hinted_id(&response, id) {
            // The backend reported which ids it removed and ours was there.
            Some(true) => Ok(()),
            // The backend reported its removals and ours was not among them,
            // so the item lives in the other list.
            Some(false) => {
                debug!(
                    nzo_id = id,
                    hinted_history = is_history,
                    "sabnzbd delete removed nothing; retrying in the other mode"
                );
                self.send_delete(id, !is_history, remove_data).await?;
                Ok(())
            }
            // No `nzo_ids` at all: real SAB history deletes never report them,
            // and SAB-compatible backends (altmount, nzbdav, decypharr) report
            // them for neither mode. Probe instead of guessing.
            None => match self.queue_contains_nzo_id(id).await {
                // `in_queue == is_history` is the contradiction: still queued
                // after a history-hinted delete, or already gone from the
                // queue after a queue-hinted delete that reported nothing.
                Ok(in_queue) if in_queue == is_history => {
                    debug!(
                        nzo_id = id,
                        hinted_history = is_history,
                        in_queue,
                        "sabnzbd queue probe contradicts the delete hint; retrying in the other mode"
                    );
                    // The probe cannot tell "removed by the hinted delete"
                    // from "never in that list", so this second delete is
                    // best effort: backends that answer an unknown id with an
                    // error (decypharr) must not turn a delete that already
                    // landed into a failure.
                    if let Err(error) = self.send_delete(id, !is_history, remove_data).await {
                        debug!(
                            nzo_id = id,
                            hinted_history = is_history,
                            error = %error,
                            "sabnzbd fallback delete after queue probe failed; keeping the hinted delete"
                        );
                    }
                    Ok(())
                }
                Ok(_) => Ok(()),
                Err(error) => {
                    // The hinted delete already went through; a probe that
                    // cannot run is no reason to fail the removal.
                    debug!(
                        nzo_id = id,
                        hinted_history = is_history,
                        error = %error,
                        "sabnzbd queue probe failed; keeping the hinted delete"
                    );
                    Ok(())
                }
            },
        }
    }
}

impl SabnzbdDownloadClient {
    /// Issue one delete in the requested mode and hand back the raw response so
    /// the caller can tell whether anything was actually removed.
    async fn send_delete(
        &self,
        id: &str,
        is_history: bool,
        remove_data: bool,
    ) -> AppResult<Value> {
        if is_history {
            let mut params = vec![("mode", "history"), ("name", "delete"), ("value", id)];
            if remove_data {
                params.push(("del_files", "1"));
            }
            self.api_get_mutation(&params, "sabnzbd_delete_history_item")
                .await
        } else {
            self.api_get_mutation(
                &[
                    ("mode", "queue"),
                    ("name", "delete"),
                    ("value", id),
                    ("del_files", "1"),
                ],
                "sabnzbd_delete_queue_item",
            )
            .await
        }
    }

    /// Whether `id` is still in the active queue.
    ///
    /// Reads the same `mode=queue` listing `list_queue` uses and matches
    /// client-side. Asking SAB to filter by `nzo_ids` would be cheaper but
    /// SAB-compatible backends ignore unknown filters and answer with the whole
    /// queue, which a server-side check would read as a match for anything.
    async fn queue_contains_nzo_id(&self, id: &str) -> AppResult<bool> {
        let json = self.api_get(&[("mode", "queue")]).await?;
        let slots = json
            .get("queue")
            .and_then(slots_from_api_section)
            .or_else(|| json.get("slots").and_then(Value::as_array));

        Ok(slots.is_some_and(|slots| {
            slots
                .iter()
                .any(|slot| slot.get("nzo_id").and_then(Value::as_str) == Some(id))
        }))
    }

    fn read_policy(&self, request_label: &'static str) -> RequestPolicy {
        RequestPolicy::safe_read(format!("sabnzbd:{}", self.base_url), request_label)
            .with_max_retries(2)
            .with_backoff(
                std::time::Duration::from_secs(1),
                std::time::Duration::from_secs(15),
            )
    }

    fn mutation_policy(&self, request_label: &'static str) -> RequestPolicy {
        RequestPolicy::no_retry(format!("sabnzbd:{}", self.base_url), request_label).with_backoff(
            std::time::Duration::from_secs(1),
            std::time::Duration::from_secs(15),
        )
    }

    /// Resolve the SAB API path to send the addfile POST to.
    ///
    /// Returns the cached path if a prior read already discovered it. Otherwise
    /// probes with an idempotent GET (`mode=queue&limit=0`, exactly what
    /// `test_connection` issues), which caches the winning path as a side
    /// effect via `api_get`. The probe is idempotent, so trying both candidate
    /// paths cannot create a duplicate. If the probe can't complete (backend
    /// momentarily unreachable), fall back to the first candidate path rather
    /// than failing the submit — the POST classification and reconciliation
    /// still guard against duplicates, and an unresolved backend simply defers.
    async fn resolve_addfile_url(&self) -> AppResult<String> {
        if let Some(url) = self.resolved_api_url.get() {
            return Ok(url.clone());
        }

        let _ = self.api_get(&[("mode", "queue"), ("limit", "0")]).await;

        Ok(self
            .resolved_api_url
            .get()
            .cloned()
            .unwrap_or_else(|| self.api_urls().into_iter().next().unwrap_or_default()))
    }

    /// After an ambiguous addfile outcome, poll the queue then the history for
    /// a job whose name matches the release we uploaded. Ports NZBGet's
    /// `reconcile_append_after_transport_error` to SAB's server-side search
    /// (`mode=queue&search=...` / `mode=history&search=...`). Returns the
    /// adopted `nzo_id` on the first match, or `None` after the ladder is
    /// exhausted.
    async fn reconcile_addfile_after_ambiguous(&self, nzb_name: &str) -> Option<String> {
        const RECONCILE_DELAYS: [std::time::Duration; 5] = [
            std::time::Duration::from_millis(0),
            std::time::Duration::from_millis(100),
            std::time::Duration::from_millis(250),
            std::time::Duration::from_millis(500),
            std::time::Duration::from_millis(1000),
        ];

        let expected = normalize_sab_job_name(nzb_name);
        warn!(
            nzb_name,
            "sabnzbd addfile response was ambiguous; reconciling queue and history before returning failure"
        );

        for delay in RECONCILE_DELAYS {
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }

            // Check the queue first — an active copy is the most relevant
            // landing spot and minimizes adopting a stale same-name history
            // entry.
            //
            // We do NOT pass SAB's `search` param: it substring-matches the
            // sanitized `final_name`, whereas `nzb_name` is the raw release
            // title. A title with SAB-illegal characters (`/ : " * ? < > |`)
            // would never match server-side, the job would be missed, and the
            // next cycle would re-submit into a duplicate — exactly what this
            // reconciliation exists to prevent. Fetch the (small) queue
            // unfiltered and match client-side via `normalize_sab_job_name`.
            match self.api_get(&[("mode", "queue"), ("limit", "0")]).await {
                Ok(json) => {
                    let matched = json
                        .get("queue")
                        .and_then(slots_from_api_section)
                        .or_else(|| json.get("slots").and_then(Value::as_array))
                        .and_then(|slots| {
                            slots.iter().find_map(|slot| {
                                sab_reconcile_slot_nzo_id(slot, "filename", &expected)
                            })
                        });
                    if let Some(nzo_id) = matched {
                        info!(
                            nzo_id = nzo_id.as_str(),
                            nzb_name, "reconciled ambiguous sabnzbd addfile from queue"
                        );
                        return Some(nzo_id);
                    }
                }
                Err(error) => {
                    warn!(
                        error = %error,
                        nzb_name,
                        "failed to read sabnzbd queue while reconciling ambiguous addfile"
                    );
                }
            }

            // History is a legitimate landing spot too (reject-to-history and
            // fast-failing jobs, per SAB's process_single_nzb); the existing
            // failure-detection flow handles a failed state normally. Same
            // reason as the queue probe above: no server-side `search`, match
            // client-side. `limit=50` bounds the recent window (the just-added
            // job is always recent).
            match self
                .api_get(&[("mode", "history"), ("start", "0"), ("limit", "50")])
                .await
            {
                Ok(json) => {
                    let matched = json
                        .get("history")
                        .and_then(slots_from_api_section)
                        .or_else(|| json.get("slots").and_then(Value::as_array))
                        .and_then(|slots| {
                            slots
                                .iter()
                                .find_map(|slot| sab_reconcile_slot_nzo_id(slot, "name", &expected))
                        });
                    if let Some(nzo_id) = matched {
                        info!(
                            nzo_id = nzo_id.as_str(),
                            nzb_name, "reconciled ambiguous sabnzbd addfile from history"
                        );
                        return Some(nzo_id);
                    }
                }
                Err(error) => {
                    warn!(
                        error = %error,
                        nzb_name,
                        "failed to read sabnzbd history while reconciling ambiguous addfile"
                    );
                }
            }
        }

        warn!(
            nzb_name,
            "ambiguous sabnzbd addfile was not found in queue or history"
        );
        None
    }

    /// Reconcile an ambiguous addfile outcome: adopt a matching queue/history
    /// job if one is found, otherwise surface the ambiguous error so the
    /// orchestration layer defers without blocklisting or failing over.
    async fn reconcile_ambiguous_sab_addfile(
        &self,
        nzb_name: &str,
        ambiguous_error: AppError,
    ) -> AppResult<DownloadGrabResult> {
        match self.reconcile_addfile_after_ambiguous(nzb_name).await {
            Some(nzo_id) => Ok(DownloadGrabResult {
                download_id: None,
                job_id: nzo_id,
                client_id: None,
                client_type: "sabnzbd".to_string(),
                info_hash: None,
                seed_goals: None,
            }),
            None => Err(ambiguous_error),
        }
    }
}

fn map_sabnzbd_outbound_error(operation: &str, error: OutboundHttpError) -> AppError {
    match error {
        OutboundHttpError::RateLimited(rate_limited) => {
            let retry_after = rate_limited.retry_after.filter(|delay| !delay.is_zero());
            AppError::rate_limited_temporary_unavailable(
                match retry_after {
                    Some(delay) => {
                        format!(
                            "{operation} was rate limited; retry after {}s",
                            delay.as_secs()
                        )
                    }
                    None => format!("{operation} was rate limited"),
                },
                retry_after,
                RateLimitCooldownAction::AlreadyRecorded,
            )
        }
        OutboundHttpError::Transport { source, .. } => AppError::Repository(format!(
            "{operation} failed: {}",
            redact_sab_secret_values(&source.to_string())
        )),
    }
}

/// Classify a transport-level failure of the addfile POST.
///
/// The mutation policy is `no_retry`, so there is exactly one POST attempt per
/// URL. A failure that provably happened before the request left the client
/// (request build failure or connect/DNS failure) enqueued nothing and is a
/// plain "unavailable, retry later". A rate-limiter refusal likewise never sent
/// the request. Any other transport failure (timeout, connection reset, body or
/// decode error) may have reached SAB after the upload streamed, so it is
/// ambiguous and must be reconciled — SABnzbd's addfile response carries no
/// idempotency key, so a blind re-POST would risk a duplicate job.
fn map_sabnzbd_addfile_send_error(
    operation: &str,
    error: scryer_outbound_http::OutboundRequestError<AppError>,
) -> AppError {
    match error {
        scryer_outbound_http::OutboundRequestError::Build(error) => {
            error.into_download_submit_unavailable()
        }
        scryer_outbound_http::OutboundRequestError::Http(OutboundHttpError::RateLimited(
            rate_limited,
        )) => map_sabnzbd_outbound_error(operation, OutboundHttpError::RateLimited(rate_limited)),
        scryer_outbound_http::OutboundRequestError::Http(OutboundHttpError::Transport {
            attempts,
            source,
            ..
        }) => {
            let redacted = redact_sab_secret_values(&source.to_string());
            if source.is_connect() {
                AppError::download_submit_unavailable(format!(
                    "{operation} could not connect to sabnzbd: {redacted}"
                ))
            } else {
                warn!(
                    attempts,
                    error = %source,
                    is_timeout = source.is_timeout(),
                    is_connect = source.is_connect(),
                    is_request = source.is_request(),
                    is_body = source.is_body(),
                    is_decode = source.is_decode(),
                    "sabnzbd addfile transport failed after sending the enqueue request"
                );
                AppError::DownloadSubmitAmbiguous(format!(
                    "sabnzbd addfile response was lost after the upload was sent: {redacted}"
                ))
            }
        }
    }
}

fn sab_addfile_query_params(
    auth: &SabApiAuth,
    nzb_name: &str,
    queue_priority: &str,
    cat: Option<&str>,
    password: Option<&str>,
) -> Vec<(&'static str, String)> {
    let mut fields = vec![
        ("mode", "addfile".to_string()),
        ("output", "json".to_string()),
        ("nzbname", nzb_name.to_string()),
        ("priority", queue_priority.to_string()),
    ];
    match auth {
        SabApiAuth::ApiKey(api_key) => {
            fields.push(("apikey", api_key.clone()));
        }
        SabApiAuth::Credentials { username, password } => {
            fields.push(("ma_username", username.clone()));
            fields.push(("ma_password", password.clone()));
        }
    }
    if let Some(cat) = cat {
        fields.push(("cat", cat.to_string()));
    }
    if let Some(password) = password {
        fields.push(("password", password.to_string()));
    }
    fields
}

fn redact_sab_secret_values(message: &str) -> String {
    SAB_SECRET_QUERY_KEYS
        .iter()
        .fold(message.to_string(), |redacted, key| {
            redact_key_value(&redacted, key)
        })
}

fn redact_key_value(input: &str, key: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let needle = format!("{key}=");
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0;
    let mut search_from = 0;

    while let Some(relative_start) = lower[search_from..].find(&needle) {
        let start = search_from + relative_start;
        let value_start = start + needle.len();
        output.push_str(&input[cursor..value_start]);
        output.push_str("<redacted>");

        let value_end = input[value_start..]
            .char_indices()
            .find(|(_, ch)| {
                matches!(
                    ch,
                    '&' | '#' | ' ' | '\t' | '\r' | '\n' | '\'' | '"' | ')' | '(' | '<' | '>'
                )
            })
            .map(|(idx, _)| value_start + idx)
            .unwrap_or(input.len());
        cursor = value_end;
        search_from = value_end;
    }

    output.push_str(&input[cursor..]);
    output
}

fn map_sabnzbd_auth_validation_error(error: AppError) -> AppError {
    match error {
        AppError::Repository(message) => AppError::Repository(format!(
            "sabnzbd authentication validation failed: {message}"
        )),
        AppError::Validation(message) => AppError::Repository(format!(
            "sabnzbd authentication validation failed: {message}"
        )),
        other => AppError::Repository(format!("sabnzbd authentication validation failed: {other}")),
    }
}

fn map_sabnzbd_response_error(operation: &str, status: StatusCode, body: &str) -> AppError {
    let detail = extract_sab_error_detail(body);
    if status == StatusCode::UNAUTHORIZED
        || status == StatusCode::FORBIDDEN
        || is_sab_auth_error_message(&detail)
    {
        return AppError::Repository(format!("sabnzbd authentication failed: {detail}"));
    }

    AppError::Repository(format!("{operation} returned status {status}: {detail}"))
}

fn map_sabnzbd_api_error(operation: &str, status: Option<StatusCode>, detail: &str) -> AppError {
    if status
        .is_some_and(|value| value == StatusCode::UNAUTHORIZED || value == StatusCode::FORBIDDEN)
        || is_sab_auth_error_message(detail)
    {
        return AppError::Repository(format!("sabnzbd authentication failed: {detail}"));
    }

    AppError::Repository(format!("{operation} error: {detail}"))
}

enum SabApiResponseEvaluation {
    Success(Value),
    Retry(AppError),
    Failure(AppError),
}

fn evaluate_sab_api_response(
    operation: &str,
    request_mode: Option<&str>,
    status: StatusCode,
    body: &str,
) -> SabApiResponseEvaluation {
    if !status.is_success() {
        let detail = extract_sab_error_detail(body);
        let error = map_sabnzbd_response_error(operation, status, body);
        return if status == StatusCode::UNAUTHORIZED
            || status == StatusCode::FORBIDDEN
            || is_sab_auth_error_message(&detail)
        {
            SabApiResponseEvaluation::Failure(error)
        } else {
            SabApiResponseEvaluation::Retry(error)
        };
    }

    let json: Value = match serde_json::from_str(body) {
        Ok(json) => json,
        Err(err) => {
            return SabApiResponseEvaluation::Retry(AppError::Repository(format!(
                "{operation} returned non-json response: {err}"
            )));
        }
    };

    if !sab_api_mode_matches_response(request_mode, &json) {
        return SabApiResponseEvaluation::Retry(AppError::Repository(format!(
            "{operation} returned unexpected response shape for mode '{}'",
            request_mode.unwrap_or("unknown")
        )));
    }

    if sab_api_status_is_false(&json) && !sab_api_reports_empty_removal(&json) {
        let error_msg = sab_api_error_message(&json).unwrap_or("unknown error");
        return SabApiResponseEvaluation::Failure(map_sabnzbd_api_error(
            operation,
            Some(status),
            error_msg,
        ));
    }

    SabApiResponseEvaluation::Success(json)
}

/// SAB's queue delete answers `{"status": bool(removed), "nzo_ids": removed}`
/// (`sabnzbd/api.py::_api_queue_delete`), so a delete that matched nothing is
/// `status: false` with an empty `nzo_ids` and no error text. That is an
/// answer, not a failure: the caller reads `nzo_ids` to learn nothing was
/// removed and tries the other list.
fn sab_api_reports_empty_removal(json: &Value) -> bool {
    sab_api_error_message(json).is_none() && json.get("nzo_ids").is_some_and(Value::is_array)
}

/// Classification of a fully-received SABnzbd addfile HTTP response.
///
/// The addfile POST is sent to a single, already-resolved API path (see
/// [`SabnzbdDownloadClient::resolve_addfile_url`]), so — unlike the shared
/// [`SabApiResponseEvaluation`] used for idempotent GET reads — it never falls
/// through to an alternate path. Any non-definitive response is therefore
/// ambiguous and must be reconciled rather than re-POSTed, since a blind
/// re-POST would risk a duplicate job (SAB's addfile carries no idempotency
/// key).
#[derive(Debug)]
enum SabAddfileOutcome {
    /// `status:true` with an nzo_id — the job is in the queue.
    Accepted(String),
    /// Definitive rejection of this NZB (status:false, empty nzo_ids). Never
    /// retried against this client; the release is blocklisted downstream.
    Rejected(String),
    /// Authentication failure — a configuration problem, nothing was enqueued.
    Auth(String),
    /// The POST was fully sent but the outcome is unknown (any non-success
    /// status, non-JSON body, or unexpected shape). Reconcile against the
    /// queue/history before failing.
    Ambiguous(AppError),
}

fn evaluate_sab_addfile_response(status: StatusCode, body: &str) -> SabAddfileOutcome {
    const OPERATION: &str = "sabnzbd addfile";

    if !status.is_success() {
        let detail = extract_sab_error_detail(body);
        if status == StatusCode::UNAUTHORIZED
            || status == StatusCode::FORBIDDEN
            || is_sab_auth_error_message(&detail)
        {
            return SabAddfileOutcome::Auth(format!("sabnzbd authentication failed: {detail}"));
        }
        // The request was sent to the resolved API path but returned a
        // non-success status; the job may or may not have been enqueued, so
        // reconcile rather than guessing.
        return SabAddfileOutcome::Ambiguous(AppError::DownloadSubmitAmbiguous(format!(
            "{OPERATION} returned status {status} after the upload was sent: {detail}"
        )));
    }

    let json: Value = match serde_json::from_str(body) {
        Ok(json) => json,
        Err(err) => {
            return SabAddfileOutcome::Ambiguous(AppError::DownloadSubmitAmbiguous(format!(
                "{OPERATION} returned a non-json response after the upload was sent: {err}"
            )));
        }
    };

    if !sab_api_mode_matches_response(Some("addfile"), &json) {
        return SabAddfileOutcome::Ambiguous(AppError::DownloadSubmitAmbiguous(format!(
            "{OPERATION} returned an unexpected response shape after the upload was sent"
        )));
    }

    if sab_api_status_is_false(&json) {
        let detail = sab_api_error_message(&json).unwrap_or("unknown error");
        if is_sab_auth_error_message(detail) {
            return SabAddfileOutcome::Auth(format!("sabnzbd authentication failed: {detail}"));
        }
        return SabAddfileOutcome::Rejected(format!("sabnzbd rejected the nzb: {detail}"));
    }

    match sab_addfile_nzo_id(&json) {
        Some(nzo_id) => SabAddfileOutcome::Accepted(nzo_id.to_string()),
        None => SabAddfileOutcome::Rejected(
            "sabnzbd accepted the request but returned no nzo_id".to_string(),
        ),
    }
}

fn build_sab_api_urls(base_url: &str) -> Vec<String> {
    dedupe_strings(vec![
        build_sab_api_url_with_suffix(base_url, &["api"]),
        build_sab_api_url_with_suffix(base_url, &["sabnzbd", "api"]),
    ])
}

fn build_sab_api_url_with_suffix(base_url: &str, suffix: &[&str]) -> String {
    let fallback = || format!("{}/api", base_url.trim_end_matches('/'));
    let Ok(mut url) = Url::parse(base_url) else {
        return fallback();
    };

    let mut path_segments = url
        .path()
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let suffix_segments = suffix
        .iter()
        .copied()
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if path_segments
        .as_slice()
        .ends_with(suffix_segments.as_slice())
    {
        // Already normalized.
    } else {
        path_segments.extend(suffix_segments);
    }
    let normalized_path = if path_segments.is_empty() {
        "/api".to_string()
    } else {
        format!("/{}", path_segments.join("/"))
    };
    url.set_path(&normalized_path);
    url.set_query(None);
    url.set_fragment(None);
    url.to_string().trim_end_matches('/').to_string()
}

fn sab_api_mode_matches_response(request_mode: Option<&str>, json: &Value) -> bool {
    match request_mode {
        Some("version") => json.get("version").is_some() || sab_api_status_is_false(json),
        Some("queue") => {
            json.get("queue").is_some()
                || json.get("slots").is_some()
                || sab_api_status_is_true(json)
                || sab_api_status_is_false(json)
        }
        Some("history") => {
            json.get("history").is_some()
                || json.get("slots").is_some()
                || sab_api_status_is_true(json)
                || sab_api_status_is_false(json)
        }
        Some("get_config") => json.get("config").is_some() || sab_api_status_is_false(json),
        Some("fullstatus") => json.get("status").is_some() || sab_api_status_is_false(json),
        Some("addfile") => {
            json.get("nzo_ids").is_some()
                || json.get("status").is_some()
                || sab_api_status_is_false(json)
        }
        _ => true,
    }
}

/// Whether a delete response says it removed `id`.
///
/// SAB reports the ids a queue delete actually removed in `nzo_ids`, so an
/// empty or non-matching array is a delete that succeeded without removing
/// anything. `None` means the backend reported no `nzo_ids` field at all —
/// real SAB history deletes and some SAB-compatible backends — in which case
/// the response says nothing either way and only a probe can tell.
fn sab_delete_removed_hinted_id(response: &Value, id: &str) -> Option<bool> {
    let reported = response.get("nzo_ids")?.as_array()?;
    Some(
        reported
            .iter()
            .any(|value| value.as_str() == Some(id)),
    )
}

fn slots_from_api_section(section: &Value) -> Option<&Vec<Value>> {
    match section {
        Value::Array(slots) => Some(slots),
        Value::Object(_) => section.get("slots").and_then(Value::as_array),
        _ => None,
    }
}

fn sab_api_status_is_false(json: &Value) -> bool {
    match json.get("status") {
        Some(Value::Bool(false)) => true,
        Some(Value::String(value)) => value.eq_ignore_ascii_case("false"),
        _ => false,
    }
}

fn sab_api_status_is_true(json: &Value) -> bool {
    match json.get("status") {
        Some(Value::Bool(true)) => true,
        Some(Value::String(value)) => value.eq_ignore_ascii_case("true"),
        _ => false,
    }
}

fn sab_api_error_message(json: &Value) -> Option<&str> {
    json.get("error")
        .and_then(Value::as_str)
        .or_else(|| json.get("message").and_then(Value::as_str))
}

fn sab_addfile_nzo_id(json: &Value) -> Option<&str> {
    json.get("nzo_ids")
        .and_then(Value::as_array)
        .and_then(|ids| ids.first())
        .and_then(Value::as_str)
}

/// Normalize a name for comparing against SABnzbd job names during
/// reconciliation.
///
/// Mirrors SAB's `create_work_name` →
/// `sanitize_foldername(strip_extensions(...))` (`nzb/object.py`,
/// `filesystem.py`) so the release title we sent and the `final_name` SAB
/// derived compare equal. The full Windows / `sanitize_safe` illegal-char
/// superset is folded on both sides, making the comparison insensitive to
/// SAB's platform and `sanitize_safe` configuration.
fn normalize_sab_job_name(value: &str) -> String {
    use unicode_normalization::UnicodeNormalization;

    // Mirror SAB's create_work_name = sanitize(strip_extensions(sanitize(name)))
    // in order. The FIRST sanitize strips trailing dots/spaces *before* the
    // extension is removed, so e.g. "Show.nzb." → "Show" (not "Show.nzb"):
    // strip the trailing junk first, then the extension.
    let pre_sanitized = value.trim().trim_end_matches(['.', ' ']);
    let without_ext = strip_sab_nzb_extension(pre_sanitized);

    // Second sanitize: NFC-normalize and fold illegal / control characters to
    // '_' (folding legal `.nzb` chars is a no-op, so it cannot resurrect an
    // extension).
    let folded: String = without_ext
        .nfc()
        .map(|ch| {
            if ch.is_control() || matches!(ch, '/' | '\\' | ':' | '"' | '*' | '?' | '<' | '>' | '|')
            {
                '_'
            } else {
                ch
            }
        })
        .collect();

    // Strip leading/trailing whitespace and any trailing dots (SAB keeps
    // leading dots), then lowercase for a case-insensitive compare.
    folded.trim().trim_end_matches(['.', ' ']).to_lowercase()
}

fn strip_sab_nzb_extension(value: &str) -> &str {
    for ext in [".nzb", ".par2", ".par"] {
        if value.len() > ext.len() && value[value.len() - ext.len()..].eq_ignore_ascii_case(ext) {
            return &value[..value.len() - ext.len()];
        }
    }
    value
}

/// Return the `nzo_id` of a queue/history slot whose name matches `expected`
/// after normalization. `name_key` is `filename` for queue slots and `name`
/// for history slots.
fn sab_reconcile_slot_nzo_id(slot: &Value, name_key: &str, expected: &str) -> Option<String> {
    let slot = slot.as_object()?;
    let nzo_id = slot.get("nzo_id").and_then(Value::as_str)?;
    let name = slot.get(name_key).and_then(Value::as_str)?;
    (normalize_sab_job_name(name) == expected).then(|| nzo_id.to_string())
}

fn extract_sab_error_detail(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|json| sab_api_error_message(&json).map(str::to_string))
        .filter(|detail| !detail.trim().is_empty())
        .unwrap_or_else(|| body.chars().take(600).collect())
}

fn is_sab_auth_error_message(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    normalized.contains("authentication")
        || normalized.contains("unauthorized")
        || normalized.contains("forbidden")
        || normalized.contains("api key required")
        || normalized.contains("api key incorrect")
        || normalized.contains("apikey required")
        || normalized.contains("apikey incorrect")
        || normalized.contains("login failed")
        || normalized.contains("invalid api key")
        || normalized.contains("invalid credentials")
}

fn sabnzbd_queue_priority(raw_priority: Option<&str>) -> i32 {
    match raw_priority
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("force") => 2,
        Some("very high") | Some("high") => 1,
        Some("normal") => 0,
        Some("low") | Some("very low") => -1,
        _ => -1,
    }
}

fn sabnzbd_queue_state(status: &str) -> Option<DownloadQueueState> {
    let normalized = status.to_ascii_uppercase();
    match normalized.as_str() {
        "DELETED" => None,
        "DOWNLOADING" => Some(DownloadQueueState::Downloading),
        "QUEUED" | "FETCHING" | "PROPAGATING" | "GRABBING" => Some(DownloadQueueState::Queued),
        "PAUSED" => Some(DownloadQueueState::Paused),
        // Post-processing stages reported in queue (SABnzbd 4.x can show these)
        "VERIFYING" | "QUICKCHECK" => Some(DownloadQueueState::Verifying),
        "REPAIRING" => Some(DownloadQueueState::Repairing),
        "EXTRACTING" => Some(DownloadQueueState::Extracting),
        "MOVING" | "RUNNING" => Some(DownloadQueueState::Downloading),
        _ => Some(DownloadQueueState::Queued),
    }
}

fn sabnzbd_postprocessing_stage(status: &str) -> Option<String> {
    let normalized = status.to_ascii_uppercase();
    match normalized.as_str() {
        "VERIFYING" | "QUICKCHECK" => Some("VERIFYING".to_string()),
        "REPAIRING" => Some("REPAIRING".to_string()),
        "EXTRACTING" => Some("UNPACKING".to_string()),
        "MOVING" => Some("MOVING".to_string()),
        "RUNNING" => Some("EXECUTING_SCRIPT".to_string()),
        _ => None,
    }
}

fn sabnzbd_history_state(
    status: &str,
    fail_message: Option<&str>,
) -> Option<(DownloadQueueState, Option<String>)> {
    const UNPACK_WRITE_FAILURE: &str = "Unpacking failed, write error or disk is full?";

    let normalized = status.to_ascii_uppercase();
    if normalized == "DELETED" {
        return None;
    }

    let (state, mut reason) = match normalized.as_str() {
        "COMPLETED" => (DownloadQueueState::Completed, None),
        "FAILED" => (DownloadQueueState::Failed, None),
        "QUEUED" => (DownloadQueueState::Queued, None),
        // Active post-processing stages in history
        "VERIFYING" | "QUICKCHECK" => (DownloadQueueState::Verifying, None),
        "REPAIRING" => (DownloadQueueState::Repairing, None),
        "EXTRACTING" => (DownloadQueueState::Extracting, None),
        "MOVING" | "RUNNING" => (DownloadQueueState::Downloading, None),
        _ => {
            if normalized.starts_with("FAILED") {
                let reason = status
                    .split_once(" - ")
                    .map(|(_, detail)| detail.trim().to_string())
                    .filter(|d| !d.is_empty());
                (DownloadQueueState::Failed, reason)
            } else {
                (DownloadQueueState::Downloading, None)
            }
        }
    };

    if state == DownloadQueueState::Failed
        && let Some(fail_message) = fail_message
            .map(str::trim)
            .filter(|value| !value.is_empty())
    {
        if fail_message.eq_ignore_ascii_case(UNPACK_WRITE_FAILURE) {
            return Some((DownloadQueueState::Warning, Some(fail_message.to_string())));
        }
        if reason.is_none() {
            reason = Some(fail_message.to_string());
        }
    }

    Some((state, reason))
}

fn extract_sabnzbd_category(slot: &serde_json::Map<String, Value>) -> Option<String> {
    slot.get("cat")
        .or_else(|| slot.get("category"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "*")
        .map(str::to_string)
}

/// SABnzbd history `nzb_name` is `nzo.filename`: the raw filename the job was
/// added with (an `addfile` upload name, or the Content-Disposition/URL-derived
/// name of an `addurl` fetch), fixed at add time — unlike `name`
/// (`nzo.final_name`), which the `nzbname` API parameter, user renames,
/// pre-queue scripts, and the replace_spaces/dots options all rewrite. It
/// almost always carries the `.nzb` (or `.par2`) extension, which would
/// otherwise parse into the release group (`-NTb.nzb` → group `NTb.nzb`), so
/// strip it exactly the way SAB's own `create_work_name` does.
fn extract_sabnzbd_release_name(slot: &serde_json::Map<String, Value>) -> Option<String> {
    slot.get("nzb_name")
        .or_else(|| slot.get("nzbName"))
        .or_else(|| slot.get("nzbname"))
        .and_then(Value::as_str)
        .map(str::trim)
        .map(|value| strip_sab_nzb_extension(value.trim_end_matches(['.', ' '])).trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn completed_downloads_from_sab_slots(
    slots: &[Value],
    cutoff_ts: Option<i64>,
) -> Vec<CompletedDownload> {
    slots
        .iter()
        .filter_map(|slot| {
            let slot = slot.as_object()?;

            let status = slot.get("status").and_then(Value::as_str).unwrap_or("");
            if !status.eq_ignore_ascii_case("Completed") {
                return None;
            }

            let nzo_id = slot.get("nzo_id").and_then(Value::as_str)?.to_string();

            let completed_ts = extract_i64_value(slot.get("completed"));
            if let Some(cutoff_ts) = cutoff_ts
                && let Some(ts) = completed_ts
                && ts < cutoff_ts
            {
                return None;
            }

            let dest_dir = slot
                .get("storage")
                .or_else(|| slot.get("path"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();

            if dest_dir.is_empty() {
                return None;
            }

            let name = slot
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("Unnamed download")
                .to_string();

            let category = extract_sabnzbd_category(slot);
            let size_bytes = extract_i64_value(slot.get("bytes"));
            let completed_at =
                completed_ts.map(|ts| DateTime::from_timestamp(ts, 0).unwrap_or_else(Utc::now));

            Some(CompletedDownload {
                client_type: "sabnzbd".to_string(),
                client_id: String::new(),
                download_client_item_id: nzo_id.clone(),
                download_id: Some(nzo_id),
                name,
                release_name: extract_sabnzbd_release_name(slot),
                dest_dir,
                category,
                size_bytes,
                completed_at,
                parameters: Vec::new(),
            })
        })
        .collect()
}

fn deserialize_sab_string_list<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    Ok(match value {
        Value::Null => Vec::new(),
        Value::String(value) => value
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect(),
        Value::Array(values) => values
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    })
}

fn normalize_optional_auth_value(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn dedupe_strings(values: Vec<String>) -> Vec<String> {
    let mut deduped = Vec::new();
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        if deduped.iter().any(|existing| existing == trimmed) {
            continue;
        }
        deduped.push(trimmed.to_string());
    }
    deduped
}

fn resolved_complete_dir(
    configured_complete_dir: &str,
    full_status: Option<&SabnzbdFullStatus>,
) -> String {
    let complete_dir = configured_complete_dir.trim();
    if complete_dir.is_empty() {
        return full_status
            .map(|status| status.complete_dir.trim().to_string())
            .unwrap_or_default();
    }

    let configured_path = std::path::Path::new(complete_dir);
    if configured_path.is_absolute() {
        return complete_dir.to_string();
    }

    full_status
        .map(|status| status.complete_dir.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| complete_dir.to_string())
}

fn category_output_root(complete_dir: &str, category_dir: &str) -> String {
    let trimmed_dir = category_dir.trim().trim_end_matches('*');
    if trimmed_dir.is_empty() {
        return complete_dir.trim().to_string();
    }
    if complete_dir.trim().is_empty() {
        return trimmed_dir.to_string();
    }

    join_output_root(complete_dir, trimmed_dir)
}

fn join_output_root(base: &str, suffix: &str) -> String {
    let base = base.trim().trim_end_matches(['/', '\\']);
    let suffix = suffix.trim().trim_start_matches(['/', '\\']);
    if base.is_empty() {
        return suffix.to_string();
    }
    if suffix.is_empty() {
        return base.to_string();
    }
    format!("{base}/{suffix}")
}

fn is_localhost_base_url(base_url: &str) -> Option<bool> {
    let parsed = reqwest::Url::parse(base_url).ok()?;
    let host = parsed.host_str()?;
    Some(matches!(
        host,
        "localhost" | "127.0.0.1" | "::1" | "0.0.0.0" | "host.docker.internal"
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        SAB_ADDFILE_UPLOAD_FIELD, SabAddfileOutcome, SabApiAuth, SabApiResponseEvaluation,
        SabnzbdDownloadClient, build_sab_api_urls, completed_downloads_from_sab_slots,
        evaluate_sab_addfile_response, evaluate_sab_api_response, extract_sabnzbd_category,
        map_sabnzbd_outbound_error, normalize_sab_job_name, redact_sab_secret_values,
        sab_addfile_query_params, sab_api_mode_matches_response, sab_delete_removed_hinted_id,
        sab_reconcile_slot_nzo_id, sabnzbd_history_state, sabnzbd_queue_state,
    };
    use chrono::Utc;
    use reqwest::StatusCode;
    use scryer_application::{
        AppError, DownloadClient, DownloadClientAddRequest, DownloadSubmissionPurpose,
        ResolvedDownloadArtifact,
    };
    use scryer_domain::{DownloadQueueState, MediaFacet, Title};
    use scryer_outbound_http::OutboundHttpError;
    use serde_json::json;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::Semaphore;
    use wiremock::matchers::{method, path, query_param, query_param_is_missing};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::downloads::staged_nzb_store::FileSystemStagedNzbStore;

    fn test_add_request(download_id: &str) -> DownloadClientAddRequest {
        let facet = MediaFacet::Movie;
        DownloadClientAddRequest {
            title: Title {
                id: "title-1".to_string(),
                name: "Test Title".to_string(),
                library_id: scryer_domain::default_library_id_for_facet(&facet),
                facet,
                monitored: true,
                tags: vec![],
                canonical_tags: vec![],
                external_ids: vec![],
                root_folder_id: scryer_domain::root_folder_id_for_path("/data/movies"),
                created_by: None,
                created_at: Utc::now(),
                year: None,
                overview: None,
                poster_url: None,
                poster_source_url: None,
                background_url: None,
                background_source_url: None,
                sort_title: None,
                catalog_sort_key: String::new(),
                slug: None,
                imdb_id: None,
                runtime_minutes: None,
                popularity: None,
                content_status: None,
                language: None,
                first_aired: None,
                network: None,
                studio: None,
                country: None,
                aliases: vec![],
                tagged_aliases: vec![],
                metadata_language: None,
                metadata_fetched_at: None,
                min_availability: None,
                digital_release_date: None,
                folder_path: None,
            },
            search_facet: None,
            purpose: DownloadSubmissionPurpose::Standard,
            download_id: Some(
                scryer_domain::download_identity::DownloadId::from_wire(download_id)
                    .expect("test token should be a wire DownloadId"),
            ),
            source_hint: Some("https://example.invalid/release.nzb".to_string()),
            staged_nzb: None,
            resolved_download_artifact: Some(ResolvedDownloadArtifact::Nzb {
                bytes: b"<nzb></nzb>".to_vec(),
                file_name: Some("Test Release.nzb".to_string()),
                content_type: Some("application/x-nzb".to_string()),
            }),
            source_kind: None,
            source_title: Some("Test Release".to_string()),
            source_password: Some("archive-password".to_string()),
            category: Some("movies".to_string()),
            queue_priority: Some("high".to_string()),
            download_directory: None,
            release_title: None,
            indexer_name: None,
            indexer_id: None,
            info_hash_hint: None,
            seed_goal_ratio: None,
            seed_goal_seconds: None,
            tracker_min_seed_ratio: None,
            tracker_min_seed_time_minutes: None,
            season_pack_seed_ratio: None,
            season_pack_seed_time_minutes: None,
            is_recent: None,
            season_pack: None,
        }
    }

    #[tokio::test]
    async fn submit_addfile_request_has_no_scryer_download_id_token() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/release.nzb"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"<nzb></nzb>".to_vec()))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api"))
            .and(query_param("mode", "queue"))
            .and(query_param("limit", "0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "queue": { "slots": [] }
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "status": true,
                "nzo_ids": ["SABnzbd_nzo_abc123"]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let download_id = "scryer-download:00000000-0000-4000-8000-000000000021";
        let staged_nzb_dir = tempfile::tempdir().expect("staged nzb directory");
        let staged_nzb_store = Arc::new(
            FileSystemStagedNzbStore::new(staged_nzb_dir.path())
                .await
                .expect("staged nzb store"),
        );
        let client = SabnzbdDownloadClient::with_staged_nzb_store(
            server.uri(),
            "test-api-key".to_string(),
            staged_nzb_store,
            Arc::new(Semaphore::new(1)),
        );
        let mut add_request = test_add_request(download_id);
        add_request.source_hint = Some(format!("{}/release.nzb", server.uri()));
        let result = client
            .submit_download(&add_request)
            .await
            .expect("addfile should succeed");
        assert_eq!(result.job_id, "SABnzbd_nzo_abc123");

        let requests = server
            .received_requests()
            .await
            .expect("addfile request should be recorded");
        let request = requests
            .iter()
            .find(|request| {
                request
                    .url
                    .query_pairs()
                    .any(|(key, value)| key == "mode" && value == "addfile")
            })
            .expect("addfile request");
        let query = request
            .url
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect::<Vec<_>>();
        assert_eq!(
            query,
            vec![
                ("mode".to_string(), "addfile".to_string()),
                ("output".to_string(), "json".to_string()),
                ("nzbname".to_string(), "Test Release".to_string()),
                ("priority".to_string(), "1".to_string()),
                ("apikey".to_string(), "test-api-key".to_string()),
                ("cat".to_string(), "movies".to_string()),
                ("password".to_string(), "archive-password".to_string()),
            ],
        );
        let request_text = format!(
            "{}\n{}",
            request.url,
            String::from_utf8_lossy(&request.body)
        );
        assert!(request_text.contains("name=\"nzbfile\""));
        assert!(request_text.contains("filename=\"Test Release.nzb\""));
        assert!(request_text.contains("<nzb></nzb>"));
        assert!(
            !request_text.contains(download_id),
            "SABnzbd must not receive a Scryer download token: {request_text}",
        );
    }

    #[test]
    fn outbound_rate_limit_preserves_retry_after() {
        let error = OutboundHttpError::RateLimited(scryer_outbound_http::RateLimitedError {
            scope: scryer_outbound_http::RateLimitScopeKey::from("sabnzbd"),
            retry_after: Some(Duration::from_secs(35)),
            attempts: 1,
            retry_after_source: scryer_outbound_http::RetryAfterSource::Seconds,
            request_label: std::borrow::Cow::Borrowed("sabnzbd"),
        });
        let error = map_sabnzbd_outbound_error("sabnzbd queue", error);

        match error {
            AppError::TemporaryUnavailable {
                message,
                retry_after,
                ..
            } => {
                assert!(message.contains("retry after 35s"));
                assert_eq!(retry_after, Some(Duration::from_secs(35)));
            }
            other => panic!("expected temporary unavailable error, got {other:?}"),
        }
    }

    #[test]
    fn exact_completed_lookup_keeps_old_retained_history() {
        let slots = vec![json!({
            "status": "Completed",
            "nzo_id": "old-nzo",
            "completed": 0,
            "storage": "/downloads/complete/retained",
            "name": "Retained download",
        })];

        assert!(
            completed_downloads_from_sab_slots(
                &slots,
                Some(Utc::now().timestamp() - (7 * 24 * 60 * 60)),
            )
            .is_empty()
        );
        assert_eq!(
            completed_downloads_from_sab_slots(&slots, None)[0].download_client_item_id,
            "old-nzo"
        );
    }

    #[test]
    fn sabnzbd_deleted_items_are_skipped() {
        assert_eq!(sabnzbd_queue_state("Deleted"), None);
        assert_eq!(sabnzbd_history_state("Deleted", None), None);
    }

    #[test]
    fn sabnzbd_unknown_history_status_remains_in_progress() {
        assert_eq!(
            sabnzbd_history_state("FutureStatus", None),
            Some((DownloadQueueState::Downloading, None))
        );
    }

    #[test]
    fn sabnzbd_unpack_write_failure_is_a_warning() {
        let message = "Unpacking failed, write error or disk is full?";
        assert_eq!(
            sabnzbd_history_state("Failed", Some(message)),
            Some((DownloadQueueState::Warning, Some(message.to_string())))
        );
    }

    #[test]
    fn sabnzbd_regular_failure_preserves_its_message() {
        assert_eq!(
            sabnzbd_history_state("Failed", Some("54 articles were missing")),
            Some((
                DownloadQueueState::Failed,
                Some("54 articles were missing".to_string())
            ))
        );
    }

    #[test]
    fn extract_sabnzbd_category_trims_and_ignores_star() {
        let slot = json!({"cat": " movies "});
        let slot = slot.as_object().expect("object");
        assert_eq!(extract_sabnzbd_category(slot).as_deref(), Some("movies"));

        let slot = json!({"category": "*"});
        let slot = slot.as_object().expect("object");
        assert_eq!(extract_sabnzbd_category(slot), None);

        let slot = json!({"category": ""});
        let slot = slot.as_object().expect("object");
        assert_eq!(extract_sabnzbd_category(slot), None);
    }

    #[test]
    fn build_sab_api_urls_includes_sabnzbd_compatibility_path() {
        assert_eq!(
            build_sab_api_urls("http://altmount:8080"),
            vec![
                "http://altmount:8080/api".to_string(),
                "http://altmount:8080/sabnzbd/api".to_string(),
            ]
        );
    }

    #[test]
    fn build_sab_api_urls_preserves_existing_prefix() {
        assert_eq!(
            build_sab_api_urls("http://example.test/altmount"),
            vec![
                "http://example.test/altmount/api".to_string(),
                "http://example.test/altmount/sabnzbd/api".to_string(),
            ]
        );
    }

    #[test]
    fn sab_addfile_uses_common_sab_upload_field() {
        assert_eq!(SAB_ADDFILE_UPLOAD_FIELD, "nzbfile");
    }

    #[test]
    fn sab_addfile_query_params_include_api_key() {
        let fields = sab_addfile_query_params(
            &SabApiAuth::ApiKey("secret-key".to_string()),
            "release-name",
            "1",
            Some("movies"),
            Some("archive-password"),
        );

        assert_eq!(
            fields,
            vec![
                ("mode", "addfile".to_string()),
                ("output", "json".to_string()),
                ("nzbname", "release-name".to_string()),
                ("priority", "1".to_string()),
                ("apikey", "secret-key".to_string()),
                ("cat", "movies".to_string()),
                ("password", "archive-password".to_string()),
            ]
        );
    }

    #[test]
    fn sab_addfile_query_params_include_arr_credentials() {
        let fields = sab_addfile_query_params(
            &SabApiAuth::Credentials {
                username: "http://sonarr:8989".to_string(),
                password: "arr-secret".to_string(),
            },
            "release-name",
            "0",
            None,
            None,
        );

        assert!(fields.contains(&("ma_username", "http://sonarr:8989".to_string())));
        assert!(fields.contains(&("ma_password", "arr-secret".to_string())));
    }

    #[test]
    fn sab_error_redaction_removes_secret_query_values() {
        let message = "request failed for http://sab/api?mode=addfile&apikey=secret-key&ma_password=arr-secret&password=archive#frag";

        let redacted = redact_sab_secret_values(message);

        assert!(!redacted.contains("secret-key"));
        assert!(!redacted.contains("arr-secret"));
        assert!(!redacted.contains("archive"));
        assert!(redacted.contains("apikey=<redacted>"));
        assert!(redacted.contains("ma_password=<redacted>"));
        assert!(redacted.contains("password=<redacted>"));
    }

    #[test]
    fn sab_api_mode_match_rejects_non_sab_version_shape() {
        let json = json!({"data": {"api_key": "abc123"}});
        assert!(!sab_api_mode_matches_response(Some("version"), &json));
    }

    #[test]
    fn sab_api_mode_match_accepts_success_status_for_queue_mutations() {
        let json = json!({"status": true});
        assert!(sab_api_mode_matches_response(Some("queue"), &json));
        assert!(sab_api_mode_matches_response(Some("history"), &json));
    }

    #[test]
    fn evaluate_sab_api_response_marks_non_sab_shape_retryable() {
        let outcome = evaluate_sab_api_response(
            "sabnzbd api",
            Some("queue"),
            StatusCode::OK,
            r#"{"data":{"api_key":"abc123"}}"#,
        );

        assert!(matches!(outcome, SabApiResponseEvaluation::Retry(_)));
    }

    #[test]
    fn evaluate_sab_api_response_retries_non_auth_unsuccessful_statuses() {
        let not_found =
            evaluate_sab_api_response("sabnzbd api", Some("queue"), StatusCode::NOT_FOUND, "");
        let server_error = evaluate_sab_api_response(
            "sabnzbd api",
            Some("queue"),
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed",
        );

        assert!(matches!(not_found, SabApiResponseEvaluation::Retry(_)));
        assert!(matches!(server_error, SabApiResponseEvaluation::Retry(_)));
    }

    // The shared GET evaluator must keep retrying idempotent reads across the
    // alternate API path (altmount/nzbdav compatibility); only the addfile
    // POST classification below changes.
    #[test]
    fn evaluate_sab_api_response_get_success_is_unchanged() {
        let outcome = evaluate_sab_api_response(
            "sabnzbd api",
            Some("queue"),
            StatusCode::OK,
            r#"{"queue":{"slots":[]}}"#,
        );
        assert!(matches!(outcome, SabApiResponseEvaluation::Success(_)));
    }

    #[test]
    fn evaluate_sab_addfile_accepts_status_true_with_nzo_id() {
        let outcome = evaluate_sab_addfile_response(
            StatusCode::OK,
            r#"{"status": true, "nzo_ids": ["SABnzbd_nzo_abc123"]}"#,
        );
        match outcome {
            SabAddfileOutcome::Accepted(nzo_id) => assert_eq!(nzo_id, "SABnzbd_nzo_abc123"),
            other => panic!("expected Accepted, got {other:?}"),
        }
    }

    #[test]
    fn evaluate_sab_addfile_empty_nzo_ids_is_rejected() {
        let outcome =
            evaluate_sab_addfile_response(StatusCode::OK, r#"{"status": true, "nzo_ids": []}"#);
        assert!(matches!(outcome, SabAddfileOutcome::Rejected(_)));
    }

    #[test]
    fn evaluate_sab_addfile_status_false_is_rejected_with_detail() {
        let outcome = evaluate_sab_addfile_response(
            StatusCode::OK,
            r#"{"status": false, "error": "Duplicate NZB"}"#,
        );
        match outcome {
            SabAddfileOutcome::Rejected(detail) => {
                assert!(detail.contains("Duplicate NZB"), "detail was {detail}")
            }
            other => panic!("expected Rejected, got {other:?}"),
        }
    }

    #[test]
    fn evaluate_sab_addfile_auth_message_status_false_is_auth() {
        let outcome = evaluate_sab_addfile_response(
            StatusCode::OK,
            r#"{"status": false, "error": "API Key Incorrect"}"#,
        );
        assert!(matches!(outcome, SabAddfileOutcome::Auth(_)));
    }

    #[test]
    fn evaluate_sab_addfile_unauthorized_status_is_auth() {
        let outcome = evaluate_sab_addfile_response(StatusCode::UNAUTHORIZED, "");
        assert!(matches!(outcome, SabAddfileOutcome::Auth(_)));
    }

    #[test]
    fn evaluate_sab_addfile_server_error_is_ambiguous() {
        let outcome =
            evaluate_sab_addfile_response(StatusCode::INTERNAL_SERVER_ERROR, "addfile failed");
        assert!(matches!(
            outcome,
            SabAddfileOutcome::Ambiguous(AppError::DownloadSubmitAmbiguous(_))
        ));
    }

    #[test]
    fn evaluate_sab_addfile_non_json_body_is_ambiguous() {
        let outcome = evaluate_sab_addfile_response(StatusCode::OK, "<html>gateway error</html>");
        assert!(matches!(outcome, SabAddfileOutcome::Ambiguous(_)));
    }

    #[test]
    fn evaluate_sab_addfile_unexpected_shape_is_ambiguous() {
        let outcome =
            evaluate_sab_addfile_response(StatusCode::OK, r#"{"data":{"api_key":"abc123"}}"#);
        assert!(matches!(outcome, SabAddfileOutcome::Ambiguous(_)));
    }

    // The addfile POST targets an already-resolved API path, so a 404 there is
    // ambiguous (reconcile) — never a silent alternate-path re-POST that could
    // duplicate a landed job.
    #[test]
    fn evaluate_sab_addfile_not_found_is_ambiguous() {
        let outcome = evaluate_sab_addfile_response(StatusCode::NOT_FOUND, "");
        assert!(matches!(outcome, SabAddfileOutcome::Ambiguous(_)));
    }

    #[test]
    fn normalize_sab_job_name_strips_nzb_extension() {
        assert_eq!(
            normalize_sab_job_name("Show.S01E05.1080p.WEB.nzb"),
            normalize_sab_job_name("Show.S01E05.1080p.WEB")
        );
        assert_eq!(
            normalize_sab_job_name("Show.S01E05.1080p.WEB.nzb"),
            "show.s01e05.1080p.web"
        );
    }

    #[test]
    fn normalize_sab_job_name_strips_par_extensions() {
        assert_eq!(
            normalize_sab_job_name("archive.part1.par2"),
            "archive.part1"
        );
        assert_eq!(normalize_sab_job_name("archive.PAR"), "archive");
    }

    #[test]
    fn normalize_sab_job_name_folds_illegal_and_control_chars() {
        assert_eq!(normalize_sab_job_name("A:B/C\"D"), "a_b_c_d");
        assert_eq!(normalize_sab_job_name("A*B?C<D>E|F\\G"), "a_b_c_d_e_f_g");
        assert_eq!(normalize_sab_job_name("tab\there"), "tab_here");
    }

    // SAB's first sanitize pass strips trailing dots/spaces BEFORE the
    // extension is removed, so trailing junk after the extension must not keep
    // the extension in our normalized form (else reconciliation misses the
    // job SAB actually named).
    #[test]
    fn normalize_sab_job_name_strips_trailing_junk_before_extension() {
        assert_eq!(normalize_sab_job_name("Show.nzb."), "show");
        assert_eq!(normalize_sab_job_name("Show.nzb "), "show");
        assert_eq!(
            normalize_sab_job_name("Show.nzb."),
            normalize_sab_job_name("Show")
        );
    }

    #[test]
    fn normalize_sab_job_name_strips_trailing_dots_and_spaces() {
        assert_eq!(normalize_sab_job_name("  Release.Name..  "), "release.name");
    }

    #[test]
    fn normalize_sab_job_name_nfc_normalizes_and_case_folds() {
        // Composed "é" (U+00E9) vs decomposed "e" + combining acute (U+0301).
        let composed = "Caf\u{00e9}";
        let decomposed = "Cafe\u{0301}";
        assert_ne!(composed, decomposed);
        assert_eq!(
            normalize_sab_job_name(composed),
            normalize_sab_job_name(decomposed)
        );
        assert_eq!(
            normalize_sab_job_name("RELEASE"),
            normalize_sab_job_name("release")
        );
    }

    #[test]
    fn sab_reconcile_slot_matches_normalized_name() {
        let slot = json!({"nzo_id": "SABnzbd_nzo_1", "filename": "My.Movie.2024.1080p"});
        let expected = normalize_sab_job_name("My.Movie.2024.1080p.nzb");
        assert_eq!(
            sab_reconcile_slot_nzo_id(&slot, "filename", &expected).as_deref(),
            Some("SABnzbd_nzo_1")
        );
        assert_eq!(
            sab_reconcile_slot_nzo_id(&slot, "filename", "different.release"),
            None
        );

        let history_slot = json!({"nzo_id": "SABnzbd_nzo_2", "name": "My.Movie.2024.1080p"});
        assert_eq!(
            sab_reconcile_slot_nzo_id(&history_slot, "name", &expected).as_deref(),
            Some("SABnzbd_nzo_2")
        );
    }

    #[tokio::test]
    async fn delete_history_item_only_deletes_data_when_requested() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api"))
            .and(query_param("mode", "history"))
            .and(query_param("name", "delete"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "status": true })))
            .expect(2)
            .mount(&server)
            .await;
        // A history delete reports no `nzo_ids`, so the client probes the queue
        // to confirm the hint. An empty queue confirms it.
        Mock::given(method("GET"))
            .and(path("/api"))
            .and(query_param("mode", "queue"))
            .and(query_param_is_missing("name"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "queue": { "slots": [] } })),
            )
            .expect(2)
            .mount(&server)
            .await;

        let client = SabnzbdDownloadClient::new(server.uri(), "api-key".to_string());
        client
            .delete_queue_item("keep-data", true, false)
            .await
            .expect("history delete without data removal should succeed");
        client
            .delete_queue_item("delete-data", true, true)
            .await
            .expect("history delete with data removal should succeed");

        let requests = server
            .received_requests()
            .await
            .expect("requests should be recorded");
        let keep_data_request = requests
            .iter()
            .find(|request| {
                request
                    .url
                    .query_pairs()
                    .any(|(key, value)| key == "value" && value == "keep-data")
            })
            .expect("history delete without data removal should be requested");
        assert!(
            !keep_data_request
                .url
                .query_pairs()
                .any(|(key, _)| key == "del_files")
        );

        let delete_data_request = requests
            .iter()
            .find(|request| {
                request
                    .url
                    .query_pairs()
                    .any(|(key, value)| key == "value" && value == "delete-data")
            })
            .expect("history delete with data removal should be requested");
        assert!(
            delete_data_request
                .url
                .query_pairs()
                .any(|(key, value)| key == "del_files" && value == "1")
        );
    }

    #[test]
    fn sab_delete_removed_hinted_id_reads_reported_removals() {
        assert_eq!(
            sab_delete_removed_hinted_id(
                &json!({ "status": true, "nzo_ids": ["SABnzbd_nzo_1"] }),
                "SABnzbd_nzo_1"
            ),
            Some(true)
        );
        assert_eq!(
            sab_delete_removed_hinted_id(&json!({ "status": true, "nzo_ids": [] }), "SABnzbd_nzo_1"),
            Some(false)
        );
        assert_eq!(
            sab_delete_removed_hinted_id(
                &json!({ "status": true, "nzo_ids": ["SABnzbd_nzo_2"] }),
                "SABnzbd_nzo_1"
            ),
            Some(false)
        );
        // No report at all (history deletes, SAB-compatible backends): unknown.
        assert_eq!(
            sab_delete_removed_hinted_id(&json!({ "status": true }), "SABnzbd_nzo_1"),
            None
        );
        assert_eq!(
            sab_delete_removed_hinted_id(&json!({ "status": true, "nzo_ids": "x" }), "SABnzbd_nzo_1"),
            None
        );
    }

    async fn mount_sab_delete(
        server: &MockServer,
        mode: &str,
        nzo_id: &str,
        body: serde_json::Value,
        expected: u64,
    ) {
        Mock::given(method("GET"))
            .and(path("/api"))
            .and(query_param("mode", mode))
            .and(query_param("name", "delete"))
            .and(query_param("value", nzo_id))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(expected)
            .mount(server)
            .await;
    }

    async fn mount_sab_queue_listing(server: &MockServer, slots: serde_json::Value, expected: u64) {
        Mock::given(method("GET"))
            .and(path("/api"))
            .and(query_param("mode", "queue"))
            .and(query_param_is_missing("name"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "queue": { "slots": slots } })),
            )
            .expect(expected)
            .mount(server)
            .await;
    }

    #[test]
    fn evaluate_sab_api_response_accepts_empty_removal_report() {
        // Real SAB: `_api_queue_delete` answers status=false when no id matched.
        let outcome = evaluate_sab_api_response(
            "sabnzbd api",
            Some("queue"),
            StatusCode::OK,
            r#"{"status": false, "nzo_ids": []}"#,
        );
        assert!(matches!(outcome, SabApiResponseEvaluation::Success(_)));

        // A status=false that carries an error is still a failure.
        let outcome = evaluate_sab_api_response(
            "sabnzbd api",
            Some("queue"),
            StatusCode::OK,
            r#"{"status": false, "nzo_ids": [], "error": "API Key Incorrect"}"#,
        );
        assert!(matches!(outcome, SabApiResponseEvaluation::Failure(_)));
    }

    // The manual-import case: the UI believes the job is queued, but SAB has
    // already moved it to history. Real SAB answers the queue delete with
    // `status: false` and an empty `nzo_ids` (see `_api_queue_delete`), so the
    // client must fall through to a history delete instead of failing or
    // reporting success and letting the next poll re-adopt the job.
    #[tokio::test]
    async fn delete_queue_hint_falls_through_to_history_when_nothing_was_removed() {
        let server = MockServer::start().await;
        mount_sab_delete(
            &server,
            "queue",
            "SABnzbd_nzo_hist",
            json!({ "status": false, "nzo_ids": [] }),
            1,
        )
        .await;
        mount_sab_delete(&server, "history", "SABnzbd_nzo_hist", json!({ "status": true }), 1).await;
        // The queue delete reported its removals, so no probe is needed.
        mount_sab_queue_listing(&server, json!([]), 0).await;

        let client = SabnzbdDownloadClient::new(server.uri(), "api-key".to_string());
        client
            .delete_queue_item("SABnzbd_nzo_hist", false, true)
            .await
            .expect("history fallback should succeed");

        let requests = server
            .received_requests()
            .await
            .expect("requests should be recorded");
        let history_delete = requests
            .iter()
            .find(|request| {
                request
                    .url
                    .query_pairs()
                    .any(|(key, value)| key == "mode" && value == "history")
            })
            .expect("history delete should be requested");
        // `remove_data` carries over to the fallback delete.
        assert!(
            history_delete
                .url
                .query_pairs()
                .any(|(key, value)| key == "del_files" && value == "1")
        );
    }

    // The reverse mistake: hinted as history while the job is still queued.
    // History deletes never report `nzo_ids`, so the client probes the queue,
    // finds the job, and issues the queue delete.
    #[tokio::test]
    async fn delete_history_hint_falls_through_to_queue_when_still_queued() {
        let server = MockServer::start().await;
        mount_sab_delete(&server, "history", "SABnzbd_nzo_q", json!({ "status": true }), 1).await;
        mount_sab_queue_listing(
            &server,
            json!([{ "nzo_id": "SABnzbd_nzo_other" }, { "nzo_id": "SABnzbd_nzo_q" }]),
            1,
        )
        .await;
        mount_sab_delete(
            &server,
            "queue",
            "SABnzbd_nzo_q",
            json!({ "status": true, "nzo_ids": ["SABnzbd_nzo_q"] }),
            1,
        )
        .await;

        let client = SabnzbdDownloadClient::new(server.uri(), "api-key".to_string());
        client
            .delete_queue_item("SABnzbd_nzo_q", true, false)
            .await
            .expect("queue fallback should succeed");
    }

    // A correct queue hint on real SAB: one request, no probe, no fallback.
    #[tokio::test]
    async fn delete_with_confirmed_removal_issues_exactly_one_request() {
        let server = MockServer::start().await;
        mount_sab_delete(
            &server,
            "queue",
            "SABnzbd_nzo_q",
            json!({ "status": true, "nzo_ids": ["SABnzbd_nzo_q"] }),
            1,
        )
        .await;
        mount_sab_delete(&server, "history", "SABnzbd_nzo_q", json!({ "status": true }), 0).await;
        mount_sab_queue_listing(&server, json!([]), 0).await;

        let client = SabnzbdDownloadClient::new(server.uri(), "api-key".to_string());
        client
            .delete_queue_item("SABnzbd_nzo_q", false, false)
            .await
            .expect("confirmed queue delete should succeed");
    }

    // SAB-compatible backends may not report `nzo_ids` for queue deletes. The
    // probe cannot tell "removed" from "never queued", so the client also
    // issues the history delete: harmless when the job is gone, and the only
    // way to remove it when the hint was wrong.
    #[tokio::test]
    async fn delete_queue_hint_without_removal_report_probes_and_covers_history() {
        let server = MockServer::start().await;
        mount_sab_delete(&server, "queue", "SABnzbd_nzo_c", json!({ "status": true }), 1).await;
        mount_sab_queue_listing(&server, json!([]), 1).await;
        mount_sab_delete(&server, "history", "SABnzbd_nzo_c", json!({ "status": true }), 1).await;

        let client = SabnzbdDownloadClient::new(server.uri(), "api-key".to_string());
        client
            .delete_queue_item("SABnzbd_nzo_c", false, false)
            .await
            .expect("compat queue delete should succeed");
    }

    // decypharr keeps one store for queue and history: any delete removes the
    // item and answers `{"status": true, "error": ""}` with no `nzo_ids`, and
    // a delete for an id it no longer knows answers HTTP 500 with
    // `status: false`. The probe then finds the item gone and the best-effort
    // history delete is rejected; that must not fail a delete that landed.
    #[tokio::test]
    async fn delete_queue_hint_tolerates_rejected_fallback_after_probe() {
        let server = MockServer::start().await;
        mount_sab_delete(
            &server,
            "queue",
            "decypharr-1",
            json!({ "status": true, "error": "" }),
            1,
        )
        .await;
        mount_sab_queue_listing(&server, json!([]), 1).await;
        Mock::given(method("GET"))
            .and(path("/api"))
            .and(query_param("mode", "history"))
            .and(query_param("name", "delete"))
            .and(query_param("value", "decypharr-1"))
            .respond_with(ResponseTemplate::new(500).set_body_json(json!({
                "status": false,
                "error": "All deletions failed: Failed to delete decypharr-1: not found",
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = SabnzbdDownloadClient::new(server.uri(), "api-key".to_string());
        client
            .delete_queue_item("decypharr-1", false, false)
            .await
            .expect("a rejected best-effort fallback must not fail the landed delete");
    }

    // A probe that cannot run must not turn an already-issued delete into a
    // failure the poller would log and ignore anyway.
    #[tokio::test]
    async fn delete_keeps_hinted_result_when_probe_fails() {
        let server = MockServer::start().await;
        mount_sab_delete(&server, "history", "SABnzbd_nzo_p", json!({ "status": true }), 1).await;
        Mock::given(method("GET"))
            .and(path("/api"))
            .and(query_param("mode", "queue"))
            .and(query_param_is_missing("name"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&server)
            .await;

        let client = SabnzbdDownloadClient::new(server.uri(), "api-key".to_string());
        client
            .delete_queue_item("SABnzbd_nzo_p", true, false)
            .await
            .expect("probe failure should not fail the delete");
    }
}
