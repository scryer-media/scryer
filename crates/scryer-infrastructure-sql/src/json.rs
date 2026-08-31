use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value as JsonValue;
use sqlx::Row;
use sqlx::types::Json;

use scryer_application::AppResult;

use super::runtime::{SqlArg, SqlRow, repo_err};

const COMPRESSED_JSON_MAX_BYTES: usize = 1024 * 1024;
const COMPRESSED_JSON_LEVEL: i32 = 3;

pub fn canonical_json_text<T: Serialize>(value: &T) -> AppResult<String> {
    serde_json::to_string(value).map_err(repo_err)
}

pub fn canonical_json_arg<T: Serialize>(value: &T) -> AppResult<SqlArg> {
    canonical_json_text(value).map(SqlArg::Text)
}

pub fn encode_compressed_json<T: Serialize>(value: &T) -> AppResult<Vec<u8>> {
    let json = serde_json::to_vec(value).map_err(repo_err)?;
    if json.len() > COMPRESSED_JSON_MAX_BYTES {
        return Err(scryer_application::AppError::Repository(format!(
            "JSON payload is {} bytes, exceeding the {}-byte limit",
            json.len(),
            COMPRESSED_JSON_MAX_BYTES
        )));
    }
    zstd::bulk::compress(&json, COMPRESSED_JSON_LEVEL).map_err(repo_err)
}

pub fn decode_compressed_json<T: DeserializeOwned>(encoded: &[u8]) -> AppResult<T> {
    let json = zstd::bulk::decompress(encoded, COMPRESSED_JSON_MAX_BYTES).map_err(repo_err)?;
    serde_json::from_slice(&json).map_err(repo_err)
}

pub fn opt_json_text(row: &SqlRow, column: &str) -> AppResult<Option<String>> {
    match row {
        SqlRow::Sqlite(row) => {
            let raw: Option<String> = row.try_get(column).map_err(repo_err)?;
            Ok(raw.filter(|value| !value.trim().is_empty()))
        }
        SqlRow::Postgres(row) => {
            if let Ok(raw) = row.try_get::<Option<String>, _>(column) {
                return Ok(raw.filter(|value| !value.trim().is_empty()));
            }
            let raw: Option<Json<JsonValue>> = row.try_get(column).map_err(repo_err)?;
            Ok(raw.map(|value| value.0.to_string()))
        }
    }
}

pub fn json_text_or(row: &SqlRow, column: &str, default: &str) -> AppResult<String> {
    Ok(opt_json_text(row, column)?.unwrap_or_else(|| default.to_string()))
}
