use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use argon2::{Algorithm, Argon2, Params, Version};
use aws_lc_rs::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use aws_lc_rs::rand::{SystemRandom, generate};
use serde::{Deserialize, Serialize};
use tempfile::TempDir;

use crate::{AppError, AppResult};

pub const BACKUP_FORMAT_VERSION: &str = "scryer-backup-bundle-v2";
pub const BACKUP_PLAINTEXT_EXTENSION: &str = ".tar.zst";
pub const BACKUP_ENCRYPTED_EXTENSION: &str = ".enc";
pub const LEGACY_BACKUP_PLAINTEXT_EXTENSION: &str = ".scryer-backup.tar.zst";
pub const LEGACY_BACKUP_ENCRYPTED_EXTENSION: &str = ".scryer-backup.enc";

const INSTANCE_SECRETS_FILENAME: &str = "instance-secrets.json";
const MANIFEST_FILENAME: &str = "manifest.json";
const TABLES_DIRNAME: &str = "tables";
pub const BLOB_MARKER_TYPE: &str = "__scryer_type";
pub const BLOB_MARKER_BASE64: &str = "base64";
pub const EXPORT_BATCH_SIZE: i64 = 1_000;
const ENCRYPTED_BUNDLE_MAGIC: [u8; 8] = [0x53, 0x42, 0x45, 0x5f, 0x96, 0x31, 0xc4, 0x2a];
const BACKUP_ENCRYPTION_VERSION_1: u8 = 1;
const BACKUP_ENCRYPTION_CHUNK_SIZE: usize = 1024 * 1024;
#[cfg(any(feature = "runtime-backups", test))]
const BACKUP_ENCRYPTION_TAG_LEN: usize = 16;
#[cfg(any(feature = "runtime-backups", test))]
const BACKUP_ENCRYPTION_MAX_CIPHERTEXT_CHUNK_LEN: usize =
    BACKUP_ENCRYPTION_CHUNK_SIZE + BACKUP_ENCRYPTION_TAG_LEN;
const BACKUP_ENCRYPTION_SALT_LEN: usize = 16;
const BACKUP_ENCRYPTION_NONCE_PREFIX_LEN: usize = 4;
const BACKUP_ENCRYPTION_METADATA_V1_LEN: usize =
    BACKUP_ENCRYPTION_SALT_LEN + BACKUP_ENCRYPTION_NONCE_PREFIX_LEN;
const BACKUP_ENCRYPTION_KEY_LEN: usize = 32;
const BACKUP_ENCRYPTION_ARGON2_M_COST_KIB: u32 = 65_536;
const BACKUP_ENCRYPTION_ARGON2_T_COST: u32 = 3;
const BACKUP_ENCRYPTION_ARGON2_P_COST: u32 = 1;

pub fn backup_table_part_filename(table: &str) -> String {
    format!("{table}.ndjson")
}

pub fn backup_table_part_relative_path(table: &str) -> String {
    format!("{TABLES_DIRNAME}/{}", backup_table_part_filename(table))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackupTableClassification {
    Export,
    ResetOnRestore,
    Rebuild,
    Ignore,
}

#[derive(Clone, Copy, Debug)]
pub struct BackupTableCatalogEntry {
    pub table: &'static str,
    pub classification: BackupTableClassification,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BackupEncryptionMetadataV1 {
    salt: [u8; BACKUP_ENCRYPTION_SALT_LEN],
    nonce_prefix: [u8; BACKUP_ENCRYPTION_NONCE_PREFIX_LEN],
}

impl BackupEncryptionMetadataV1 {
    fn generate() -> AppResult<Self> {
        let rng = SystemRandom::new();
        let salt = generate::<[u8; BACKUP_ENCRYPTION_SALT_LEN]>(&rng)
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to generate backup encryption salt: {error}"
                ))
            })?
            .expose();
        let nonce_prefix = generate::<[u8; BACKUP_ENCRYPTION_NONCE_PREFIX_LEN]>(&rng)
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to generate backup encryption nonce prefix: {error}"
                ))
            })?
            .expose();
        Ok(Self { salt, nonce_prefix })
    }

    fn to_bytes(self) -> [u8; BACKUP_ENCRYPTION_METADATA_V1_LEN] {
        let mut bytes = [0_u8; BACKUP_ENCRYPTION_METADATA_V1_LEN];
        bytes[..BACKUP_ENCRYPTION_SALT_LEN].copy_from_slice(&self.salt);
        bytes[BACKUP_ENCRYPTION_SALT_LEN..].copy_from_slice(&self.nonce_prefix);
        bytes
    }

    #[cfg(any(feature = "runtime-backups", test))]
    fn from_bytes(bytes: &[u8]) -> AppResult<Self> {
        if bytes.len() != BACKUP_ENCRYPTION_METADATA_V1_LEN {
            return Err(AppError::Validation(
                "backup encryption metadata is invalid".into(),
            ));
        }

        let mut salt = [0_u8; BACKUP_ENCRYPTION_SALT_LEN];
        salt.copy_from_slice(&bytes[..BACKUP_ENCRYPTION_SALT_LEN]);
        let mut nonce_prefix = [0_u8; BACKUP_ENCRYPTION_NONCE_PREFIX_LEN];
        nonce_prefix.copy_from_slice(&bytes[BACKUP_ENCRYPTION_SALT_LEN..]);
        Ok(Self { salt, nonce_prefix })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct EncryptedBundleHeaderV1 {
    metadata: BackupEncryptionMetadataV1,
}

struct AtomicOutputWriter {
    temp_file: tempfile::NamedTempFile,
    writer: BufWriter<File>,
    final_path: PathBuf,
}

impl AtomicOutputWriter {
    fn new(path: &Path) -> AppResult<Self> {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent).map_err(|error| {
            AppError::Repository(format!("failed to create {}: {error}", parent.display()))
        })?;
        let temp_file = tempfile::Builder::new()
            .prefix(".scryer-backup-")
            .tempfile_in(parent)
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to create staged backup output in {}: {error}",
                    parent.display()
                ))
            })?;
        ensure_owner_only_permissions(temp_file.path())?;
        let writer = BufWriter::new(temp_file.reopen().map_err(|error| {
            AppError::Repository(format!(
                "failed to open staged backup output {}: {error}",
                temp_file.path().display()
            ))
        })?);
        Ok(Self {
            temp_file,
            writer,
            final_path: path.to_path_buf(),
        })
    }

    fn finish(mut self) -> AppResult<()> {
        self.writer.flush().map_err(|error| {
            AppError::Repository(format!(
                "failed to flush staged backup output {}: {error}",
                self.temp_file.path().display()
            ))
        })?;
        let file = self.writer.into_inner().map_err(|error| {
            AppError::Repository(format!(
                "failed to finalize staged backup output {}: {error}",
                self.temp_file.path().display()
            ))
        })?;
        file.sync_all().map_err(|error| {
            AppError::Repository(format!(
                "failed to sync staged backup output {}: {error}",
                self.temp_file.path().display()
            ))
        })?;
        drop(file);

        self.temp_file.persist(&self.final_path).map_err(|error| {
            AppError::Repository(format!(
                "failed to persist staged backup output to {}: {}",
                self.final_path.display(),
                error.error
            ))
        })?;
        ensure_owner_only_permissions(&self.final_path)?;
        Ok(())
    }
}

impl Write for AtomicOutputWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.writer.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

struct AeadChunkWriter<W> {
    writer: W,
    key: LessSafeKey,
    version: u8,
    metadata_bytes: [u8; BACKUP_ENCRYPTION_METADATA_V1_LEN],
    nonce_prefix: [u8; BACKUP_ENCRYPTION_NONCE_PREFIX_LEN],
    chunk_index: u64,
    buffer: Vec<u8>,
}

impl<W: Write> AeadChunkWriter<W> {
    fn new(mut writer: W, passphrase: &str) -> AppResult<Self> {
        let header = EncryptedBundleHeaderV1 {
            metadata: BackupEncryptionMetadataV1::generate()?,
        };
        let version = BACKUP_ENCRYPTION_VERSION_1;
        let metadata_bytes = header.metadata.to_bytes();
        let key_bytes = derive_backup_encryption_key(passphrase, &header.metadata.salt)?;
        let key = make_backup_aead_key(&key_bytes)?;

        writer.write_all(&ENCRYPTED_BUNDLE_MAGIC).map_err(|error| {
            AppError::Repository(format!("failed to write encrypted backup header: {error}"))
        })?;
        writer.write_all(&[version]).map_err(|error| {
            AppError::Repository(format!("failed to write encrypted backup version: {error}"))
        })?;
        writer
            .write_all(&(metadata_bytes.len() as u32).to_be_bytes())
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to write encrypted backup metadata length: {error}"
                ))
            })?;
        writer.write_all(&metadata_bytes).map_err(|error| {
            AppError::Repository(format!(
                "failed to write encrypted backup metadata: {error}"
            ))
        })?;

        Ok(Self {
            writer,
            key,
            version,
            metadata_bytes,
            nonce_prefix: header.metadata.nonce_prefix,
            chunk_index: 0,
            buffer: Vec::with_capacity(BACKUP_ENCRYPTION_CHUNK_SIZE),
        })
    }

    fn write_chunk(&mut self, plaintext: &[u8]) -> AppResult<()> {
        if plaintext.is_empty() {
            return Ok(());
        }

        let mut in_out = plaintext.to_vec();
        let nonce = chunk_nonce(self.nonce_prefix, self.chunk_index);
        let aad = chunk_aad(self.version, &self.metadata_bytes, self.chunk_index);
        self.key
            .seal_in_place_append_tag(nonce, Aad::from(aad.as_slice()), &mut in_out)
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to encrypt backup chunk {}: {error}",
                    self.chunk_index
                ))
            })?;

        let chunk_len = u32::try_from(in_out.len()).map_err(|_| {
            AppError::Repository("encrypted backup chunk length exceeds u32".into())
        })?;
        self.writer
            .write_all(&chunk_len.to_be_bytes())
            .and_then(|_| self.writer.write_all(&in_out))
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to write encrypted backup chunk {}: {error}",
                    self.chunk_index
                ))
            })?;
        self.chunk_index = self
            .chunk_index
            .checked_add(1)
            .ok_or_else(|| AppError::Repository("backup chunk index overflowed".into()))?;
        Ok(())
    }

    fn flush_buffer(&mut self) -> AppResult<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }

        let buffer = std::mem::take(&mut self.buffer);
        self.write_chunk(&buffer)
    }

    fn finish(mut self) -> AppResult<W> {
        self.flush_buffer()?;
        self.writer.flush().map_err(|error| {
            AppError::Repository(format!("failed to finalize encrypted bundle: {error}"))
        })?;
        Ok(self.writer)
    }
}

impl<W: Write> Write for AeadChunkWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut consumed = 0_usize;
        while consumed < buf.len() {
            let remaining = BACKUP_ENCRYPTION_CHUNK_SIZE - self.buffer.len();
            let to_copy = remaining.min(buf.len() - consumed);
            self.buffer
                .extend_from_slice(&buf[consumed..consumed + to_copy]);
            consumed += to_copy;
            if self.buffer.len() == BACKUP_ENCRYPTION_CHUNK_SIZE {
                self.flush_buffer().map_err(app_error_to_io_error)?;
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flush_buffer().map_err(app_error_to_io_error)?;
        self.writer.flush()
    }
}

#[cfg(any(feature = "runtime-backups", test))]
struct AeadChunkReader<R> {
    reader: R,
    key: LessSafeKey,
    metadata_bytes: [u8; BACKUP_ENCRYPTION_METADATA_V1_LEN],
    nonce_prefix: [u8; BACKUP_ENCRYPTION_NONCE_PREFIX_LEN],
    chunk_index: u64,
    plaintext: Vec<u8>,
    plaintext_offset: usize,
    finished: bool,
}

#[cfg(any(feature = "runtime-backups", test))]
impl<R: Read> AeadChunkReader<R> {
    fn new(reader: R, header: EncryptedBundleHeaderV1, passphrase: &str) -> AppResult<Self> {
        let metadata_bytes = header.metadata.to_bytes();
        let key_bytes = derive_backup_encryption_key(passphrase, &header.metadata.salt)?;
        let key = make_backup_aead_key(&key_bytes)?;
        Ok(Self {
            reader,
            key,
            metadata_bytes,
            nonce_prefix: header.metadata.nonce_prefix,
            chunk_index: 0,
            plaintext: Vec::new(),
            plaintext_offset: 0,
            finished: false,
        })
    }

    fn fill_plaintext(&mut self) -> AppResult<()> {
        if self.finished {
            return Ok(());
        }

        let Some(ciphertext_len) = read_encrypted_chunk_len(&mut self.reader)? else {
            self.finished = true;
            self.plaintext.clear();
            self.plaintext_offset = 0;
            return Ok(());
        };

        let mut in_out = vec![0_u8; ciphertext_len];
        self.reader.read_exact(&mut in_out).map_err(|error| {
            AppError::Validation(format!(
                "encrypted backup payload is truncated or invalid: {error}"
            ))
        })?;

        let nonce = chunk_nonce(self.nonce_prefix, self.chunk_index);
        let aad = chunk_aad(
            BACKUP_ENCRYPTION_VERSION_1,
            &self.metadata_bytes,
            self.chunk_index,
        );
        let plaintext = self
            .key
            .open_in_place(nonce, Aad::from(aad.as_slice()), &mut in_out)
            .map_err(|_| {
                AppError::Validation(
                    "failed to decrypt backup bundle: wrong password or corrupted data".into(),
                )
            })?;
        let plaintext_len = plaintext.len();
        in_out.truncate(plaintext_len);
        self.plaintext = in_out;
        self.plaintext_offset = 0;
        self.chunk_index = self
            .chunk_index
            .checked_add(1)
            .ok_or_else(|| AppError::Repository("backup chunk index overflowed".into()))?;
        Ok(())
    }
}

#[cfg(any(feature = "runtime-backups", test))]
impl<R: Read> Read for AeadChunkReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        while self.plaintext_offset >= self.plaintext.len() && !self.finished {
            self.fill_plaintext().map_err(app_error_to_io_error)?;
        }

        if self.plaintext_offset >= self.plaintext.len() {
            return Ok(0);
        }

        let remaining = &self.plaintext[self.plaintext_offset..];
        let to_copy = remaining.len().min(buf.len());
        buf[..to_copy].copy_from_slice(&remaining[..to_copy]);
        self.plaintext_offset += to_copy;
        Ok(to_copy)
    }
}

pub const BACKUP_TABLE_CATALOG: &[BackupTableCatalogEntry] = &[
    BackupTableCatalogEntry {
        table: "_sqlx_migrations",
        classification: BackupTableClassification::Ignore,
    },
    BackupTableCatalogEntry {
        table: "application_migrations",
        classification: BackupTableClassification::Export,
    },
    // Legacy: no current migration creates this table, but installs that
    // upgraded through the pre-0122 schema may still carry it. The entry is
    // deliberately retained — an `Ignore` entry for an absent table costs
    // nothing, whereas removing it would make validate_backup_catalog reject
    // the schema of any install that still has the table, failing every backup
    // they take. Keep until a migration explicitly DROPs it everywhere.
    BackupTableCatalogEntry {
        table: "subtitle_providers",
        classification: BackupTableClassification::Ignore,
    },
    BackupTableCatalogEntry {
        table: "external_import_setup_secret_drafts",
        classification: BackupTableClassification::Ignore,
    },
    BackupTableCatalogEntry {
        table: "external_import_setup_instance_api_keys",
        classification: BackupTableClassification::Ignore,
    },
    BackupTableCatalogEntry {
        table: "external_import_setup_download_client_api_key_overrides",
        classification: BackupTableClassification::Ignore,
    },
    BackupTableCatalogEntry {
        table: "external_import_setup_download_client_password_overrides",
        classification: BackupTableClassification::Ignore,
    },
    BackupTableCatalogEntry {
        table: "external_import_setup_indexer_api_key_overrides",
        classification: BackupTableClassification::Ignore,
    },
    BackupTableCatalogEntry {
        table: "upstream_scheduler_states",
        classification: BackupTableClassification::Ignore,
    },
    BackupTableCatalogEntry {
        table: "upstream_destination_cooldowns",
        classification: BackupTableClassification::Ignore,
    },
    BackupTableCatalogEntry {
        table: "upstream_scheduler_rss_cadence",
        classification: BackupTableClassification::Ignore,
    },
    // Convergence coverage records durable observations of external
    // indexer searches. Internal state transitions must not erase that evidence.
    BackupTableCatalogEntry {
        table: "scope_indexer_coverage",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "settings_definitions",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "title_search_terms",
        classification: BackupTableClassification::Rebuild,
    },
    BackupTableCatalogEntry {
        table: "title_search_spellfix",
        classification: BackupTableClassification::Rebuild,
    },
    BackupTableCatalogEntry {
        table: "blocklist",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "collection_external_ids",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "collections",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "title_metadata_tags",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "title_metadata_tag_sources",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "title_metadata_tag_source_keys",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "title_metadata_rating_summaries",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "title_metadata_rating_sources",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "title_metadata_external_ratings",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "title_credits",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "discovery_title_metadata_tags",
        classification: BackupTableClassification::ResetOnRestore,
    },
    BackupTableCatalogEntry {
        table: "discovery_title_metadata_tag_sources",
        classification: BackupTableClassification::ResetOnRestore,
    },
    BackupTableCatalogEntry {
        table: "discovery_title_metadata_tag_source_keys",
        classification: BackupTableClassification::ResetOnRestore,
    },
    BackupTableCatalogEntry {
        table: "discovery_title_metadata_rating_summaries",
        classification: BackupTableClassification::ResetOnRestore,
    },
    BackupTableCatalogEntry {
        table: "discovery_title_metadata_rating_sources",
        classification: BackupTableClassification::ResetOnRestore,
    },
    BackupTableCatalogEntry {
        table: "discovery_title_metadata_external_ratings",
        classification: BackupTableClassification::ResetOnRestore,
    },
    BackupTableCatalogEntry {
        table: "discovery_facets",
        classification: BackupTableClassification::ResetOnRestore,
    },
    BackupTableCatalogEntry {
        table: "discovery_item_library_provenance",
        classification: BackupTableClassification::ResetOnRestore,
    },
    BackupTableCatalogEntry {
        table: "discovery_item_rank_components",
        classification: BackupTableClassification::ResetOnRestore,
    },
    BackupTableCatalogEntry {
        table: "discovery_item_subject_links",
        classification: BackupTableClassification::ResetOnRestore,
    },
    BackupTableCatalogEntry {
        table: "discovery_items",
        classification: BackupTableClassification::ResetOnRestore,
    },
    BackupTableCatalogEntry {
        table: "discovery_pending_context_changes",
        classification: BackupTableClassification::ResetOnRestore,
    },
    BackupTableCatalogEntry {
        table: "discovery_section_items",
        classification: BackupTableClassification::ResetOnRestore,
    },
    BackupTableCatalogEntry {
        table: "discovery_sections",
        classification: BackupTableClassification::ResetOnRestore,
    },
    BackupTableCatalogEntry {
        table: "discovery_submitted_subjects",
        classification: BackupTableClassification::ResetOnRestore,
    },
    BackupTableCatalogEntry {
        table: "discovery_sync_runs",
        classification: BackupTableClassification::ResetOnRestore,
    },
    BackupTableCatalogEntry {
        table: "discovery_sync_state",
        classification: BackupTableClassification::ResetOnRestore,
    },
    BackupTableCatalogEntry {
        table: "discovery_title_external_ids",
        classification: BackupTableClassification::ResetOnRestore,
    },
    BackupTableCatalogEntry {
        table: "discovery_title_source_tag_values",
        classification: BackupTableClassification::ResetOnRestore,
    },
    BackupTableCatalogEntry {
        table: "discovery_title_source_tags",
        classification: BackupTableClassification::ResetOnRestore,
    },
    BackupTableCatalogEntry {
        table: "discovery_title_terms",
        classification: BackupTableClassification::ResetOnRestore,
    },
    BackupTableCatalogEntry {
        table: "discovery_titles",
        classification: BackupTableClassification::ResetOnRestore,
    },
    BackupTableCatalogEntry {
        table: "domain_events",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "download_clients",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "download_client_bindings",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "downloads",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "download_import_artifacts",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "download_identity_states",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "download_queue_commands",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "download_submission_episode_links",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "download_submissions",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "episode_external_ids",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "episodes",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "event_subscriber_offsets",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "external_import_monitor_snapshot_chunks",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "external_subtitle_probe_cache",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "file_episode_map",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "history_events",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "image_proxy_cache_entries",
        classification: BackupTableClassification::ResetOnRestore,
    },
    BackupTableCatalogEntry {
        table: "image_proxy_sources",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "imports",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "indexer_api_quotas",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "indexer_errors",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "indexer_search_candidates",
        classification: BackupTableClassification::ResetOnRestore,
    },
    BackupTableCatalogEntry {
        table: "indexer_search_candidate_source_values",
        classification: BackupTableClassification::ResetOnRestore,
    },
    BackupTableCatalogEntry {
        table: "indexer_search_candidate_sources",
        classification: BackupTableClassification::ResetOnRestore,
    },
    BackupTableCatalogEntry {
        table: "indexer_search_run_candidate_sources",
        classification: BackupTableClassification::ResetOnRestore,
    },
    BackupTableCatalogEntry {
        table: "indexer_search_learning",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "indexer_search_runs",
        classification: BackupTableClassification::ResetOnRestore,
    },
    BackupTableCatalogEntry {
        table: "indexer_system_backoffs",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "indexer_proxy_configs",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "indexers",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "libraries",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "library_probe_signatures",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "library_roots",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "library_scan_unmatched_items",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "login_verification_challenges",
        classification: BackupTableClassification::Ignore,
    },
    // A manual-import selection is deliberate user intent — the files a user
    // picked and the targets they mapped them to — held until the import
    // executes, so it is backed up like every other download/import lifecycle
    // table (download_submissions, download_identity_states, imports). Rows
    // reference a download-client item that may not exist after a restore, but
    // that is already self-healing: the tracked-download prune calls
    // delete_manual_import_selections_for_source once the download is observed
    // to be gone.
    BackupTableCatalogEntry {
        table: "manual_import_selection_candidates",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "manual_import_selections",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "media_files",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "movie_entities",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "series_movie_links",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "file_series_movie_link_map",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "media_server_connections",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "media_server_playback_items",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "media_server_default_library_grants",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "media_server_path_mappings",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "jellyfin_media_server_details",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "plex_media_server_details",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "emby_media_server_details",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "media_request_external_ids",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "media_request_requesters",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "media_requests",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "notification_channels",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "notification_subscriptions",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "api_keys",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "oauth_client_redirect_uris",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "oauth_client_registrations",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "oauth_authorization_codes",
        classification: BackupTableClassification::Ignore,
    },
    BackupTableCatalogEntry {
        table: "oauth_refresh_grants",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "oauth_refresh_tokens",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "pending_releases",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "plugin_catalog_sources",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "plugin_catalog_status",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "plugin_installations",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "post_processing_script_runs",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "post_processing_scripts",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "quality_profile_audio_codec_allowlist",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "quality_profile_audio_codec_blocklist",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "quality_profile_quality_tiers",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "quality_profile_source_allowlist",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "quality_profile_source_blocklist",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "quality_profile_video_codec_allowlist",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "quality_profile_video_codec_blocklist",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "quality_profiles",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "release_decisions",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "release_download_attempts",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "rule_set_history",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "rule_sets",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "seeding_profiles",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "settings_values",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "subtitle_blocklist",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "subtitle_downloads",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "subtitle_provider_configs",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "title_external_ids",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "title_image_blobs",
        classification: BackupTableClassification::ResetOnRestore,
    },
    BackupTableCatalogEntry {
        table: "title_image_variants",
        classification: BackupTableClassification::ResetOnRestore,
    },
    BackupTableCatalogEntry {
        table: "title_images",
        classification: BackupTableClassification::ResetOnRestore,
    },
    BackupTableCatalogEntry {
        table: "title_more_like_this_items",
        classification: BackupTableClassification::ResetOnRestore,
    },
    BackupTableCatalogEntry {
        table: "title_recommendation_cards",
        classification: BackupTableClassification::ResetOnRestore,
    },
    BackupTableCatalogEntry {
        table: "titles",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "totp_credentials",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "totp_enrollment_challenges",
        classification: BackupTableClassification::Ignore,
    },
    BackupTableCatalogEntry {
        table: "totp_failed_attempts",
        classification: BackupTableClassification::Ignore,
    },
    BackupTableCatalogEntry {
        table: "totp_recovery_codes",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "user_app_permission_masks",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "user_external_accounts",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "user_library_permission_masks",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "user_ui_settings",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "user_ui_table_columns",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "users",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "wanted_items",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "webauthn_challenges",
        classification: BackupTableClassification::Ignore,
    },
    BackupTableCatalogEntry {
        table: "webauthn_credentials",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "workflow_operations",
        classification: BackupTableClassification::Export,
    },
];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BackupBundleInspectSummary {
    pub format_version: String,
    pub created_at: String,
    pub source_scryer_version: String,
    pub source_engine: String,
    pub source_migration_key: Option<String>,
    pub encrypted: bool,
    pub row_counts: BTreeMap<String, u64>,
}

impl BackupBundleInspectSummary {
    pub fn total_rows(&self) -> u64 {
        self.row_counts.values().copied().sum()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BackupBundleManifest {
    pub format_version: String,
    pub created_at: String,
    pub source_scryer_version: String,
    pub source_engine: String,
    pub source_migration_key: Option<String>,
    pub encrypted: bool,
    pub row_counts: BTreeMap<String, u64>,
    pub part_checksums: BTreeMap<String, String>,
}

impl BackupBundleManifest {
    pub fn summary(&self) -> BackupBundleInspectSummary {
        BackupBundleInspectSummary {
            format_version: self.format_version.clone(),
            created_at: self.created_at.clone(),
            source_scryer_version: self.source_scryer_version.clone(),
            source_engine: self.source_engine.clone(),
            source_migration_key: self.source_migration_key.clone(),
            encrypted: self.encrypted,
            row_counts: self.row_counts.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BackupInstanceSecrets {
    encryption_master_key: String,
    jwt_signing_secret: String,
    smg_registration_secret: Option<String>,
    smg_gateway_url: Option<String>,
}

impl BackupInstanceSecrets {
    pub fn from_export_secrets(secrets: BackupExportSecrets) -> Self {
        Self {
            encryption_master_key: secrets.encryption_master_key,
            jwt_signing_secret: secrets.jwt_signing_secret,
            smg_registration_secret: secrets.smg_registration_secret,
            smg_gateway_url: secrets.smg_gateway_url,
        }
    }

    pub fn to_env_file(&self) -> String {
        let mut output = String::new();
        push_env_assignment(
            &mut output,
            "SCRYER_ENCRYPTION_KEY",
            &self.encryption_master_key,
        );
        push_env_assignment(
            &mut output,
            "SCRYER_JWT_SIGNING_SECRET",
            &self.jwt_signing_secret,
        );
        if let Some(value) = self.smg_registration_secret.as_deref() {
            push_env_assignment(&mut output, "SCRYER_SMG_REGISTRATION_SECRET", value);
        }
        if let Some(value) = self.smg_gateway_url.as_deref() {
            push_env_assignment(&mut output, "SCRYER_METADATA_GATEWAY_GRAPHQL_URL", value);
        }
        output
    }
}

#[derive(Clone, Debug)]
pub struct BackupExportSecrets {
    pub encryption_master_key: String,
    pub jwt_signing_secret: String,
    pub smg_registration_secret: Option<String>,
    pub smg_gateway_url: Option<String>,
}

#[derive(Clone, Debug)]
pub struct BackupBundleExportRequest {
    pub output_path: PathBuf,
    pub passphrase: String,
    pub source_migration_key: Option<String>,
    pub source_scryer_version: String,
    pub source_engine: String,
    pub secrets: BackupExportSecrets,
}

#[derive(Clone, Debug)]
pub struct BackupExportOutcome {
    pub summary: BackupBundleInspectSummary,
}

pub struct BackupBundleStaging {
    staging: TempDir,
    row_counts: BTreeMap<String, u64>,
    part_checksums: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
pub struct BackupRestorePreparedBundle {
    summary: BackupBundleInspectSummary,
    instance_secrets_env: String,
}

#[derive(Clone, Debug)]
pub struct PreparedBackupBundleDirectory {
    root: PathBuf,
    manifest: BackupBundleManifest,
}

impl BackupRestorePreparedBundle {
    pub fn from_summary_and_instance_secrets_env(
        summary: BackupBundleInspectSummary,
        instance_secrets_env: String,
    ) -> Self {
        Self {
            summary,
            instance_secrets_env,
        }
    }

    pub fn summary(&self) -> &BackupBundleInspectSummary {
        &self.summary
    }

    pub fn instance_secrets_env(&self) -> String {
        self.instance_secrets_env.clone()
    }
}

impl PreparedBackupBundleDirectory {
    pub fn load(root: &Path) -> AppResult<Self> {
        let manifest = load_manifest(root)?;
        validate_extracted_bundle(root, &manifest)?;
        Ok(Self {
            root: root.to_path_buf(),
            manifest,
        })
    }

    pub fn tables_dir(&self) -> PathBuf {
        self.root.join(TABLES_DIRNAME)
    }

    pub fn manifest(&self) -> &BackupBundleManifest {
        &self.manifest
    }

    pub fn summary(&self) -> BackupBundleInspectSummary {
        self.manifest.summary()
    }

    pub fn instance_secrets_env(&self) -> AppResult<String> {
        Ok(load_instance_secrets(&self.root)?.to_env_file())
    }
}

impl BackupBundleStaging {
    pub fn new() -> AppResult<Self> {
        let staging = tempfile::tempdir().map_err(|error| {
            AppError::Repository(format!("failed to create backup staging dir: {error}"))
        })?;
        let tables_dir = staging.path().join(TABLES_DIRNAME);
        std::fs::create_dir_all(&tables_dir).map_err(|error| {
            AppError::Repository(format!("failed to create tables staging dir: {error}"))
        })?;

        Ok(Self {
            staging,
            row_counts: BTreeMap::new(),
            part_checksums: BTreeMap::new(),
        })
    }

    pub fn tables_dir(&self) -> PathBuf {
        self.staging.path().join(TABLES_DIRNAME)
    }

    pub fn record_table_part(
        &mut self,
        table: &str,
        row_count: u64,
        checksum: String,
    ) -> AppResult<()> {
        self.row_counts.insert(table.to_string(), row_count);
        let rel_path = backup_table_part_relative_path(table);
        self.part_checksums.insert(rel_path, checksum);
        Ok(())
    }

    pub fn finish(mut self, request: BackupBundleExportRequest) -> AppResult<BackupExportOutcome> {
        if request.passphrase.trim().is_empty() {
            return Err(AppError::Validation(
                "backup export requires a non-empty password".into(),
            ));
        }

        let instance_secrets = BackupInstanceSecrets::from_export_secrets(request.secrets);
        let instance_secrets_path = self.staging.path().join(INSTANCE_SECRETS_FILENAME);
        write_json_file(&instance_secrets_path, &instance_secrets)?;
        self.part_checksums.insert(
            INSTANCE_SECRETS_FILENAME.to_string(),
            checksum_hex(&instance_secrets_path)?,
        );

        let manifest = BackupBundleManifest {
            format_version: BACKUP_FORMAT_VERSION.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            source_scryer_version: request.source_scryer_version,
            source_engine: request.source_engine,
            source_migration_key: request.source_migration_key,
            encrypted: true,
            row_counts: self.row_counts,
            part_checksums: self.part_checksums,
        };
        let manifest_path = self.staging.path().join(MANIFEST_FILENAME);
        write_json_file(&manifest_path, &manifest)?;

        let writer = AtomicOutputWriter::new(&request.output_path)?;
        let writer = AeadChunkWriter::new(writer, &request.passphrase)?;
        let writer = write_bundle_payload(self.staging.path(), writer)?.finish()?;
        writer.finish()?;

        Ok(BackupExportOutcome {
            summary: manifest.summary(),
        })
    }
}

pub fn inspect_backup_bundle(
    bundle_path: &Path,
    passphrase: Option<&str>,
) -> AppResult<BackupBundleInspectSummary> {
    let extracted = extract_bundle_to_tempdir(bundle_path, passphrase)?;
    let manifest = load_manifest(extracted.path())?;
    validate_extracted_bundle(extracted.path(), &manifest)?;
    Ok(manifest.summary())
}

pub struct BackupBundleRestorePayload {
    extracted: TempDir,
    manifest: BackupBundleManifest,
}

impl BackupBundleRestorePayload {
    pub fn tables_dir(&self) -> PathBuf {
        self.extracted.path().join(TABLES_DIRNAME)
    }

    pub fn manifest(&self) -> &BackupBundleManifest {
        &self.manifest
    }

    pub fn summary(&self) -> BackupBundleInspectSummary {
        self.manifest.summary()
    }

    pub fn instance_secrets_env(&self) -> AppResult<String> {
        Ok(load_instance_secrets(self.extracted.path())?.to_env_file())
    }

    pub fn persist_extracted_dir(&self, target_root: &Path) -> AppResult<()> {
        if target_root.exists() {
            std::fs::remove_dir_all(target_root).map_err(|error| {
                AppError::Repository(format!(
                    "failed to clear prepared backup directory {}: {error}",
                    target_root.display()
                ))
            })?;
        }
        copy_directory_contents(self.extracted.path(), target_root)
    }
}

pub fn prepare_backup_restore_payload(
    bundle_path: &Path,
    passphrase: Option<&str>,
) -> AppResult<BackupBundleRestorePayload> {
    let extracted = extract_bundle_to_tempdir(bundle_path, passphrase)?;
    let manifest = load_manifest(extracted.path())?;
    validate_extracted_bundle(extracted.path(), &manifest)?;

    Ok(BackupBundleRestorePayload {
        extracted,
        manifest,
    })
}

fn push_env_assignment(output: &mut String, key: &str, value: &str) {
    output.push_str(key);
    output.push('=');
    output.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            _ => output.push(ch),
        }
    }
    output.push_str("\"\n");
}

fn write_json_file(path: &Path, value: &impl Serialize) -> AppResult<()> {
    let file = File::create(path).map_err(|error| {
        AppError::Repository(format!("failed to create {}: {error}", path.display()))
    })?;
    let writer = BufWriter::new(file);
    serde_json::to_writer_pretty(writer, value).map_err(|error| {
        AppError::Repository(format!("failed to serialize {}: {error}", path.display()))
    })
}

fn checksum_hex(path: impl AsRef<Path>) -> AppResult<String> {
    let mut file = File::open(path.as_ref()).map_err(|error| {
        AppError::Repository(format!(
            "failed to open {} for checksum: {error}",
            path.as_ref().display()
        ))
    })?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            AppError::Repository(format!(
                "failed to read {} for checksum: {error}",
                path.as_ref().display()
            ))
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

#[cfg(feature = "runtime-backups")]
fn write_bundle_payload<W: Write>(stage_dir: &Path, writer: W) -> AppResult<W> {
    let encoder = zstd::Encoder::new(writer, 3)
        .map_err(|error| AppError::Repository(format!("failed to start zstd encoder: {error}")))?;
    let mut tar = tar::Builder::new(encoder);
    tar.append_path_with_name(stage_dir.join(MANIFEST_FILENAME), MANIFEST_FILENAME)
        .map_err(|error| {
            AppError::Repository(format!("failed to append manifest to tar: {error}"))
        })?;
    tar.append_path_with_name(
        stage_dir.join(INSTANCE_SECRETS_FILENAME),
        INSTANCE_SECRETS_FILENAME,
    )
    .map_err(|error| AppError::Repository(format!("failed to append secrets to tar: {error}")))?;
    tar.append_dir_all(TABLES_DIRNAME, stage_dir.join(TABLES_DIRNAME))
        .map_err(|error| {
            AppError::Repository(format!("failed to append tables to tar: {error}"))
        })?;
    let encoder = tar.into_inner().map_err(|error| {
        AppError::Repository(format!("failed to finalize tar payload: {error}"))
    })?;
    encoder
        .finish()
        .map_err(|error| AppError::Repository(format!("failed to finalize zstd payload: {error}")))
}

#[cfg(not(feature = "runtime-backups"))]
fn write_bundle_payload<W: Write>(_stage_dir: &Path, _writer: W) -> AppResult<W> {
    Err(AppError::Repository(
        "backup bundle payload support is not compiled into this target".into(),
    ))
}

#[cfg(feature = "runtime-backups")]
fn extract_bundle_to_tempdir(bundle_path: &Path, passphrase: Option<&str>) -> AppResult<TempDir> {
    let tempdir = tempfile::tempdir().map_err(|error| {
        AppError::Repository(format!(
            "failed to create restore staging directory: {error}"
        ))
    })?;

    let payload_reader = open_bundle_payload_reader(bundle_path, passphrase)?;
    let decoder = zstd::Decoder::new(payload_reader)
        .map_err(|error| map_streaming_bundle_error(error, "backup payload is not valid zstd"))?;
    let mut archive = tar::Archive::new(decoder);
    archive.unpack(tempdir.path()).map_err(|error| {
        map_streaming_bundle_error(error, "backup payload is not a valid tar archive")
    })?;

    Ok(tempdir)
}

#[cfg(not(feature = "runtime-backups"))]
fn extract_bundle_to_tempdir(_bundle_path: &Path, _passphrase: Option<&str>) -> AppResult<TempDir> {
    Err(AppError::Repository(
        "backup bundle payload support is not compiled into this target".into(),
    ))
}

#[cfg(feature = "runtime-backups")]
fn open_bundle_payload_reader(
    bundle_path: &Path,
    passphrase: Option<&str>,
) -> AppResult<Box<dyn Read>> {
    let input = File::open(bundle_path).map_err(|error| {
        AppError::Repository(format!(
            "failed to open bundle {}: {error}",
            bundle_path.display()
        ))
    })?;
    let mut reader = BufReader::new(input);
    let mut prefix = [0_u8; ENCRYPTED_BUNDLE_MAGIC.len()];
    let read = reader.read(&mut prefix).map_err(|error| {
        AppError::Repository(format!("failed to read backup bundle header: {error}"))
    })?;
    if read == ENCRYPTED_BUNDLE_MAGIC.len() && prefix == ENCRYPTED_BUNDLE_MAGIC {
        let passphrase = passphrase.ok_or_else(|| {
            AppError::Validation("this backup bundle is encrypted and requires a password".into())
        })?;
        let header = parse_encrypted_bundle_header_after_magic(&mut reader)?;
        let reader = AeadChunkReader::new(reader, header, passphrase)?;
        Ok(Box::new(reader))
    } else {
        Ok(Box::new(
            std::io::Cursor::new(prefix[..read].to_vec()).chain(reader),
        ))
    }
}

#[cfg(test)]
fn parse_encrypted_bundle_header_from_reader(
    reader: &mut impl Read,
) -> AppResult<Option<EncryptedBundleHeaderV1>> {
    let mut magic = [0_u8; ENCRYPTED_BUNDLE_MAGIC.len()];
    let read = reader.read(&mut magic).map_err(|error| {
        AppError::Repository(format!("failed to read encrypted backup header: {error}"))
    })?;
    if read != ENCRYPTED_BUNDLE_MAGIC.len() || magic != ENCRYPTED_BUNDLE_MAGIC {
        return Ok(None);
    }

    parse_encrypted_bundle_header_after_magic(reader).map(Some)
}

#[cfg(any(feature = "runtime-backups", test))]
fn parse_encrypted_bundle_header_after_magic(
    reader: &mut impl Read,
) -> AppResult<EncryptedBundleHeaderV1> {
    let mut version = [0_u8; 1];
    reader.read_exact(&mut version).map_err(|error| {
        AppError::Validation(format!("encrypted backup header is truncated: {error}"))
    })?;
    if version[0] != BACKUP_ENCRYPTION_VERSION_1 {
        return Err(AppError::Validation(format!(
            "unsupported encrypted backup version {}",
            version[0]
        )));
    }

    let mut metadata_len = [0_u8; 4];
    reader.read_exact(&mut metadata_len).map_err(|error| {
        AppError::Validation(format!(
            "encrypted backup metadata header is truncated: {error}"
        ))
    })?;
    let metadata_len = u32::from_be_bytes(metadata_len) as usize;
    if metadata_len != BACKUP_ENCRYPTION_METADATA_V1_LEN {
        return Err(AppError::Validation(
            "backup encryption metadata is invalid".into(),
        ));
    }

    let mut metadata_bytes = [0_u8; BACKUP_ENCRYPTION_METADATA_V1_LEN];
    reader.read_exact(&mut metadata_bytes).map_err(|error| {
        AppError::Validation(format!("encrypted backup metadata is truncated: {error}"))
    })?;

    Ok(EncryptedBundleHeaderV1 {
        metadata: BackupEncryptionMetadataV1::from_bytes(&metadata_bytes)?,
    })
}

fn derive_backup_encryption_key(
    passphrase: &str,
    salt: &[u8; BACKUP_ENCRYPTION_SALT_LEN],
) -> AppResult<[u8; BACKUP_ENCRYPTION_KEY_LEN]> {
    let params = Params::new(
        BACKUP_ENCRYPTION_ARGON2_M_COST_KIB,
        BACKUP_ENCRYPTION_ARGON2_T_COST,
        BACKUP_ENCRYPTION_ARGON2_P_COST,
        Some(BACKUP_ENCRYPTION_KEY_LEN),
    )
    .map_err(|error| {
        AppError::Repository(format!(
            "failed to configure backup encryption KDF parameters: {error}"
        ))
    })?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0_u8; BACKUP_ENCRYPTION_KEY_LEN];
    argon2
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|error| {
            AppError::Validation(format!(
                "failed to derive backup encryption key from password: {error}"
            ))
        })?;
    Ok(key)
}

fn make_backup_aead_key(key_bytes: &[u8; BACKUP_ENCRYPTION_KEY_LEN]) -> AppResult<LessSafeKey> {
    let unbound = UnboundKey::new(&AES_256_GCM, key_bytes).map_err(|error| {
        AppError::Repository(format!(
            "failed to construct backup encryption key: {error}"
        ))
    })?;
    Ok(LessSafeKey::new(unbound))
}

fn chunk_nonce(nonce_prefix: [u8; BACKUP_ENCRYPTION_NONCE_PREFIX_LEN], chunk_index: u64) -> Nonce {
    let mut nonce = [0_u8; 12];
    nonce[..BACKUP_ENCRYPTION_NONCE_PREFIX_LEN].copy_from_slice(&nonce_prefix);
    nonce[BACKUP_ENCRYPTION_NONCE_PREFIX_LEN..].copy_from_slice(&chunk_index.to_be_bytes());
    Nonce::assume_unique_for_key(nonce)
}

fn chunk_aad(version: u8, metadata_bytes: &[u8], chunk_index: u64) -> Vec<u8> {
    let mut aad = Vec::with_capacity(ENCRYPTED_BUNDLE_MAGIC.len() + 1 + metadata_bytes.len() + 8);
    aad.extend_from_slice(&ENCRYPTED_BUNDLE_MAGIC);
    aad.push(version);
    aad.extend_from_slice(metadata_bytes);
    aad.extend_from_slice(&chunk_index.to_be_bytes());
    aad
}

#[cfg(any(feature = "runtime-backups", test))]
fn read_encrypted_chunk_len(reader: &mut impl Read) -> AppResult<Option<usize>> {
    let mut len = [0_u8; 4];
    let read = reader.read(&mut len[..1]).map_err(|error| {
        AppError::Validation(format!(
            "encrypted backup payload length is invalid: {error}"
        ))
    })?;
    if read == 0 {
        return Ok(None);
    }

    reader.read_exact(&mut len[1..]).map_err(|error| {
        AppError::Validation(format!(
            "encrypted backup payload length is truncated: {error}"
        ))
    })?;
    let chunk_len = u32::from_be_bytes(len) as usize;
    if !(BACKUP_ENCRYPTION_TAG_LEN..=BACKUP_ENCRYPTION_MAX_CIPHERTEXT_CHUNK_LEN)
        .contains(&chunk_len)
    {
        return Err(AppError::Validation(
            "encrypted backup payload length is invalid".into(),
        ));
    }
    Ok(Some(chunk_len))
}

fn app_error_to_io_error(error: AppError) -> io::Error {
    io::Error::other(error.to_string())
}

#[cfg(feature = "runtime-backups")]
fn map_streaming_bundle_error<E>(error: E, fallback: &str) -> AppError
where
    E: std::error::Error + 'static,
{
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(&error);
    while let Some(source) = current {
        let message = source.to_string();
        if let Some(validation) = message.strip_prefix("validation: ") {
            return AppError::Validation(validation.to_string());
        }
        if let Some(repository) = message.strip_prefix("repository: ") {
            return AppError::Repository(repository.to_string());
        }
        current = source.source();
    }
    AppError::Validation(format!("{fallback}: {error}"))
}

fn load_manifest(root: &Path) -> AppResult<BackupBundleManifest> {
    let path = root.join(MANIFEST_FILENAME);
    let file = File::open(&path)
        .map_err(|error| AppError::Validation(format!("backup manifest missing: {error}")))?;
    serde_json::from_reader(BufReader::new(file))
        .map_err(|error| AppError::Validation(format!("backup manifest is invalid: {error}")))
}

fn load_instance_secrets(root: &Path) -> AppResult<BackupInstanceSecrets> {
    let path = root.join(INSTANCE_SECRETS_FILENAME);
    let file = File::open(&path).map_err(|error| {
        AppError::Validation(format!("backup secrets payload missing: {error}"))
    })?;
    serde_json::from_reader(BufReader::new(file)).map_err(|error| {
        AppError::Validation(format!("backup secrets payload is invalid: {error}"))
    })
}

fn expected_bundle_parts(manifest: &BackupBundleManifest) -> BTreeSet<String> {
    manifest
        .row_counts
        .keys()
        .map(|table| backup_table_part_relative_path(table))
        .chain(std::iter::once(INSTANCE_SECRETS_FILENAME.to_string()))
        .collect()
}

fn copy_directory_contents(source: &Path, target: &Path) -> AppResult<()> {
    std::fs::create_dir_all(target).map_err(|error| {
        AppError::Repository(format!(
            "failed to create prepared backup directory {}: {error}",
            target.display()
        ))
    })?;

    for entry in std::fs::read_dir(source).map_err(|error| {
        AppError::Repository(format!(
            "failed to read prepared backup directory {}: {error}",
            source.display()
        ))
    })? {
        let entry = entry.map_err(|error| {
            AppError::Repository(format!(
                "failed to inspect prepared backup directory {}: {error}",
                source.display()
            ))
        })?;
        let file_type = entry.file_type().map_err(|error| {
            AppError::Repository(format!(
                "failed to inspect prepared backup entry {}: {error}",
                entry.path().display()
            ))
        })?;
        let destination = target.join(entry.file_name());
        if file_type.is_dir() {
            copy_directory_contents(&entry.path(), &destination)?;
        } else if file_type.is_file() {
            std::fs::copy(entry.path(), &destination).map_err(|error| {
                AppError::Repository(format!(
                    "failed to stage prepared backup file {}: {error}",
                    destination.display()
                ))
            })?;
        } else {
            return Err(AppError::Validation(format!(
                "prepared backup directory contains unsupported entry {}",
                entry.path().display()
            )));
        }
    }

    Ok(())
}

fn validate_extracted_bundle(root: &Path, manifest: &BackupBundleManifest) -> AppResult<()> {
    if manifest.format_version != BACKUP_FORMAT_VERSION {
        return Err(AppError::Validation(format!(
            "unsupported backup format version {}",
            manifest.format_version
        )));
    }

    let canonical_root = root.canonicalize().map_err(|error| {
        AppError::Repository(format!(
            "failed to resolve extracted backup root {}: {error}",
            root.display()
        ))
    })?;

    for (part, expected_checksum) in &manifest.part_checksums {
        let part_path = Path::new(part);
        if part_path.is_absolute() {
            return Err(AppError::Validation(format!(
                "backup part path must be relative: {part}"
            )));
        }
        if part_path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(AppError::Validation(format!(
                "backup part path escapes the bundle root: {part}"
            )));
        }

        let canonical_part = root.join(part_path).canonicalize().map_err(|error| {
            AppError::Validation(format!("backup part path is invalid: {part}: {error}"))
        })?;
        if !canonical_part.starts_with(&canonical_root) {
            return Err(AppError::Validation(format!(
                "backup part path escapes the bundle root: {part}"
            )));
        }
        let metadata = std::fs::metadata(&canonical_part).map_err(|error| {
            AppError::Validation(format!("backup part path is unreadable: {part}: {error}"))
        })?;
        if !metadata.is_file() {
            return Err(AppError::Validation(format!(
                "backup part path must be a regular file: {part}"
            )));
        }

        let actual_checksum = checksum_hex(&canonical_part)?;
        if &actual_checksum != expected_checksum {
            return Err(AppError::Validation(format!(
                "backup checksum mismatch for {part}"
            )));
        }
    }

    let expected_parts = expected_bundle_parts(manifest);
    let actual_parts = manifest
        .part_checksums
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    if actual_parts != expected_parts {
        let missing = expected_parts
            .difference(&actual_parts)
            .cloned()
            .collect::<Vec<_>>();
        let unexpected = actual_parts
            .difference(&expected_parts)
            .cloned()
            .collect::<Vec<_>>();
        return Err(AppError::Validation(format!(
            "backup checksum manifest is incomplete: missing [{}], unexpected [{}]",
            missing.join(", "),
            unexpected.join(", ")
        )));
    }

    Ok(())
}

fn ensure_owner_only_permissions(path: &Path) -> AppResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let permissions = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(path, permissions).map_err(|error| {
            AppError::Repository(format!(
                "failed to set permissions on {}: {error}",
                path.display()
            ))
        })?;
    }
    #[cfg(not(unix))]
    let _ = path;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::io::{Cursor, Seek, SeekFrom, Write};

    fn manifest_with_part(part: &str, checksum: &str) -> BackupBundleManifest {
        manifest_with_parts(
            BTreeMap::new(),
            BTreeMap::from([(part.to_string(), checksum.to_string())]),
        )
    }

    fn manifest_with_parts(
        row_counts: BTreeMap<String, u64>,
        part_checksums: BTreeMap<String, String>,
    ) -> BackupBundleManifest {
        BackupBundleManifest {
            format_version: BACKUP_FORMAT_VERSION.to_string(),
            created_at: "2026-05-15T00:00:00Z".to_string(),
            source_scryer_version: "0.15.0".to_string(),
            source_engine: "sqlite".to_string(),
            source_migration_key: None,
            encrypted: false,
            row_counts,
            part_checksums,
        }
    }

    #[test]
    fn backup_table_catalog_exports_download_identity_states() {
        let classification = BACKUP_TABLE_CATALOG
            .iter()
            .find(|entry| entry.table == "download_identity_states")
            .map(|entry| entry.classification);

        assert_eq!(classification, Some(BackupTableClassification::Export));
    }

    #[test]
    fn backup_table_catalog_preserves_image_proxy_sources_but_resets_cached_bytes() {
        for (table, expected) in [
            ("image_proxy_sources", BackupTableClassification::Export),
            (
                "image_proxy_cache_entries",
                BackupTableClassification::ResetOnRestore,
            ),
        ] {
            let classification = BACKUP_TABLE_CATALOG
                .iter()
                .find(|entry| entry.table == table)
                .map(|entry| entry.classification);

            assert_eq!(classification, Some(expected), "{table}");
        }
    }

    #[test]
    fn backup_table_catalog_preserves_oauth_grants_but_not_codes() {
        for (table, expected) in [
            ("api_keys", BackupTableClassification::Export),
            (
                "oauth_authorization_codes",
                BackupTableClassification::Ignore,
            ),
            ("oauth_refresh_grants", BackupTableClassification::Export),
            ("oauth_refresh_tokens", BackupTableClassification::Export),
        ] {
            let classification = BACKUP_TABLE_CATALOG
                .iter()
                .find(|entry| entry.table == table)
                .map(|entry| entry.classification);

            assert_eq!(
                classification,
                Some(expected),
                "{table} should have the intended backup classification"
            );
        }
    }

    #[test]
    fn backup_table_catalog_exports_manual_import_selection_tables() {
        // Migration 0151 shipped these tables without catalog entries, which
        // failed EVERY backup at the catalog-completeness check — the feature
        // was entirely broken, not merely missing this data.
        for table in [
            "manual_import_selections",
            "manual_import_selection_candidates",
        ] {
            let classification = BACKUP_TABLE_CATALOG
                .iter()
                .find(|entry| entry.table == table)
                .map(|entry| entry.classification);

            assert_eq!(
                classification,
                Some(BackupTableClassification::Export),
                "{table} should be exported in logical backups"
            );
        }
    }

    #[test]
    fn backup_table_catalog_has_no_duplicate_entries() {
        // A duplicate would let two classifications disagree for one table,
        // with the winner decided by iteration order.
        let mut seen = std::collections::BTreeSet::new();
        for entry in BACKUP_TABLE_CATALOG {
            assert!(
                seen.insert(entry.table),
                "{} appears twice in BACKUP_TABLE_CATALOG",
                entry.table
            );
        }
    }

    #[test]
    fn backup_table_catalog_preserves_application_migration_successes() {
        let entry = BACKUP_TABLE_CATALOG
            .iter()
            .find(|entry| entry.table == "application_migrations")
            .expect("application migration ledger should be classified");
        assert_eq!(entry.classification, BackupTableClassification::Export);
    }

    #[test]
    fn backup_table_catalog_exports_series_movie_tables() {
        for table in [
            "movie_entities",
            "series_movie_links",
            "file_series_movie_link_map",
        ] {
            let classification = BACKUP_TABLE_CATALOG
                .iter()
                .find(|entry| entry.table == table)
                .map(|entry| entry.classification);

            assert_eq!(
                classification,
                Some(BackupTableClassification::Export),
                "{table} should be exported in logical backups"
            );
        }
    }

    #[test]
    fn backup_table_catalog_exports_owner_scoped_metadata_tables() {
        for table in [
            "title_metadata_tags",
            "title_metadata_tag_sources",
            "title_metadata_tag_source_keys",
            "title_metadata_rating_summaries",
            "title_metadata_rating_sources",
            "title_metadata_external_ratings",
            "title_credits",
        ] {
            let classification = BACKUP_TABLE_CATALOG
                .iter()
                .find(|entry| entry.table == table)
                .map(|entry| entry.classification);

            assert_eq!(
                classification,
                Some(BackupTableClassification::Export),
                "{table} should be exported in logical backups"
            );
        }

        assert!(
            BACKUP_TABLE_CATALOG
                .iter()
                .all(|entry| !entry.table.starts_with("canonical_media_"))
        );
    }

    #[test]
    fn env_file_writer_escapes_multiline_values() {
        let secrets = BackupInstanceSecrets {
            encryption_master_key: "enc".into(),
            jwt_signing_secret: "jwt".into(),
            smg_registration_secret: Some("reg".into()),
            smg_gateway_url: Some("https://smg.example/graphql".into()),
        };

        let env_file = secrets.to_env_file();
        assert!(env_file.contains("SCRYER_ENCRYPTION_KEY=\"enc\""));
    }

    #[test]
    fn validate_extracted_bundle_accepts_relative_regular_files() {
        let temp = tempfile::tempdir().expect("tempdir");
        let tables_dir = temp.path().join(TABLES_DIRNAME);
        std::fs::create_dir_all(&tables_dir).expect("tables dir");
        let table_path = tables_dir.join("titles.ndjson");
        let secrets_path = temp.path().join(INSTANCE_SECRETS_FILENAME);
        std::fs::write(&table_path, b"[]").expect("table payload");
        std::fs::write(&secrets_path, b"{}").expect("secrets");
        let table_checksum = checksum_hex(&table_path).expect("checksum");
        let secrets_checksum = checksum_hex(&secrets_path).expect("checksum");

        validate_extracted_bundle(
            temp.path(),
            &manifest_with_parts(
                BTreeMap::from([("titles".to_string(), 0)]),
                BTreeMap::from([
                    (backup_table_part_relative_path("titles"), table_checksum),
                    (INSTANCE_SECRETS_FILENAME.to_string(), secrets_checksum),
                ]),
            ),
        )
        .expect("bundle should validate");
    }

    #[test]
    fn validate_extracted_bundle_rejects_absolute_paths() {
        let temp = tempfile::tempdir().expect("tempdir");
        let payload = temp.path().join("payload.txt");
        std::fs::write(&payload, b"ok").expect("payload");
        let checksum = checksum_hex(&payload).expect("checksum");

        let error = validate_extracted_bundle(
            temp.path(),
            &manifest_with_part(payload.to_string_lossy().as_ref(), &checksum),
        )
        .expect_err("absolute path should fail");

        assert!(
            matches!(error, AppError::Validation(message) if message.contains("must be relative"))
        );
    }

    #[test]
    fn validate_extracted_bundle_rejects_traversal_paths() {
        let temp = tempfile::tempdir().expect("tempdir");
        let payload = temp.path().join("payload.txt");
        std::fs::write(&payload, b"ok").expect("payload");
        let checksum = checksum_hex(&payload).expect("checksum");

        let error = validate_extracted_bundle(
            temp.path(),
            &manifest_with_part("../payload.txt", &checksum),
        )
        .expect_err("traversal path should fail");

        assert!(
            matches!(error, AppError::Validation(message) if message.contains("escapes the bundle root"))
        );
    }

    #[test]
    fn validate_extracted_bundle_rejects_non_regular_files() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(temp.path().join("nested")).expect("nested dir");

        let error =
            validate_extracted_bundle(temp.path(), &manifest_with_part("nested", "ignored"))
                .expect_err("directories should fail");

        assert!(matches!(error, AppError::Validation(message) if message.contains("regular file")));
    }

    #[test]
    fn validate_extracted_bundle_requires_secrets_checksum() {
        let temp = tempfile::tempdir().expect("tempdir");
        let table_path = temp.path().join(TABLES_DIRNAME).join("titles.ndjson");
        std::fs::create_dir_all(table_path.parent().expect("tables dir")).expect("tables dir");
        std::fs::write(&table_path, b"[]").expect("table payload");
        std::fs::write(temp.path().join(INSTANCE_SECRETS_FILENAME), b"{}").expect("secrets");

        let checksum = checksum_hex(&table_path).expect("checksum");
        let manifest = manifest_with_parts(
            BTreeMap::from([("titles".to_string(), 0)]),
            BTreeMap::from([(backup_table_part_relative_path("titles"), checksum)]),
        );

        let error = validate_extracted_bundle(temp.path(), &manifest)
            .expect_err("missing secrets checksum should fail");

        assert!(
            matches!(error, AppError::Validation(message) if message.contains(INSTANCE_SECRETS_FILENAME))
        );
    }

    #[test]
    fn validate_extracted_bundle_requires_table_checksums_for_all_manifest_tables() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join(INSTANCE_SECRETS_FILENAME), b"{}").expect("secrets");
        let secrets_checksum =
            checksum_hex(temp.path().join(INSTANCE_SECRETS_FILENAME)).expect("checksum");

        let manifest = manifest_with_parts(
            BTreeMap::from([("titles".to_string(), 0)]),
            BTreeMap::from([(INSTANCE_SECRETS_FILENAME.to_string(), secrets_checksum)]),
        );

        let error = validate_extracted_bundle(temp.path(), &manifest)
            .expect_err("missing table checksum should fail");

        assert!(
            matches!(error, AppError::Validation(message) if message.contains("tables/titles.ndjson"))
        );
    }

    #[cfg(feature = "runtime-backups")]
    #[test]
    fn prepared_backup_directory_round_trip_preserves_restore_inputs() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bundle_path = temp.path().join("roundtrip.scryer-backup.enc");
        write_test_bundle(&bundle_path, "backup-passphrase").expect("write bundle");

        let prepared = prepare_backup_restore_payload(&bundle_path, Some("backup-passphrase"))
            .expect("prepare restore payload");
        let prepared_dir = temp.path().join("prepared");
        prepared
            .persist_extracted_dir(&prepared_dir)
            .expect("persist extracted dir");

        let staged =
            PreparedBackupBundleDirectory::load(&prepared_dir).expect("load prepared backup dir");
        assert_eq!(staged.manifest().row_counts.get("titles"), Some(&1));
        assert_eq!(staged.summary().source_scryer_version, "test");
        assert!(
            staged
                .instance_secrets_env()
                .expect("instance secrets env")
                .contains("SCRYER_ENCRYPTION_KEY=\"master-key\"")
        );
    }

    #[cfg(feature = "runtime-backups")]
    #[test]
    fn encrypted_backup_round_trip_uses_versioned_header() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bundle_path = temp.path().join("roundtrip.scryer-backup.enc");
        let passphrase = "backup-passphrase";
        write_test_bundle(&bundle_path, passphrase).expect("write bundle");

        let summary = inspect_backup_bundle(&bundle_path, Some(passphrase)).expect("inspect");
        assert!(summary.encrypted);

        let bytes = std::fs::read(&bundle_path).expect("read bundle");
        assert_eq!(
            &bytes[..ENCRYPTED_BUNDLE_MAGIC.len()],
            &ENCRYPTED_BUNDLE_MAGIC
        );
        assert_eq!(
            bytes[ENCRYPTED_BUNDLE_MAGIC.len()],
            BACKUP_ENCRYPTION_VERSION_1
        );
    }

    #[cfg(feature = "runtime-backups")]
    #[test]
    fn encrypted_backup_rejects_wrong_passphrase() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bundle_path = temp.path().join("wrong-pass.scryer-backup.enc");
        write_test_bundle(&bundle_path, "correct").expect("write bundle");

        let error = inspect_backup_bundle(&bundle_path, Some("wrong"))
            .expect_err("wrong password should fail");
        assert!(
            error
                .to_string()
                .contains("failed to decrypt backup bundle")
        );
    }

    #[cfg(feature = "runtime-backups")]
    #[test]
    fn encrypted_backup_rejects_unknown_version() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bundle_path = temp.path().join("unknown-version.scryer-backup.enc");
        write_test_bundle(&bundle_path, "correct").expect("write bundle");

        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&bundle_path)
            .expect("open bundle");
        file.seek(SeekFrom::Start(ENCRYPTED_BUNDLE_MAGIC.len() as u64))
            .expect("seek");
        file.write_all(&[BACKUP_ENCRYPTION_VERSION_1 + 1])
            .expect("write version");

        let error = inspect_backup_bundle(&bundle_path, Some("correct"))
            .expect_err("unknown version should fail");
        assert!(
            error
                .to_string()
                .contains("unsupported encrypted backup version")
        );
    }

    #[cfg(feature = "runtime-backups")]
    #[test]
    fn encrypted_backup_rejects_truncated_metadata() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bundle_path = temp.path().join("truncated-metadata.scryer-backup.enc");
        write_test_bundle(&bundle_path, "correct").expect("write bundle");

        let mut bytes = std::fs::read(&bundle_path).expect("read bundle");
        bytes.truncate(ENCRYPTED_BUNDLE_MAGIC.len() + 1 + 4 + 3);
        std::fs::write(&bundle_path, bytes).expect("rewrite bundle");

        let error = inspect_backup_bundle(&bundle_path, Some("correct"))
            .expect_err("truncated metadata should fail");
        assert!(error.to_string().contains("metadata is truncated"));
    }

    #[cfg(feature = "runtime-backups")]
    #[test]
    fn encrypted_backup_rejects_invalid_metadata_length() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bundle_path = temp
            .path()
            .join("invalid-metadata-length.scryer-backup.enc");
        write_test_bundle(&bundle_path, "correct").expect("write bundle");

        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&bundle_path)
            .expect("open bundle");
        file.seek(SeekFrom::Start((ENCRYPTED_BUNDLE_MAGIC.len() + 1) as u64))
            .expect("seek");
        file.write_all(&(0_u32).to_be_bytes())
            .expect("write metadata length");

        let error = inspect_backup_bundle(&bundle_path, Some("correct"))
            .expect_err("invalid metadata length should fail");
        assert!(error.to_string().contains("metadata is invalid"));
    }

    #[cfg(feature = "runtime-backups")]
    #[test]
    fn encrypted_backup_rejects_truncated_chunk() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bundle_path = temp.path().join("truncated-chunk.scryer-backup.enc");
        write_test_bundle(&bundle_path, "correct").expect("write bundle");

        let mut bytes = std::fs::read(&bundle_path).expect("read bundle");
        bytes.pop();
        std::fs::write(&bundle_path, bytes).expect("rewrite bundle");

        let error = inspect_backup_bundle(&bundle_path, Some("correct"))
            .expect_err("truncated chunk should fail");
        assert!(
            error
                .to_string()
                .contains("payload is truncated or invalid")
        );
    }

    #[cfg(feature = "runtime-backups")]
    #[test]
    fn plaintext_backup_extracts_plain_ndjson_parts() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bundle_path = temp.path().join("plain.scryer-backup.tar.zst");
        write_legacy_plaintext_test_bundle(&bundle_path).expect("write bundle");

        let summary = inspect_backup_bundle(&bundle_path, None).expect("inspect");
        assert!(!summary.encrypted);

        let extracted = extract_bundle_to_tempdir(&bundle_path, None).expect("extract");
        assert!(
            extracted
                .path()
                .join(backup_table_part_relative_path("titles"))
                .exists()
        );
        assert!(!extracted.path().join("tables/titles.ndjson.zst").exists());
        assert!(!extracted.path().join("payload.tar.zst").exists());
    }

    #[test]
    fn atomic_output_writer_preserves_existing_file_until_finish() {
        let temp = tempfile::tempdir().expect("tempdir");
        let output_path = temp.path().join("existing.scryer-backup.tar.zst");
        std::fs::write(&output_path, b"known-good").expect("seed output");

        let mut writer = AtomicOutputWriter::new(&output_path).expect("create atomic writer");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mode = std::fs::metadata(writer.temp_file.path())
                .expect("temp metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
        writer.write_all(b"partial").expect("write partial");
        assert_eq!(
            std::fs::read(&output_path).expect("read existing output"),
            b"known-good"
        );
        drop(writer);

        assert_eq!(
            std::fs::read(&output_path).expect("read preserved output"),
            b"known-good"
        );
    }

    #[test]
    fn atomic_output_writer_replaces_existing_file_on_finish() {
        let temp = tempfile::tempdir().expect("tempdir");
        let output_path = temp.path().join("replace.scryer-backup.tar.zst");
        std::fs::write(&output_path, b"old-backup").expect("seed output");

        let mut writer = AtomicOutputWriter::new(&output_path).expect("create atomic writer");
        writer
            .write_all(b"new-backup")
            .expect("write staged output");
        writer.finish().expect("finish output");

        assert_eq!(
            std::fs::read(&output_path).expect("read replaced output"),
            b"new-backup"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mode = std::fs::metadata(&output_path)
                .expect("output metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn backup_table_catalog_resets_ephemeral_table_families() {
        for entry in BACKUP_TABLE_CATALOG.iter().filter(|entry| {
            entry.table.starts_with("discovery_")
                || matches!(
                    entry.table,
                    "title_more_like_this_items"
                        | "title_images"
                        | "title_image_variants"
                        | "title_image_blobs"
                        | "indexer_search_candidates"
                        | "indexer_search_runs"
                )
        }) {
            assert_eq!(
                entry.classification,
                BackupTableClassification::ResetOnRestore,
                "{} should be reset instead of exported",
                entry.table
            );
        }
    }

    #[test]
    fn backup_table_catalog_exports_convergence_coverage() {
        let classification = BACKUP_TABLE_CATALOG
            .iter()
            .find(|entry| entry.table == "scope_indexer_coverage")
            .map(|entry| entry.classification);

        assert_eq!(classification, Some(BackupTableClassification::Export));
    }

    #[test]
    fn encrypted_chunk_writer_round_trip_handles_exact_chunk_boundary() {
        let passphrase = "exact-boundary-pass";
        let payload = deterministic_payload_bytes(BACKUP_ENCRYPTION_CHUNK_SIZE);
        let round_trip = round_trip_aead_payload(&payload, passphrase).expect("round trip");
        assert_eq!(round_trip, payload);
    }

    #[test]
    fn encrypted_chunk_writer_round_trip_spans_multiple_chunks() {
        let passphrase = "multi-chunk-pass";
        let payload = deterministic_payload_bytes(BACKUP_ENCRYPTION_CHUNK_SIZE + 1);
        let round_trip = round_trip_aead_payload(&payload, passphrase).expect("round trip");
        assert_eq!(round_trip, payload);
    }

    #[cfg(feature = "runtime-backups")]
    #[test]
    fn inspect_rejects_legacy_bundle_format_version() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bundle_path = temp.path().join("legacy-format.scryer-backup.tar.zst");
        write_legacy_plaintext_test_bundle(&bundle_path).expect("write bundle");

        let extracted = extract_bundle_to_tempdir(&bundle_path, None).expect("extract");
        let mut manifest = load_manifest(extracted.path()).expect("manifest");
        manifest.format_version = "scryer-backup-bundle-v1".to_string();
        write_json_file(&extracted.path().join(MANIFEST_FILENAME), &manifest).expect("rewrite");

        let rebuilt_path = temp.path().join("legacy-rebuilt.scryer-backup.tar.zst");
        let writer = write_bundle_payload(
            extracted.path(),
            AtomicOutputWriter::new(&rebuilt_path).expect("create output"),
        )
        .expect("rebuild bundle");
        writer.finish().expect("persist rebuilt bundle");

        let error = inspect_backup_bundle(&rebuilt_path, None).expect_err("legacy format");
        assert!(
            error
                .to_string()
                .contains("unsupported backup format version")
        );
    }

    #[cfg(feature = "runtime-backups")]
    #[test]
    fn encrypted_backup_rejects_oversized_chunk_length() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bundle_path = temp.path().join("oversized-chunk.scryer-backup.enc");
        write_test_bundle(&bundle_path, "correct").expect("write bundle");

        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&bundle_path)
            .expect("open bundle");
        let chunk_len_offset =
            (ENCRYPTED_BUNDLE_MAGIC.len() + 1 + 4 + BACKUP_ENCRYPTION_METADATA_V1_LEN) as u64;
        file.seek(SeekFrom::Start(chunk_len_offset)).expect("seek");
        file.write_all(
            &(u32::try_from(BACKUP_ENCRYPTION_MAX_CIPHERTEXT_CHUNK_LEN).unwrap() + 1).to_be_bytes(),
        )
        .expect("write chunk length");

        let error = inspect_backup_bundle(&bundle_path, Some("correct"))
            .expect_err("oversized chunk should fail");
        assert!(error.to_string().contains("payload length is invalid"));
    }

    #[cfg(feature = "runtime-backups")]
    fn write_test_bundle(output_path: &Path, passphrase: &str) -> AppResult<()> {
        write_test_bundle_with_payload_size(output_path, passphrase, 64)
    }

    #[cfg(feature = "runtime-backups")]
    fn write_test_bundle_with_payload_size(
        output_path: &Path,
        passphrase: &str,
        payload_size: usize,
    ) -> AppResult<()> {
        let staging = test_bundle_staging(payload_size)?;
        staging.finish(test_export_request(output_path, passphrase))?;
        Ok(())
    }

    #[cfg(feature = "runtime-backups")]
    fn write_legacy_plaintext_test_bundle(output_path: &Path) -> AppResult<()> {
        let mut staging = test_bundle_staging(64)?;
        let instance_secrets = BackupInstanceSecrets::from_export_secrets(test_export_secrets());
        let instance_secrets_path = staging.staging.path().join(INSTANCE_SECRETS_FILENAME);
        write_json_file(&instance_secrets_path, &instance_secrets)?;
        staging.part_checksums.insert(
            INSTANCE_SECRETS_FILENAME.to_string(),
            checksum_hex(&instance_secrets_path)?,
        );

        let manifest = BackupBundleManifest {
            format_version: BACKUP_FORMAT_VERSION.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            source_migration_key: Some("0112".to_string()),
            source_scryer_version: "test".to_string(),
            source_engine: "sqlite".to_string(),
            encrypted: false,
            row_counts: staging.row_counts,
            part_checksums: staging.part_checksums,
        };
        write_json_file(&staging.staging.path().join(MANIFEST_FILENAME), &manifest)?;

        let writer = write_bundle_payload(
            staging.staging.path(),
            AtomicOutputWriter::new(output_path)?,
        )?;
        writer.finish()?;
        Ok(())
    }

    #[cfg(feature = "runtime-backups")]
    fn test_bundle_staging(payload_size: usize) -> AppResult<BackupBundleStaging> {
        let mut staging = BackupBundleStaging::new()?;
        let table_path = staging
            .tables_dir()
            .join(backup_table_part_filename("titles"));
        write_test_payload(&table_path, payload_size)?;
        let checksum = checksum_hex(&table_path)?;
        staging.record_table_part("titles", 1, checksum)?;

        Ok(staging)
    }

    #[cfg(feature = "runtime-backups")]
    fn test_export_request(output_path: &Path, passphrase: &str) -> BackupBundleExportRequest {
        BackupBundleExportRequest {
            output_path: output_path.to_path_buf(),
            passphrase: passphrase.to_string(),
            source_migration_key: Some("0112".to_string()),
            source_scryer_version: "test".to_string(),
            source_engine: "sqlite".to_string(),
            secrets: test_export_secrets(),
        }
    }

    #[cfg(feature = "runtime-backups")]
    fn test_export_secrets() -> BackupExportSecrets {
        BackupExportSecrets {
            encryption_master_key: "master-key".to_string(),
            jwt_signing_secret: "jwt-secret".to_string(),
            smg_registration_secret: Some("smg-secret".to_string()),
            smg_gateway_url: None,
        }
    }

    #[cfg(feature = "runtime-backups")]
    fn write_test_payload(path: &Path, payload_size: usize) -> AppResult<()> {
        let file = File::create(path).map_err(|error| {
            AppError::Repository(format!("failed to create test payload: {error}"))
        })?;
        let mut writer = BufWriter::new(file);
        let payload = deterministic_payload_bytes(payload_size);
        writer.write_all(&payload).map_err(|error| {
            AppError::Repository(format!("failed to write test payload: {error}"))
        })?;
        writer.flush().map_err(|error| {
            AppError::Repository(format!("failed to flush test payload: {error}"))
        })?;
        Ok(())
    }

    fn deterministic_payload_bytes(payload_size: usize) -> Vec<u8> {
        let mut payload = Vec::with_capacity(payload_size);
        let mut state = 0x0123_4567_89ab_cdef_u64;
        for _ in 0..payload_size {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            payload.push((state & 0xff) as u8);
        }
        payload
    }

    fn round_trip_aead_payload(payload: &[u8], passphrase: &str) -> AppResult<Vec<u8>> {
        let writer = AeadChunkWriter::new(Cursor::new(Vec::new()), passphrase)?;
        let mut writer = writer;
        for chunk in payload.chunks(7919) {
            writer.write_all(chunk).map_err(|error| {
                AppError::Repository(format!("failed to write chunked test payload: {error}"))
            })?;
        }
        let mut encrypted = writer.finish()?;
        encrypted.seek(SeekFrom::Start(0)).map_err(|error| {
            AppError::Repository(format!("failed to seek test payload: {error}"))
        })?;

        let mut encrypted_reader = BufReader::new(encrypted);
        let header =
            parse_encrypted_bundle_header_from_reader(&mut encrypted_reader)?.ok_or_else(|| {
                AppError::Validation("missing encrypted bundle header in test payload".into())
            })?;
        let mut reader = AeadChunkReader::new(encrypted_reader, header, passphrase)?;
        let mut plaintext = Vec::new();
        let mut buffer = [0_u8; 4093];
        loop {
            let read = reader.read(&mut buffer).map_err(|error| {
                AppError::Repository(format!("failed to read chunked test payload: {error}"))
            })?;
            if read == 0 {
                break;
            }
            plaintext.extend_from_slice(&buffer[..read]);
        }
        Ok(plaintext)
    }
}
