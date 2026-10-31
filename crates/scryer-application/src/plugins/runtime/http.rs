static PLUGIN_HTTP_RATE_LIMITS: LazyLock<RateLimitRegistry> = LazyLock::new(RateLimitRegistry::new);
static DEFAULT_PLUGIN_HTTP_CLIENT: LazyLock<Result<OutboundHttpClient, String>> =
    LazyLock::new(build_plugin_http_client);
#[cfg(test)]
static RULE_PACK_PLUGIN_HTTP_CLIENT: LazyLock<Result<OutboundHttpClient, String>> =
    LazyLock::new(build_plugin_http_client);
const PLUGIN_HTTP_MAX_VALIDATED_REDIRECTS: usize = 3;

#[derive(Clone, Copy, Eq, PartialEq)]
enum PluginRedirectPolicy {
    Reject,
    FollowValidated,
}

#[derive(Debug)]
struct FetchedPluginBytes {
    bytes: Vec<u8>,
    actual_url: String,
}

#[derive(Clone, Copy)]
enum PluginHttpClientProfile {
    DefaultFetch,
    #[cfg(test)]
    RulePackFetch,
}
fn build_plugin_http_client() -> Result<OutboundHttpClient, String> {
    Ok(OutboundHttpClient::new(
        no_redirect_reqwest_client(),
        PLUGIN_HTTP_RATE_LIMITS.clone(),
    ))
}
fn plugin_http_client(profile: PluginHttpClientProfile) -> AppResult<&'static OutboundHttpClient> {
    let cached = match profile {
        PluginHttpClientProfile::DefaultFetch => &*DEFAULT_PLUGIN_HTTP_CLIENT,
        #[cfg(test)]
        PluginHttpClientProfile::RulePackFetch => &*RULE_PACK_PLUGIN_HTTP_CLIENT,
    };

    cached
        .as_ref()
        .map_err(|error| AppError::Repository(error.clone()))
}
fn map_plugin_outbound_error(label: &str, error: OutboundHttpError) -> AppError {
    match error {
        OutboundHttpError::DispatchRejected => {
            AppError::canceled(format!("failed to download {label}: dispatch rejected"))
        }
        OutboundHttpError::RateLimited(rate_limited) => {
            let retry_after = rate_limited.retry_after.filter(|delay| !delay.is_zero());
            AppError::rate_limited_temporary_unavailable(
                match retry_after {
                    Some(delay) => format!(
                        "failed to download {label}: rate limited, retry after {}s",
                        delay.as_secs()
                    ),
                    None => format!("failed to download {label}: rate limited"),
                },
                retry_after,
                RateLimitCooldownAction::AlreadyRecorded,
            )
        }
        OutboundHttpError::Transport { source, .. } => {
            AppError::Repository(format!("failed to download {label}: {source}"))
        }
    }
}
async fn fetch_plugin_bytes_with_redirect_policy(
    url: &str,
    label: &str,
    scope: impl Into<String>,
    redirect_policy: PluginRedirectPolicy,
) -> AppResult<FetchedPluginBytes> {
    let outbound_http = plugin_http_client(PluginHttpClientProfile::DefaultFetch)?;
    let scope = scope.into();
    let mut redirects_followed = 0;
    let mut target =
        scryer_outbound_http::prepare_untrusted_public_http_target(url, "plugin artifact")
            .await
            .map_err(map_artifact_destination_error)?;
    loop {
        let request_scope = if redirects_followed == 0 {
            scope.clone()
        } else {
            format!("{scope}:redirect:{redirects_followed}")
        };
        let response = outbound_http
            .send(plugin_request_policy(request_scope, label), || {
                target.client().get(target.url().clone())
            })
            .await
            .map_err(|error| map_plugin_outbound_error(label, error))?;
        if response.status().is_redirection() {
            if redirect_policy == PluginRedirectPolicy::Reject {
                return Err(AppError::validation_refused(crate::error_reasons::plugins::ARTIFACT_REDIRECTS_NOT_ALLOWED, format!(
                    "plugin artifact redirects are not allowed for {label}"
                )));
            }
            if redirects_followed >= PLUGIN_HTTP_MAX_VALIDATED_REDIRECTS {
                return Err(AppError::validation_refused(crate::error_reasons::plugins::ARTIFACT_REDIRECT_LIMIT_EXCEEDED, format!(
                    "plugin artifact redirect limit exceeded for {label}"
                )));
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .ok_or_else(|| {
                    AppError::validation_refused(crate::error_reasons::plugins::ARTIFACT_REDIRECT_DID_NOT_INCLUDE_LOCATION_HEADER, format!(
                        "plugin artifact redirect for {label} did not include a Location header"
                    ))
                })?;
            let redirect_url = plugin_redirect_location_url(target.url(), location, label)?;
            target = scryer_outbound_http::prepare_untrusted_public_http_target_from_url(
                redirect_url,
                "plugin artifact",
            )
            .await
            .map_err(map_artifact_destination_error)?;
            redirects_followed += 1;
            continue;
        }
        let response = response.error_for_status().map_err(|error| {
            AppError::Repository(format!("failed to download {label}: {error}"))
        })?;
        let bytes = response
            .bytes()
            .await
            .map_err(|error| AppError::Repository(format!("failed to read {label}: {error}")))?;
        return Ok(FetchedPluginBytes {
            bytes: bytes.to_vec(),
            actual_url: target.url().to_string(),
        });
    }
}

fn plugin_redirect_location_url(
    current_url: &reqwest::Url,
    location: &reqwest::header::HeaderValue,
    label: &str,
) -> AppResult<reqwest::Url> {
    let location = location.to_str().map_err(|error| {
        AppError::validation_refused(crate::error_reasons::plugins::ARTIFACT_REDIRECT_INCLUDED_INVALID_LOCATION_HEADER, format!(
            "plugin artifact redirect for {label} included an invalid Location header: {error}"
        ))
    })?;
    current_url.join(location).map_err(|error| {
        AppError::validation_refused(crate::error_reasons::plugins::ARTIFACT_REDIRECT_INCLUDED_INVALID_LOCATION_URL, format!(
            "plugin artifact redirect for {label} included an invalid Location URL: {error}"
        ))
    })
}

fn map_artifact_destination_error(error: scryer_outbound_http::OutboundDestinationError) -> AppError {
    use scryer_outbound_http::OutboundDestinationError as Error;
    use crate::error_reasons::plugins as reason;
    let code = match &error {
        Error::InvalidUrl { .. } | Error::UnsupportedScheme { .. } | Error::MissingHost { .. } | Error::EmbeddedCredentials { .. } => reason::URL_INVALID,
        Error::ForbiddenAddress { .. } | Error::BlockedLinkLocalOrMetadata { .. } => reason::ARTIFACT_DESTINATION_FORBIDDEN,
        Error::ResolveFailed { .. } | Error::NoResolvedAddresses { .. } => reason::ARTIFACT_HOST_UNAVAILABLE,
        Error::TrustBundle { .. } => reason::ARTIFACT_UNTRUSTED,
        // A reqwest builder failure is an internal transport invariant. Keep its
        // diagnostic without inventing a client-actionable cause.
        Error::ClientBuild { .. } => return AppError::Validation(error.to_string()),
    };
    AppError::validation_refused(code, error.to_string())
}
