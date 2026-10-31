use std::collections::HashSet;

use crate::{
    AppError, AppResult, AppUseCase, UiCatalogViewUpdate, UiDashboardBreakpoint, UiDashboardLayout,
    UiSettings, UiSettingsFacet, UiSettingsUpdate, UiTableColumnSetting, UiTableViewMode,
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

/// Widest dashboard grid a layout may declare.
const MAX_DASHBOARD_COLUMNS: i32 = 24;
/// Most panels one dashboard layout may place.
const MAX_DASHBOARD_ITEMS: usize = 64;
/// Longest dashboard panel id accepted.
const MAX_DASHBOARD_PANEL_ID_LEN: usize = 64;

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
                return Err(AppError::validation_refused(
                    crate::error_reasons::settings::CATALOG_VIEW_COLUMNS_INVALID,
                    "catalog view columns must match the saved device class and facet",
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

    /// Replaces the caller's dashboard layout for one breakpoint. Panel ids are
    /// opaque here: the web owns the panel registry.
    pub async fn set_my_dashboard_layout(
        &self,
        actor: &User,
        layout: UiDashboardLayout,
    ) -> AppResult<UiSettings> {
        validate_dashboard_layout(&layout)?;
        let settings = self
            .services
            .identity
            .ui_settings
            .set_dashboard_layout(&actor.id, layout)
            .await?;
        Ok(without_unknown_table_columns(settings))
    }

    /// Forgets the caller's dashboard layout for one breakpoint, or for every
    /// breakpoint when `breakpoint` is `None`, so the web falls back to its default.
    pub async fn reset_my_dashboard_layout(
        &self,
        actor: &User,
        breakpoint: Option<UiDashboardBreakpoint>,
    ) -> AppResult<UiSettings> {
        let settings = self
            .services
            .identity
            .ui_settings
            .reset_dashboard_layouts(&actor.id, breakpoint)
            .await?
            .unwrap_or_else(|| UiSettings::defaults_for_user(actor.id.clone()));
        Ok(without_unknown_table_columns(settings))
    }
}

fn validate_dashboard_layout(layout: &UiDashboardLayout) -> AppResult<()> {
    use crate::error_reasons::settings;

    if !(1..=MAX_DASHBOARD_COLUMNS).contains(&layout.columns) {
        return Err(AppError::validation_refused(
            settings::DASHBOARD_LAYOUT_COLUMNS_OUT_OF_RANGE,
            format!("dashboard layout columns must be between 1 and {MAX_DASHBOARD_COLUMNS}"),
        ));
    }
    if layout.items.len() > MAX_DASHBOARD_ITEMS {
        return Err(AppError::validation_refused(
            settings::DASHBOARD_LAYOUT_TOO_MANY_ITEMS,
            format!("a dashboard layout may hold at most {MAX_DASHBOARD_ITEMS} panels"),
        ));
    }
    let mut seen = HashSet::new();
    for item in &layout.items {
        let panel_id = item.panel_id.as_str();
        if panel_id.is_empty()
            || panel_id.len() > MAX_DASHBOARD_PANEL_ID_LEN
            || !panel_id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(AppError::validation_refused(
                settings::DASHBOARD_PANEL_ID_INVALID,
                format!(
                    "dashboard panel ids must be 1 to {MAX_DASHBOARD_PANEL_ID_LEN} lowercase letters, digits or hyphens"
                ),
            ));
        }
        if !seen.insert(panel_id) {
            return Err(AppError::validation_refused(
                settings::DASHBOARD_PANEL_ID_DUPLICATED,
                format!("dashboard panel {panel_id:?} appears more than once"),
            ));
        }
        if item.x < 0 || item.y < 0 {
            return Err(AppError::validation_refused(
                settings::DASHBOARD_ITEM_POSITION_OUT_OF_RANGE,
                format!("dashboard panel {panel_id:?} must not start at a negative position"),
            ));
        }
        if item.w < 1 || item.h < 1 {
            return Err(AppError::validation_refused(
                settings::DASHBOARD_ITEM_SIZE_OUT_OF_RANGE,
                format!("dashboard panel {panel_id:?} must be at least one cell wide and tall"),
            ));
        }
        if i64::from(item.x) + i64::from(item.w) > i64::from(layout.columns) {
            return Err(AppError::validation_refused(
                settings::DASHBOARD_ITEM_EXCEEDS_COLUMNS,
                format!(
                    "dashboard panel {panel_id:?} extends past column {}",
                    layout.columns
                ),
            ));
        }
    }
    Ok(())
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
        return Err(AppError::validation_refused(
            crate::error_reasons::settings::LANGUAGE_OUT_OF_RANGE,
            format!(
                "language must be a language code of at most {MAX_LANGUAGE_CODE_LEN} letters, digits or hyphens"
            ),
        ));
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
        return Err(AppError::validation_refused(
            crate::error_reasons::settings::MUST_RRGGBB_HEX_COLOR,
            format!("{field_name} must be a #RRGGBB hex color"),
        ));
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
            return Err(AppError::validation_refused(
                crate::error_reasons::settings::UI_TABLE_COLUMN_SETTING_COLUMN_DUPLICATED,
                format!(
                    "duplicate UI table column setting for {} {} {} column {}",
                    column.device_class.as_str(),
                    column.facet.as_str(),
                    column.table_view_mode.as_str(),
                    column.column_id
                ),
            ));
        }
        if column.column_order < 0 {
            return Err(AppError::validation_refused(
                crate::error_reasons::settings::TABLE_COLUMN_ORDER_OUT_OF_RANGE,
                "table column order must be greater than or equal to 0",
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
        return Err(AppError::validation_refused(
            crate::error_reasons::settings::TABLE_COLUMN_UNSUPPORTED,
            format!(
                "unsupported {} table column {:?}",
                column.table_view_mode.as_str(),
                column.column_id
            ),
        ));
    }
    if column.facet == UiSettingsFacet::Movies && column.column_id == "episodes" {
        return Err(AppError::validation_refused(
            crate::error_reasons::settings::MOVIES_TABLE_SETTINGS_CANNOT_INCLUDE_EPISODES_COLUMN,
            "movies table settings cannot include the episodes column",
        ));
    }
    Ok(())
}

fn without_unknown_table_columns(mut settings: UiSettings) -> UiSettings {
    settings.table_columns.retain(is_known_table_column);
    settings
}
