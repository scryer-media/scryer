use std::collections::HashSet;

use crate::{
    AppError, AppResult, AppUseCase, UiCatalogViewUpdate, UiSettings, UiSettingsFacet,
    UiSettingsUpdate, UiTableColumnSetting, UiTableViewMode,
};
use scryer_domain::User;

/// Columns every catalog table offers. Ids outside these lists are rejected on
/// save and dropped on read, so a column removed in a later version cannot
/// block saving the rest of the settings.
const CATALOG_TABLE_COLUMNS: &[&str] = &[
    "library",
    "monitored",
    "quality",
    "profile",
    "episodes",
    "size",
    "added",
    "year",
    "runtime",
    "status",
    "root",
    "popularity",
    "resolution",
    "hdr",
    "audioCodec",
    "ratingScryer",
    "ratingImdb",
    "ratingRottenTomatoes",
    "ratingPopcornmeter",
    "ratingMetacritic",
    "ratingMetacriticUser",
    "ratingLetterboxd",
    "ratingTmdb",
    "ratingTrakt",
    "ratingMyanimelist",
    "ratingAnilist",
    "ratingAnidb",
    "ratingMdblist",
    "actions",
];
const COMPACT_TABLE_FIXED_COLUMNS: &[&str] = &["select", "name"];
const POSTER_TABLE_FIXED_COLUMNS: &[&str] = &["poster", "name"];

/// Longest interface language code accepted, e.g. `zh-HK` or `pt-BR`.
const MAX_LANGUAGE_CODE_LEN: usize = 16;

impl AppUseCase {
    pub async fn get_my_ui_settings(&self, actor: &User) -> AppResult<UiSettings> {
        let settings = self
            .services
            .identity
            .ui_settings
            .get_by_user_id(&actor.id)
            .await?
            .unwrap_or_else(|| UiSettings::defaults_for_user(actor.id.clone()));
        Ok(without_unknown_table_columns(settings))
    }

    pub async fn set_my_ui_settings(
        &self,
        actor: &User,
        input: UiSettingsUpdate,
    ) -> AppResult<UiSettings> {
        let input = validate_ui_settings_update(input)?;
        let settings = self
            .services
            .identity
            .ui_settings
            .upsert(&actor.id, input)
            .await?;
        Ok(without_unknown_table_columns(settings))
    }

    /// Saves the catalog layout the caller chose on one device class for one facet.
    pub async fn set_my_catalog_view(
        &self,
        actor: &User,
        mut update: UiCatalogViewUpdate,
    ) -> AppResult<UiSettings> {
        if let Some(columns) = update.columns.as_mut() {
            if columns.iter().any(|column| {
                column.device_class != update.device_class || column.facet != update.facet
            }) {
                return Err(AppError::Validation(
                    "catalog view columns must match the saved device class and facet".into(),
                ));
            }
            validate_table_columns(columns)?;
        }
        let settings = self
            .services
            .identity
            .ui_settings
            .set_catalog_view(&actor.id, update)
            .await?;
        Ok(without_unknown_table_columns(settings))
    }
}

fn validate_ui_settings_update(mut input: UiSettingsUpdate) -> AppResult<UiSettingsUpdate> {
    input.highlight_color = normalize_optional_hex_color(input.highlight_color, "highlightColor")?;
    input.secondary_color = normalize_optional_hex_color(input.secondary_color, "secondaryColor")?;
    if let Some(language) = input.language.take() {
        input.language = Some(normalize_optional_language(language)?);
    }
    if let Some(columns) = input.table_columns.as_mut() {
        validate_table_columns(columns)?;
    }
    Ok(input)
}

/// Accepts a short language code (letters, digits and hyphens) and clears the
/// preference when it is blank. Whether the interface has that translation is
/// the client's concern: it falls back when it does not.
fn normalize_optional_language(value: Option<String>) -> AppResult<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.len() > MAX_LANGUAGE_CODE_LEN
        || !value.starts_with(|ch: char| ch.is_ascii_alphabetic())
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
    {
        return Err(AppError::Validation(format!(
            "language must be a language code of at most {MAX_LANGUAGE_CODE_LEN} letters, digits or hyphens"
        )));
    }
    Ok(Some(value.to_string()))
}

fn normalize_optional_hex_color(
    value: Option<String>,
    field_name: &str,
) -> AppResult<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.len() != 7
        || !value.starts_with('#')
        || !value[1..].chars().all(|ch| ch.is_ascii_hexdigit())
    {
        return Err(AppError::Validation(format!(
            "{field_name} must be a #RRGGBB hex color"
        )));
    }
    Ok(Some(value.to_ascii_lowercase()))
}

fn validate_table_columns(columns: &mut [UiTableColumnSetting]) -> AppResult<()> {
    let mut seen = HashSet::new();
    for column in columns.iter() {
        validate_table_column(column)?;
        let key = (
            column.device_class,
            column.facet,
            column.table_view_mode,
            column.column_id.as_str(),
        );
        if !seen.insert(key) {
            return Err(AppError::Validation(format!(
                "duplicate UI table column setting for {} {} {} column {}",
                column.device_class.as_str(),
                column.facet.as_str(),
                column.table_view_mode.as_str(),
                column.column_id
            )));
        }
        if column.column_order < 0 {
            return Err(AppError::Validation(
                "table column order must be greater than or equal to 0".into(),
            ));
        }
    }

    columns.sort_by(|left, right| {
        left.device_class
            .as_str()
            .cmp(right.device_class.as_str())
            .then_with(|| left.facet.as_str().cmp(right.facet.as_str()))
            .then_with(|| {
                left.table_view_mode
                    .as_str()
                    .cmp(right.table_view_mode.as_str())
            })
            .then_with(|| left.column_order.cmp(&right.column_order))
            .then_with(|| left.column_id.cmp(&right.column_id))
    });

    Ok(())
}

fn is_known_table_column(column: &UiTableColumnSetting) -> bool {
    let fixed_columns = match column.table_view_mode {
        UiTableViewMode::Compact => COMPACT_TABLE_FIXED_COLUMNS,
        UiTableViewMode::PosterTable => POSTER_TABLE_FIXED_COLUMNS,
    };
    let column_id = column.column_id.as_str();
    fixed_columns.contains(&column_id) || CATALOG_TABLE_COLUMNS.contains(&column_id)
}

fn validate_table_column(column: &UiTableColumnSetting) -> AppResult<()> {
    if !is_known_table_column(column) {
        return Err(AppError::Validation(format!(
            "unsupported {} table column {:?}",
            column.table_view_mode.as_str(),
            column.column_id
        )));
    }
    if column.facet == UiSettingsFacet::Movies && column.column_id == "episodes" {
        return Err(AppError::Validation(
            "movies table settings cannot include the episodes column".into(),
        ));
    }
    Ok(())
}

fn without_unknown_table_columns(mut settings: UiSettings) -> UiSettings {
    settings.table_columns.retain(is_known_table_column);
    settings
}
