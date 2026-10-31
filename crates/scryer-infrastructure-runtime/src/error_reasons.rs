//! Stable reasons for conditions raised at this infrastructure boundary.

/// backup blob payload is invalid base64: {error}
pub const DATASTORE_BACKUP_BLOB_BASE64_INVALID: &str = "DATASTORE_BACKUP_BLOB_BASE64_INVALID";

/// backup blob payload is missing base64 bytes
pub const DATASTORE_BACKUP_BLOB_BYTES_MISSING: &str = "DATASTORE_BACKUP_BLOB_BYTES_MISSING";

/// backup bundle table set does not match the current restore catalog: missing [{}], unexpected []
pub const DATASTORE_BACKUP_TABLE_SET_MISMATCH: &str = "DATASTORE_BACKUP_TABLE_SET_MISMATCH";

/// backup row for {table}:{line_number} is not an object
pub const DATASTORE_BACKUP_ROW_NOT_OBJECT: &str = "DATASTORE_BACKUP_ROW_NOT_OBJECT";

/// backup row contains an integer outside SQLite i64 range
pub const DATASTORE_BACKUP_INTEGER_OUT_OF_RANGE: &str = "DATASTORE_BACKUP_INTEGER_OUT_OF_RANGE";

/// backup row contains an unsupported numeric value
pub const DATASTORE_BACKUP_NUMBER_UNSUPPORTED: &str = "DATASTORE_BACKUP_NUMBER_UNSUPPORTED";

/// backup row contains an unsupported PostgreSQL bytea value
pub const DATASTORE_BACKUP_BYTEA_UNSUPPORTED: &str = "DATASTORE_BACKUP_BYTEA_UNSUPPORTED";

/// backup table payload missing for {table}: {error}
pub const DATASTORE_BACKUP_TABLE_MISSING: &str = "DATASTORE_BACKUP_TABLE_MISSING";

/// failed to compact restored release explanation: {error}
pub const DATASTORE_RESTORED_EXPLANATION_COMPACTION_FAILED: &str =
    "DATASTORE_RESTORED_EXPLANATION_COMPACTION_FAILED";

/// failed to encode restored domain event payload: {error}
pub const DATASTORE_RESTORED_EVENT_INVALID: &str = "DATASTORE_RESTORED_EVENT_INVALID";

/// failed to encode restored release explanation: {error}
pub const DATASTORE_RESTORED_EXPLANATION_INVALID: &str = "DATASTORE_RESTORED_EXPLANATION_INVALID";

/// failed to import backup row for {table}:{line_number}: {error}
pub const DATASTORE_BACKUP_ROW_IMPORT_FAILED: &str = "DATASTORE_BACKUP_ROW_IMPORT_FAILED";

/// failed to read backup row {table}:{line_number}: {error}
pub const DATASTORE_BACKUP_ROW_UNREADABLE: &str = "DATASTORE_BACKUP_ROW_UNREADABLE";

/// failed to read SCRYER_DB_PASSWORD_FILE {}: {error}
pub const DATASTORE_PASSWORD_FILE_UNREADABLE: &str = "DATASTORE_PASSWORD_FILE_UNREADABLE";

/// failed to set PostgreSQL password on datastore URL
pub const DATASTORE_POSTGRES_PASSWORD_INVALID: &str = "DATASTORE_POSTGRES_PASSWORD_INVALID";

/// failed to set PostgreSQL username on datastore URL
pub const DATASTORE_POSTGRES_USERNAME_INVALID: &str = "DATASTORE_POSTGRES_USERNAME_INVALID";

/// invalid backup row for {table}:{line_number}: {error}
pub const DATASTORE_BACKUP_ROW_INVALID: &str = "DATASTORE_BACKUP_ROW_INVALID";

/// invalid SCRYER_DB_URL: {error}
pub const DATASTORE_URL_INVALID: &str = "DATASTORE_URL_INVALID";

/// PostgreSQL datastore requires SCRYER_DB_PASSWORD, SCRYER_DB_PASSWORD_FILE, or a URL password
pub const DATASTORE_POSTGRES_PASSWORD_REQUIRED: &str = "DATASTORE_POSTGRES_PASSWORD_REQUIRED";

/// PostgreSQL datastore requires SCRYER_DB_USER or a URL username
pub const DATASTORE_POSTGRES_USERNAME_REQUIRED: &str = "DATASTORE_POSTGRES_USERNAME_REQUIRED";

/// PostgreSQL datastore URL must include a database name
pub const DATASTORE_POSTGRES_DATABASE_REQUIRED: &str = "DATASTORE_POSTGRES_DATABASE_REQUIRED";

/// PostgreSQL datastore URL must include an explicit sslmode
pub const DATASTORE_POSTGRES_SSLMODE_REQUIRED: &str = "DATASTORE_POSTGRES_SSLMODE_REQUIRED";

/// PostgreSQL datastore URL must include a host
pub const DATASTORE_POSTGRES_HOST_REQUIRED: &str = "DATASTORE_POSTGRES_HOST_REQUIRED";

/// prepared backup directory restore is only supported for PostgreSQL
pub const DATASTORE_DIRECTORY_RESTORE_ENGINE_UNSUPPORTED: &str =
    "DATASTORE_DIRECTORY_RESTORE_ENGINE_UNSUPPORTED";

/// present restored release explanation encoded as null
pub const DATASTORE_RESTORED_EXPLANATION_NULL: &str = "DATASTORE_RESTORED_EXPLANATION_NULL";

/// restored database failed foreign key validation: {}
pub const DATASTORE_BACKUP_FOREIGN_KEY_INVALID: &str = "DATASTORE_BACKUP_FOREIGN_KEY_INVALID";

/// restored {field} is invalid JSON: {error}
pub const DATASTORE_RESTORED_JSON_INVALID: &str = "DATASTORE_RESTORED_JSON_INVALID";

/// restored table {table} row count mismatch: expected {expected_rows}, got {actual_rows}
pub const DATASTORE_RESTORED_TABLE_ROW_COUNT_MISMATCH: &str =
    "DATASTORE_RESTORED_TABLE_ROW_COUNT_MISMATCH";

/// restored {field} is unexpectedly null
pub const DATASTORE_RESTORED_REQUIRED_VALUE_NULL: &str = "DATASTORE_RESTORED_REQUIRED_VALUE_NULL";

/// SCRYER_DB_PASSWORD_FILE did not contain a password
pub const DATASTORE_PASSWORD_FILE_EMPTY: &str = "DATASTORE_PASSWORD_FILE_EMPTY";

/// unsupported datastore URL scheme '{scheme}'; expected sqlite, postgres, or postgresql
pub const DATASTORE_URL_SCHEME_UNSUPPORTED: &str = "DATASTORE_URL_SCHEME_UNSUPPORTED";

/// unsupported PostgreSQL sslmode '{sslmode}'; expected disable, prefer, require, verify-ca, or verify-full
pub const DATASTORE_POSTGRES_SSLMODE_UNSUPPORTED: &str = "DATASTORE_POSTGRES_SSLMODE_UNSUPPORTED";

/// The backup row is missing a required column.
pub const DATASTORE_BACKUP_REQUIRED_COLUMN_MISSING: &str =
    "DATASTORE_BACKUP_REQUIRED_COLUMN_MISSING";

/// The backup row contains a blank required timestamp.
pub const DATASTORE_BACKUP_TIMESTAMP_BLANK: &str = "DATASTORE_BACKUP_TIMESTAMP_BLANK";

/// The backup row contains null in a required column.
pub const DATASTORE_BACKUP_REQUIRED_COLUMN_NULL: &str = "DATASTORE_BACKUP_REQUIRED_COLUMN_NULL";

#[cfg(test)]
const ALL: &[&str] = &[
    DATASTORE_BACKUP_REQUIRED_COLUMN_MISSING,
    DATASTORE_BACKUP_TIMESTAMP_BLANK,
    DATASTORE_BACKUP_REQUIRED_COLUMN_NULL,
    DATASTORE_BACKUP_BLOB_BASE64_INVALID,
    DATASTORE_BACKUP_BLOB_BYTES_MISSING,
    DATASTORE_BACKUP_TABLE_SET_MISMATCH,
    DATASTORE_BACKUP_ROW_NOT_OBJECT,
    DATASTORE_BACKUP_INTEGER_OUT_OF_RANGE,
    DATASTORE_BACKUP_NUMBER_UNSUPPORTED,
    DATASTORE_BACKUP_BYTEA_UNSUPPORTED,
    DATASTORE_BACKUP_TABLE_MISSING,
    DATASTORE_RESTORED_EXPLANATION_COMPACTION_FAILED,
    DATASTORE_RESTORED_EVENT_INVALID,
    DATASTORE_RESTORED_EXPLANATION_INVALID,
    DATASTORE_BACKUP_ROW_IMPORT_FAILED,
    DATASTORE_BACKUP_ROW_UNREADABLE,
    DATASTORE_PASSWORD_FILE_UNREADABLE,
    DATASTORE_POSTGRES_PASSWORD_INVALID,
    DATASTORE_POSTGRES_USERNAME_INVALID,
    DATASTORE_BACKUP_ROW_INVALID,
    DATASTORE_URL_INVALID,
    DATASTORE_POSTGRES_PASSWORD_REQUIRED,
    DATASTORE_POSTGRES_USERNAME_REQUIRED,
    DATASTORE_POSTGRES_DATABASE_REQUIRED,
    DATASTORE_POSTGRES_SSLMODE_REQUIRED,
    DATASTORE_POSTGRES_HOST_REQUIRED,
    DATASTORE_DIRECTORY_RESTORE_ENGINE_UNSUPPORTED,
    DATASTORE_RESTORED_EXPLANATION_NULL,
    DATASTORE_BACKUP_FOREIGN_KEY_INVALID,
    DATASTORE_RESTORED_JSON_INVALID,
    DATASTORE_RESTORED_TABLE_ROW_COUNT_MISMATCH,
    DATASTORE_RESTORED_REQUIRED_VALUE_NULL,
    DATASTORE_PASSWORD_FILE_EMPTY,
    DATASTORE_URL_SCHEME_UNSUPPORTED,
    DATASTORE_POSTGRES_SSLMODE_UNSUPPORTED,
];

#[cfg(test)]
mod tests {
    #[test]
    fn reason_codes_are_distinct_and_stable_format() {
        let mut seen = std::collections::HashSet::new();
        for code in super::ALL {
            assert!(seen.insert(*code), "duplicate reason: {code}");
            assert!(["DATASTORE_"].iter().any(|prefix| code.starts_with(prefix)));
            assert!(
                code.bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
            );
        }
    }
}
