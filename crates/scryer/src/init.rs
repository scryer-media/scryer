use scryer_infrastructure_crypto::EncryptionKey;
use scryer_infrastructure_datastore::keystore;
use std::io::{self, BufRead, Write};
use std::path::Path;

use scryer_application::disk::{Access, Disk, DiskError, Open, reason};

const KEY_FILE: &str = "scryer_encryption_key.txt";

fn exists(path: &Path) -> bool {
    Disk::process().follow_blocking(path).exists() == Some(true)
}

/// Why [`create_new_file`] did not leave the whole file behind.
#[derive(Debug)]
enum CreateFailure {
    /// The file was not created.
    Create(DiskError),
    /// The file was created, but writing or closing it failed.
    Write(io::Error),
}

impl CreateFailure {
    /// Something already holds the name: a file, or a link that was not
    /// followed.
    fn name_taken(&self) -> bool {
        matches!(
            self,
            Self::Create(error)
                if matches!(
                    error.reason(),
                    reason::DISK_DESTINATION_EXISTS | reason::DISK_LINK_NOT_FOLLOWED
                )
        )
    }

    fn words(&self) -> String {
        match self {
            Self::Create(error) => error.words().to_string(),
            Self::Write(error) => error.to_string(),
        }
    }
}

/// Create `path` holding `contents`, with `access` applied as it is created.
///
/// Anything already at `path` is refused and kept, a link included, so the
/// contents never land in a file someone else placed there. Answers the
/// permission bits the file was created with, where the platform has them.
fn create_new_file(
    disk: &Disk,
    path: &Path,
    contents: &[u8],
    access: Access,
) -> Result<Option<u32>, CreateFailure> {
    let mut file = disk
        .open_blocking(
            path,
            Open::create_new()
                .with_access(access)
                .without_following_links(),
        )
        .map_err(CreateFailure::Create)?;
    file.write_all(contents).map_err(CreateFailure::Write)?;
    #[cfg(unix)]
    let mode = disk.opened_file_mode_blocking(&file);
    #[cfg(not(unix))]
    let mode = None;
    disk.finish_blocking(path, file)
        .map_err(|error| CreateFailure::Write(error.into()))?;
    Ok(mode)
}

/// What init says about the key file's permissions: the exact owner-only
/// mode when it has it, the mode it has when that is narrower still, and
/// nothing where the platform has no such modes.
fn key_file_permissions_note(mode: Option<u32>) -> String {
    match mode {
        Some(0o600) => " (0600)".to_string(),
        Some(mode) => format!(" ({mode:04o})"),
        None => String::new(),
    }
}

enum InitMode {
    Docker,
    #[cfg(target_os = "macos")]
    Native,
    #[cfg(target_os = "windows")]
    Native,
    #[cfg(target_os = "linux")]
    Linux,
}

fn detect_init_mode(args: &[String]) -> InitMode {
    if args.iter().any(|a| a == "--compose") {
        return InitMode::Docker;
    }
    if exists(Path::new("/.dockerenv")) {
        return InitMode::Docker;
    }
    #[cfg(target_os = "macos")]
    {
        InitMode::Native
    }
    #[cfg(target_os = "windows")]
    {
        InitMode::Native
    }
    #[cfg(target_os = "linux")]
    {
        InitMode::Linux
    }
}

pub fn run_init(args: Vec<String>) {
    match detect_init_mode(&args) {
        InitMode::Docker => run_init_docker(args),
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        InitMode::Native => run_init_native(),
        #[cfg(target_os = "linux")]
        InitMode::Linux => run_init_linux(),
    }
}

// ── Docker init ─────────────────────────────────────────────────────────────

fn run_init_docker(args: Vec<String>) {
    let output_path = parse_output_arg(&args);
    let write_to_stdout = output_path.as_deref() == Some("-");

    if !write_to_stdout {
        let compose_path = output_path.as_deref().unwrap_or("docker-compose.yml");
        if exists(Path::new(compose_path)) {
            eprintln!("error: {compose_path} already exists (refusing to overwrite)");
            std::process::exit(1);
        }
        if exists(Path::new(KEY_FILE)) {
            eprintln!("error: scryer_encryption_key.txt already exists (refusing to overwrite)");
            std::process::exit(1);
        }
    }

    let stdin = io::stdin();
    let mut reader = stdin.lock();

    let movies_path = prompt(&mut reader, "Movies directory on this host", "/data/movies");
    let series_path = prompt(&mut reader, "Series directory on this host", "/data/series");
    let encryption_key = EncryptionKey::generate().to_base64();
    let compose = generate_compose(&movies_path, &series_path);

    if write_to_stdout {
        print!("{encryption_key}\n---\n{compose}");
    } else {
        let compose_path = output_path.as_deref().unwrap_or("docker-compose.yml");
        let disk = Disk::process();
        let create = |path: &str, contents: &str, access: Access| {
            create_new_file(&disk, Path::new(path), contents.as_bytes(), access).unwrap_or_else(
                |failure| {
                    if failure.name_taken() {
                        eprintln!("error: {path} already exists (refusing to overwrite)");
                    } else {
                        eprintln!("error: failed to write {path}: {}", failure.words());
                    }
                    std::process::exit(1);
                },
            )
        };
        create(compose_path, &compose, Access::Default);
        let key_mode = create(KEY_FILE, &encryption_key, Access::OwnerOnly);
        eprintln!("wrote {compose_path}");
        eprintln!(
            "wrote scryer_encryption_key.txt{}",
            key_file_permissions_note(key_mode)
        );
        eprintln!();
        eprintln!("Your encryption key is stored in scryer_encryption_key.txt and mounted");
        eprintln!("as a Docker secret. Do not lose this file — you will need to reconfigure");
        eprintln!("passwords and API keys if it changes.");
        eprintln!();
        eprintln!("next steps:");
        eprintln!("  docker compose up -d");
    }
}

// ── Native init (macOS / Windows) ───────────────────────────────────────────

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn run_init_native() {
    let stores = keystore::platform_keystores(None);

    // Check if a key already exists
    for store in &stores {
        if let Ok(Some(_)) = store.get_key() {
            eprintln!("encryption key already exists in {}", store.name());
            eprintln!("no action needed — scryer will use it automatically on startup.");
            return;
        }
    }

    let key = EncryptionKey::generate();

    for store in &stores {
        match store.set_key(&key.to_base64()) {
            Ok(()) => {
                eprintln!("Encryption key generated and stored in {}.", store.name());
                #[cfg(target_os = "macos")]
                {
                    eprintln!("  service: media.scryer.app");
                    eprintln!("  account: encryption-master-key");
                    eprintln!();
                    eprintln!(
                        "The key is protected by your login keychain. No further setup needed."
                    );
                }
                #[cfg(target_os = "windows")]
                {
                    eprintln!("  target: scryer/encryption-master-key");
                    eprintln!();
                    eprintln!("The key is tied to your user account. No further setup needed.");
                }
                return;
            }
            Err(e) => {
                eprintln!("warning: could not store key in {}: {e}", store.name());
            }
        }
    }

    // Fallback: print key if no keystore accepted it
    eprintln!("Encryption key generated but could not be stored in any keystore.");
    eprintln!("Set this environment variable before starting scryer:");
    eprintln!();
    eprintln!("  SCRYER_ENCRYPTION_KEY={}", key.to_base64());
}

// ── Linux bare-metal init ───────────────────────────────────────────────────

#[cfg(target_os = "linux")]
fn run_init_linux() {
    let data_dir = directories::ProjectDirs::from("", "", "scryer")
        .map(|p| p.data_dir().to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    let stores = keystore::platform_keystores(Some(data_dir.clone()));
    let key_file_path = data_dir.join("encryption.key");

    // Check if a key already exists
    for store in &stores {
        if let Ok(Some(_)) = store.get_key() {
            eprintln!("encryption key already exists in {}", store.name());
            eprintln!("no action needed — scryer will use it automatically on startup.");
            return;
        }
    }

    let key = EncryptionKey::generate();

    // Try the KeyFile store (skip DockerSecret — it's read-only)
    for store in &stores {
        match store.set_key(&key.to_base64()) {
            Ok(()) => {
                eprintln!(
                    "Encryption key generated and saved to {}",
                    key_file_path.display()
                );
                eprintln!("  permissions: 0600 (owner read/write only)");
                eprintln!();
                eprintln!("Alternatively, set the SCRYER_ENCRYPTION_KEY environment variable.");
                return;
            }
            Err(_) => continue,
        }
    }

    // Fallback: print key
    eprintln!("Encryption key generated. Set this environment variable:");
    eprintln!();
    eprintln!("  SCRYER_ENCRYPTION_KEY={}", key.to_base64());
}

// ── Shared helpers ──────────────────────────────────────────────────────────

fn parse_output_arg(args: &[String]) -> Option<String> {
    let mut iter = args.iter().skip(1); // skip binary name
    while let Some(arg) = iter.next() {
        if arg == "--output" || arg == "-o" {
            return iter.next().cloned();
        }
        if let Some(val) = arg.strip_prefix("--output=") {
            return Some(val.to_string());
        }
        // skip "init" itself
    }
    None
}

fn prompt(reader: &mut impl BufRead, label: &str, default: &str) -> String {
    eprint!("{label} [{default}]: ");
    io::stderr().flush().ok();

    let mut input = String::new();
    if reader.read_line(&mut input).is_err() {
        return default.to_string();
    }

    let trimmed = input.trim();
    if trimmed.is_empty() {
        default.to_string()
    } else {
        trimmed.to_string()
    }
}

fn generate_compose(movies_path: &str, series_path: &str) -> String {
    format!(
        r#"services:
  scryer:
    image: ghcr.io/scryer-media/scryer:latest
    container_name: scryer
    restart: unless-stopped
    ports:
      - "8080:8080"
    volumes:
      - scryer-config:/config
      - {movies_path}:/data/movies
      - {series_path}:/data/series
    secrets:
      - scryer_encryption_key
    environment:
      # ── User/group identity ─────────────────────────────────────
      # Set to match your host user so file permissions are correct.
      PUID: "1000"
      PGID: "1000"

      # ── Authentication ────────────────────────────────────────────
      # Beta default: authentication is disabled and all requests act
      # as the built-in admin user. Set this to "true" to require login.
      SCRYER_AUTH_ENABLED: "false"
      # For authenticated first boot, remove the false setting above and set
      # SCRYER_ADMIN_PASSWORD to your temporary admin password, or mount a
      # Compose secret and set SCRYER_ADMIN_PASSWORD_FILE: /run/secrets/admin_password
      # Add that secret to both this service and the top-level secrets section.
      # Optionally set SCRYER_ADMIN_USERNAME: box-owner and
      # SCRYER_DISABLE_DEFAULT_ADMIN: "true" to disable the built-in admin.
      # Sign in as the selected user with the supplied temporary password,
      # then choose a new password. The database retains the replacement.
      # SCRYER_ADMIN_PASSWORD_RESET: "true" deliberately resets that password
      # on EVERY startup. Remove the flag after use. It does not reset MFA;
      # lost MFA requires the separate recovery-admin workflow.
    # ── Upgrade procedure ───────────────────────────────────────────
    # 1. docker compose pull
    # 2. docker compose up -d
    #
    # Migrations run automatically. The scryer-config volume preserves
    # your database and all settings across upgrades.

secrets:
  scryer_encryption_key:
    file: ./scryer_encryption_key.txt

volumes:
  scryer-config:
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(path: &Path) -> Vec<u8> {
        Disk::process()
            .read_blocking(path)
            .expect("fixture file is readable")
    }

    #[test]
    fn a_new_key_file_is_created_owner_only_with_its_contents() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("key.txt");

        let mode = create_new_file(&Disk::process(), &path, b"synthetic-key", Access::OwnerOnly)
            .expect("a free name is created");

        assert_eq!(read(&path), b"synthetic-key");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = mode.expect("unix reports a mode");
            assert_eq!(mode & 0o077, 0, "no access beyond the owner: {mode:o}");
            let on_disk = std::fs::metadata(&path)
                .expect("created file has metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, on_disk);
        }
        #[cfg(not(unix))]
        assert_eq!(mode, None);
    }

    #[test]
    fn an_existing_file_is_refused_and_kept() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("key.txt");
        let disk = Disk::process();
        create_new_file(&disk, &path, b"earlier-contents", Access::Default).expect("fixture");

        let failure = create_new_file(&disk, &path, b"synthetic-key", Access::OwnerOnly)
            .expect_err("an existing file is never replaced");

        assert!(failure.name_taken(), "{failure:?}");
        assert_eq!(read(&path), b"earlier-contents");
    }

    #[cfg(unix)]
    #[test]
    fn a_planted_link_is_refused_and_its_target_is_left_alone() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("temp dir");
        let target = dir.path().join("elsewhere.txt");
        let link = dir.path().join("key.txt");
        let disk = Disk::process();
        create_new_file(&disk, &target, b"someone-elses-file", Access::Default).expect("fixture");
        let target_mode = || {
            std::fs::metadata(&target)
                .expect("target has metadata")
                .permissions()
                .mode()
                & 0o777
        };
        let mode_before = target_mode();
        std::os::unix::fs::symlink(&target, &link).expect("plant link");

        let failure = create_new_file(&disk, &link, b"synthetic-key", Access::OwnerOnly)
            .expect_err("a link at the name is never written through");

        assert!(failure.name_taken(), "{failure:?}");
        assert_eq!(read(&target), b"someone-elses-file");
        assert_eq!(target_mode(), mode_before);
    }

    #[cfg(unix)]
    #[test]
    fn a_dangling_link_is_refused_and_creates_nothing() {
        let dir = tempfile::tempdir().expect("temp dir");
        let target = dir.path().join("not-there.txt");
        let link = dir.path().join("key.txt");
        std::os::unix::fs::symlink(&target, &link).expect("plant link");

        let failure = create_new_file(&Disk::process(), &link, b"synthetic-key", Access::OwnerOnly)
            .expect_err("a dangling link is never followed");

        assert!(failure.name_taken(), "{failure:?}");
        assert_eq!(
            Disk::process().follow_blocking(&target).exists(),
            Some(false)
        );
    }

    #[test]
    fn the_permissions_note_claims_only_what_is_true() {
        assert_eq!(key_file_permissions_note(Some(0o600)), " (0600)");
        assert_eq!(key_file_permissions_note(Some(0o400)), " (0400)");
        assert_eq!(key_file_permissions_note(None), "");
    }
}
