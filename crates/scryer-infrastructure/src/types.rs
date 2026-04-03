#[derive(Debug, Clone)]
pub struct MigrationStatus {
    pub migration_key: String,
    pub migration_checksum: String,
    pub applied_at: String,
    pub success: bool,
    pub error_message: Option<String>,
    pub runtime_version: String,
}

#[derive(Debug, Clone)]
pub struct WorkflowOperationRecord {
    pub id: String,
    pub operation_type: String,
    pub status: String,
    pub job_key: Option<String>,
    pub trigger_source: Option<String>,
    pub actor_user_id: Option<String>,
    pub title_id: Option<String>,
    pub collection_id: Option<String>,
    pub episode_id: Option<String>,
    pub release_id: Option<String>,
    pub media_file_id: Option<String>,
    pub external_reference: Option<String>,
    pub progress_json: Option<String>,
    pub summary_json: Option<String>,
    pub summary_text: Option<String>,
    pub error_text: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct LibraryProbeSignatureRecord {
    pub title_id: String,
    pub path: String,
    pub probe_signature_scheme: Option<String>,
    pub probe_signature_value: Option<String>,
    pub last_probed_at: Option<String>,
    pub last_changed_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct ReleaseDownloadFailureSignatureRecord {
    pub source_hint: Option<String>,
    pub source_title: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TitleReleaseBlocklistRecord {
    pub source_hint: Option<String>,
    pub source_title: Option<String>,
    pub error_message: Option<String>,
    pub attempted_at: String,
}

#[derive(Debug, Clone)]
pub struct SettingsDefinitionRecord {
    pub id: String,
    pub category: String,
    pub scope: String,
    pub key_name: String,
    pub data_type: String,
    pub default_value_json: String,
    pub is_sensitive: bool,
    pub validation_json: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct SettingsValueRecord {
    pub definition_id: String,
    pub category: String,
    pub scope: String,
    pub key_name: String,
    pub data_type: String,
    pub default_value_json: String,
    pub is_sensitive: bool,
    pub validation_json: Option<String>,
    pub effective_value_json: String,
    pub value_json: Option<String>,
    pub source: Option<String>,
    pub scope_id: Option<String>,
    pub updated_by_user_id: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

impl SettingsValueRecord {
    pub fn has_override(&self) -> bool {
        self.value_json.is_some()
    }
}

#[derive(Debug, Clone)]
pub struct SettingDefinitionSeed {
    pub category: String,
    pub scope: String,
    pub key_name: String,
    pub data_type: String,
    pub default_value_json: String,
    pub is_sensitive: bool,
    pub validation_json: Option<String>,
}

#[derive(Debug, Clone)]
pub struct EmbeddedMigrationDescriptor {
    pub filename: String,
    pub key: String,
    pub checksum: String,
}

#[derive(Clone, Copy, Debug, Default)]
pub enum MigrationMode {
    ValidateOnly,
    #[default]
    Apply,
}

pub(crate) fn sqlite_url_with_create(path: &str) -> String {
    if path.starts_with("sqlite:") {
        if path.starts_with("sqlite://:memory:") {
            let with_mode = if path.contains("?mode=") {
                path.to_string()
            } else if path.contains('?') {
                format!("{path}&mode=memory")
            } else {
                format!("{path}?mode=memory")
            };

            let with_cache = if with_mode.contains("cache=shared") {
                with_mode
            } else if with_mode.contains('?') {
                format!("{with_mode}&cache=shared")
            } else {
                format!("{with_mode}?cache=shared")
            };

            return with_cache.replace("sqlite://:memory:", "sqlite://file::memory:");
        }

        if path.contains("?mode=") {
            return path.to_string();
        }

        return if path.contains('?') {
            format!("{path}&mode=rwc")
        } else {
            format!("{path}?mode=rwc")
        };
    }

    format!("sqlite://{path}?mode=rwc")
}
