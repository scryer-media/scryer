use scryer_infrastructure::encryption::EncryptionKey;
use scryer_infrastructure::keystore;
use std::io::{self, BufRead, Write};
use std::path::Path;

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
    if Path::new("/.dockerenv").exists() {
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
        if Path::new(compose_path).exists() {
            eprintln!("error: {compose_path} already exists (refusing to overwrite)");
            std::process::exit(1);
        }
        if Path::new("scryer_encryption_key.txt").exists() {
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
        std::fs::write(compose_path, &compose).unwrap_or_else(|e| {
            eprintln!("error: failed to write {compose_path}: {e}");
            std::process::exit(1);
        });
        std::fs::write("scryer_encryption_key.txt", &encryption_key).unwrap_or_else(|e| {
            eprintln!("error: failed to write scryer_encryption_key.txt: {e}");
            std::process::exit(1);
        });
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                "scryer_encryption_key.txt",
                std::fs::Permissions::from_mode(0o600),
            )
            .unwrap_or_else(|e| {
                eprintln!(
                    "warning: could not set scryer_encryption_key.txt permissions to 600: {e}"
                );
            });
        }
        eprintln!("wrote {compose_path}");
        eprintln!("wrote scryer_encryption_key.txt (0600)");
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

      # ── Metadata gateway ──────────────────────────────────────────
      SCRYER_METADATA_GATEWAY_GRAPHQL_URL: https://smg.scryer.media/graphql

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
