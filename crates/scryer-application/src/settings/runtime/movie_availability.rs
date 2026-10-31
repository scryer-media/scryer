pub const DEFAULT_MOVIE_AVAILABILITY: &str = "released";
pub const DEFAULT_MOVIE_RELEASE_MARKET: &str = "US";

/// Assigned ISO 3166-1 alpha-2 country codes used for regional release dates.
fn is_movie_release_market(market: &str) -> bool {
    const COUNTRIES: &str = "AD AE AF AG AI AL AM AO AQ AR AS AT AU AW AX AZ \
        BA BB BD BE BF BG BH BI BJ BL BM BN BO BQ BR BS BT BV BW BY BZ \
        CA CC CD CF CG CH CI CK CL CM CN CO CR CU CV CW CX CY CZ \
        DE DJ DK DM DO DZ EC EE EG EH ER ES ET FI FJ FK FM FO FR \
        GA GB GD GE GF GG GH GI GL GM GN GP GQ GR GS GT GU GW GY \
        HK HM HN HR HT HU ID IE IL IM IN IO IQ IR IS IT JE JM JO JP \
        KE KG KH KI KM KN KP KR KW KY KZ LA LB LC LI LK LR LS LT LU LV LY \
        MA MC MD ME MF MG MH MK ML MM MN MO MP MQ MR MS MT MU MV MW MX MY MZ \
        NA NC NE NF NG NI NL NO NP NR NU NZ OM PA PE PF PG PH PK PL PM PN PR PS PT PW PY \
        QA RE RO RS RU RW SA SB SC SD SE SG SH SI SJ SK SL SM SN SO SR SS ST SV SX SY SZ \
        TC TD TF TG TH TJ TK TL TM TN TO TR TT TV TW TZ UA UG UM US UY UZ \
        VA VC VE VG VI VN VU WF WS YE YT ZA ZM ZW";
    COUNTRIES
        .split_ascii_whitespace()
        .any(|code| code == market)
}

fn normalize_movie_default_availability(value: &str) -> AppResult<String> {
    let value = value.trim().to_ascii_lowercase();
    if matches!(value.as_str(), "announced" | "in_cinemas" | "released") {
        Ok(value)
    } else {
        Err(AppError::validation_refused(crate::error_reasons::settings::DEFAULT_MOVIE_AVAILABILITY_INVALID, "default movie availability must be announced, in_cinemas, or released"))
    }
}

fn normalize_movie_release_market(value: &str) -> AppResult<String> {
    let market = value.trim().to_ascii_uppercase();
    if is_movie_release_market(&market) {
        Ok(market)
    } else {
        Err(AppError::validation_refused(crate::error_reasons::settings::MOVIE_RELEASE_MARKET_INVALID, "movie release market must be a two-letter ISO-3166 country code"))
    }
}

/// The movie availability gate settings one scope sets; `None` inherits.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MovieAvailabilityOverrides {
    pub default_availability: Option<String>,
    pub release_market: Option<String>,
    pub delay_days: Option<i32>,
}

impl MovieAvailabilityOverrides {
    /// Validates and normalizes every set field.
    pub(crate) fn normalized(self) -> AppResult<Self> {
        Ok(Self {
            default_availability: normalize_optional_string(self.default_availability)
                .map(|value| normalize_movie_default_availability(&value))
                .transpose()?,
            release_market: normalize_optional_string(self.release_market)
                .map(|value| normalize_movie_release_market(&value))
                .transpose()?,
            delay_days: self.delay_days,
        })
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.default_availability.is_none()
            && self.release_market.is_none()
            && self.delay_days.is_none()
    }
}

/// Movie availability gate settings with every field resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovieAvailabilitySettings {
    /// Threshold for a movie without its own: `announced`, `in_cinemas` or `released`.
    pub default_availability: String,
    /// ISO-3166-1 alpha-2 market whose release dates are read first.
    pub release_market: String,
    /// Signed whole-day offset from the selected release date.
    pub delay_days: i32,
}

impl Default for MovieAvailabilitySettings {
    fn default() -> Self {
        Self {
            default_availability: DEFAULT_MOVIE_AVAILABILITY.to_string(),
            release_market: DEFAULT_MOVIE_RELEASE_MARKET.to_string(),
            delay_days: 0,
        }
    }
}

impl MovieAvailabilitySettings {
    fn overridden_by(&self, overrides: &MovieAvailabilityOverrides) -> Self {
        Self {
            default_availability: overrides
                .default_availability
                .clone()
                .unwrap_or_else(|| self.default_availability.clone()),
            release_market: overrides
                .release_market
                .clone()
                .unwrap_or_else(|| self.release_market.clone()),
            delay_days: overrides.delay_days.unwrap_or(self.delay_days),
        }
    }

    /// The gate policy for a movie with this threshold of its own, if any.
    pub(crate) fn policy<'a>(
        &'a self,
        minimum_availability: Option<&'a str>,
    ) -> crate::acquisition::targets::MovieAvailabilityPolicy<'a> {
        crate::acquisition::targets::MovieAvailabilityPolicy {
            minimum_availability,
            default_minimum_availability: &self.default_availability,
            release_market: &self.release_market,
            delay_days: self.delay_days,
        }
    }
}

/// The Movie facet settings and each Movie library's effective settings,
/// loaded once for a pass over many titles. Precedence is title, then
/// library, then facet: the title contributes only its own threshold.
#[derive(Debug, Clone, Default)]
pub(crate) struct MovieAvailabilityPolicies {
    facet: MovieAvailabilitySettings,
    libraries: HashMap<String, MovieAvailabilitySettings>,
}

impl MovieAvailabilityPolicies {
    /// The effective settings for titles in this library; a library with no
    /// overrides, or one not loaded, takes the facet settings.
    pub(crate) fn for_library(&self, library_id: &str) -> &MovieAvailabilitySettings {
        self.libraries.get(library_id).unwrap_or(&self.facet)
    }

    pub(crate) fn facet(&self) -> &MovieAvailabilitySettings {
        &self.facet
    }

    /// Libraries whose effective release market differs between the two loads.
    fn libraries_with_changed_market(&self, other: &Self) -> Vec<String> {
        let mut library_ids = self
            .libraries
            .keys()
            .chain(other.libraries.keys())
            .filter(|library_id| {
                self.for_library(library_id).release_market
                    != other.for_library(library_id).release_market
            })
            .cloned()
            .collect::<Vec<_>>();
        library_ids.sort();
        library_ids.dedup();
        library_ids
    }
}

impl AppUseCase {
    /// The Movie facet's availability gate settings, for callers that report
    /// them alongside the acquisition settings.
    pub async fn get_movie_availability_facet_settings(
        &self,
        actor: &User,
    ) -> AppResult<MovieAvailabilitySettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        self.movie_availability_facet_settings().await
    }

    pub(crate) async fn movie_availability_facet_settings(
        &self,
    ) -> AppResult<MovieAvailabilitySettings> {
        let scope_id = Some(MediaFacet::Movie.as_str());
        let defaults = MovieAvailabilitySettings::default();
        Ok(MovieAvailabilitySettings {
            default_availability: self
                .read_setting_string_value(MOVIE_DEFAULT_AVAILABILITY_KEY, scope_id)
                .await?
                .and_then(|value| normalize_movie_default_availability(&value).ok())
                .unwrap_or(defaults.default_availability),
            release_market: self
                .read_setting_string_value(MOVIE_RELEASE_MARKET_KEY, scope_id)
                .await?
                .and_then(|value| normalize_movie_release_market(&value).ok())
                .unwrap_or(defaults.release_market),
            delay_days: self
                .read_setting_i64_value(MOVIE_AVAILABILITY_DELAY_DAYS_KEY, scope_id)
                .await?
                .and_then(|value| i32::try_from(value).ok())
                .unwrap_or(defaults.delay_days),
        })
    }

    pub(crate) async fn movie_availability_library_overrides(
        &self,
        library_id: &str,
    ) -> AppResult<MovieAvailabilityOverrides> {
        let scope_id = Some(library_id);
        Ok(MovieAvailabilityOverrides {
            default_availability: self
                .read_setting_string_value_explicit(MOVIE_DEFAULT_AVAILABILITY_KEY, scope_id)
                .await?
                .and_then(|value| normalize_movie_default_availability(&value).ok()),
            release_market: self
                .read_setting_string_value_explicit(MOVIE_RELEASE_MARKET_KEY, scope_id)
                .await?
                .and_then(|value| normalize_movie_release_market(&value).ok()),
            delay_days: self
                .read_setting_string_value_explicit(MOVIE_AVAILABILITY_DELAY_DAYS_KEY, scope_id)
                .await?
                .and_then(|value| value.trim().parse::<i32>().ok()),
        })
    }

    /// The facet settings plus every Movie library's overrides.
    pub(crate) async fn movie_availability_policies(&self) -> AppResult<MovieAvailabilityPolicies> {
        let facet = self.movie_availability_facet_settings().await?;
        let mut libraries = HashMap::new();
        for library in self
            .services
            .catalog
            .libraries
            .list(Some(MediaFacet::Movie))
            .await?
        {
            let overrides = self
                .movie_availability_library_overrides(&library.id)
                .await?;
            if !overrides.is_empty() {
                libraries.insert(library.id, facet.overridden_by(&overrides));
            }
        }
        Ok(MovieAvailabilityPolicies { facet, libraries })
    }

    /// The effective settings for one library: its overrides over the facet.
    pub(crate) async fn movie_availability_settings_for_library(
        &self,
        library_id: &str,
    ) -> AppResult<MovieAvailabilitySettings> {
        let facet = self.movie_availability_facet_settings().await?;
        let overrides = self
            .movie_availability_library_overrides(library_id)
            .await?;
        Ok(facet.overridden_by(&overrides))
    }

    /// The effective availability gate settings for one title: its own
    /// threshold, then its library's overrides, then the Movie facet. The
    /// returned threshold is the one the gate applies.
    pub(crate) async fn effective_movie_availability_policy_for_title(
        &self,
        title: &scryer_domain::Title,
    ) -> AppResult<MovieAvailabilitySettings> {
        let mut settings = self
            .movie_availability_settings_for_library(&title.library_id)
            .await?;
        if let Some(threshold) = title
            .min_availability
            .as_deref()
            .and_then(|value| normalize_movie_default_availability(value).ok())
        {
            settings.default_availability = threshold;
        }
        Ok(settings)
    }

    /// Writes each set field at this scope. With `replace`, an unset field's
    /// value is removed so the scope inherits it again. Returns the keys
    /// written or removed.
    pub(crate) async fn write_movie_availability_values(
        &self,
        scope_id: &str,
        values: &MovieAvailabilityOverrides,
        replace: bool,
        actor_id: &str,
    ) -> AppResult<Vec<String>> {
        let mut changed_keys = Vec::new();
        let fields = [
            (
                MOVIE_DEFAULT_AVAILABILITY_KEY,
                values
                    .default_availability
                    .as_ref()
                    .map(serde_json::to_value),
            ),
            (
                MOVIE_RELEASE_MARKET_KEY,
                values.release_market.as_ref().map(serde_json::to_value),
            ),
            (
                MOVIE_AVAILABILITY_DELAY_DAYS_KEY,
                values.delay_days.as_ref().map(serde_json::to_value),
            ),
        ];
        for (key, value) in fields {
            match value {
                Some(value) => {
                    let value = value.map_err(|error| AppError::Repository(error.to_string()))?;
                    self.upsert_scoped_system_setting_json(
                        key,
                        scope_id,
                        &value,
                        Some(actor_id.to_string()),
                    )
                    .await?;
                }
                None if replace => self.delete_scoped_system_setting(key, scope_id).await?,
                None => continue,
            }
            changed_keys.push(key.to_string());
        }
        Ok(changed_keys)
    }

    /// Marks for rehydration the movies in every library whose effective
    /// release market differs between the two loads, so their release dates
    /// are fetched for the market now in force.
    pub(crate) async fn queue_movie_release_market_refresh_where_changed(
        &self,
        before: &MovieAvailabilityPolicies,
        after: &MovieAvailabilityPolicies,
    ) -> AppResult<()> {
        let library_ids = if before.facet.release_market != after.facet.release_market {
            // Libraries without an override follow the facet; the loads only
            // list libraries that override something, so take every library.
            self.services
                .catalog
                .libraries
                .list(Some(MediaFacet::Movie))
                .await?
                .into_iter()
                .map(|library| library.id)
                .filter(|library_id| {
                    before.for_library(library_id).release_market
                        != after.for_library(library_id).release_market
                })
                .collect::<Vec<_>>()
        } else {
            before.libraries_with_changed_market(after)
        };
        self.queue_movie_release_market_refresh_for_libraries(&library_ids)
            .await
    }
}
