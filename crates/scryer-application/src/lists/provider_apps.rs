//! Optional direct provider registrations, held in encrypted system settings.
use super::account_transport::ListProviderAppConfig;
use crate::{AppError, AppResult, AppUseCase};
use scryer_domain::{AppPermission, User};
const KEY: &str = "lists.provider_apps";
const PROVIDERS: [&str; 3] = ["trakt", "anilist", "mal"];
#[derive(Clone)]
pub struct ListProviderAppView {
    pub provider: String,
    pub client_id: Option<String>,
    pub redirect_uri: Option<String>,
    pub client_secret_set: bool,
    pub enabled: bool,
}
fn view(provider: &str, app: Option<&ListProviderAppConfig>) -> ListProviderAppView {
    ListProviderAppView {
        provider: provider.into(),
        client_id: app.map(|app| app.client_id.clone()),
        redirect_uri: app.map(|app| app.redirect_uri.clone()),
        client_secret_set: app.is_some_and(|app| !app.client_secret.is_empty()),
        enabled: app.is_some(),
    }
}
impl AppUseCase {
    pub(crate) async fn list_provider_app_config(
        &self,
        provider: &str,
    ) -> AppResult<Option<ListProviderAppConfig>> {
        if !PROVIDERS.contains(&provider) {
            return Ok(None);
        }
        Ok(self
            .read_setting_json_value::<Option<ListProviderAppConfig>>(KEY, Some(provider))
            .await?
            .flatten())
    }
    pub async fn list_provider_apps(&self, actor: &User) -> AppResult<Vec<ListProviderAppView>> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        self.require_lists_enabled().await?;
        let mut result = Vec::new();
        for provider in PROVIDERS {
            result.push(view(
                provider,
                self.list_provider_app_config(provider).await?.as_ref(),
            ));
        }
        Ok(result)
    }
    pub async fn update_list_provider_app(
        &self,
        actor: &User,
        provider: &str,
        client_id: Option<String>,
        client_secret: Option<String>,
        redirect_uri: Option<String>,
        enabled: bool,
    ) -> AppResult<ListProviderAppView> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        self.require_lists_enabled().await?;
        if !PROVIDERS.contains(&provider) {
            return Err(AppError::Validation(
                "this provider does not support an instance app".into(),
            ));
        }
        let app = if enabled {
            let existing = self
                .list_provider_app_config(provider)
                .await?
                .unwrap_or_default();
            let app = ListProviderAppConfig {
                client_id: client_id.unwrap_or(existing.client_id).trim().into(),
                client_secret: client_secret
                    .unwrap_or(existing.client_secret)
                    .trim()
                    .into(),
                redirect_uri: redirect_uri.unwrap_or(existing.redirect_uri).trim().into(),
                access_token: None,
            };
            let redirect = url::Url::parse(&app.redirect_uri).map_err(|_| {
                AppError::Validation("a provider app needs a valid redirect URI".into())
            })?;
            if app.client_id.is_empty()
                || app.client_id.len() > 512
                || app.client_secret.len() > 4096
                || app.redirect_uri.len() > 2048
                || (provider != "mal" && app.client_secret.is_empty())
                || !matches!(redirect.scheme(), "http" | "https")
                || redirect.host_str().is_none()
                || !redirect.username().is_empty()
                || redirect.password().is_some()
                || redirect.fragment().is_some()
                || redirect.query().is_some()
                || !redirect.path().ends_with("/lists/oauth/callback")
            {
                return Err(AppError::Validation(
                    "a provider app needs a client ID, secret and valid account callback URI"
                        .into(),
                ));
            }
            Some(app)
        } else {
            None
        };
        self.upsert_scoped_system_setting_json(KEY, provider, &app, Some(actor.id.clone()))
            .await?;
        Ok(view(provider, app.as_ref()))
    }
}
