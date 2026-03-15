use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

fn main() {
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR is not set");
    let output_path = Path::new(&out_dir).join("embedded_ui_assets.rs");
    let mut output = String::new();

    if let Some(raw_dir) = env::var_os("SCRYER_EMBED_UI_DIR") {
        let configured_dir = PathBuf::from(raw_dir);
        let embed_dir = configured_dir.canonicalize().unwrap_or_else(|error| {
            panic!(
                "invalid SCRYER_EMBED_UI_DIR '{}': {error}",
                configured_dir.display()
            )
        });

        if !embed_dir.is_dir() {
            panic!(
                "SCRYER_EMBED_UI_DIR must point to a directory: {}",
                embed_dir.display()
            );
        }

        let index_html = embed_dir.join("index.html");
        if !index_html.is_file() {
            panic!(
                "SCRYER_EMBED_UI_DIR must contain an index.html file: {}",
                embed_dir.display()
            );
        }

        let mut entries = collect_files(&embed_dir).unwrap_or_else(|error| {
            panic!(
                "failed to collect embedded web assets from {}: {error}",
                embed_dir.display()
            )
        });

        // Skip uncompressed files when a .gz pre-compressed variant exists.
        // The .gz version is smaller in the binary; the server decompresses
        // on the fly for the rare client that doesn't accept gzip.
        let gz_paths: std::collections::HashSet<String> = entries
            .iter()
            .map(|(p, _)| p.clone())
            .filter(|p| p.ends_with(".gz"))
            .collect();
        entries.retain(|(path, _)| {
            let gz_variant = format!("{path}.gz");
            !gz_paths.contains(&gz_variant)
        });

        entries.sort_by(|(a, _), (b, _)| a.cmp(b));

        output.push_str("pub const HAS_EMBEDDED_WEB_UI: bool = true;\n");
        output.push_str("pub static EMBEDDED_WEB_FILES: &[(&str, &[u8])] = &[\n");
        for (asset_path, asset_source) in &entries {
            let source_str = asset_source.to_string_lossy().replace('\\', "/");
            output.push_str("    (\"");
            output.push_str(asset_path);
            output.push_str("\", include_bytes!(r#\"");
            output.push_str(&source_str);
            output.push_str("\"#)),\n");
        }
        output.push_str("];\n");
        println!("cargo:rerun-if-changed={}", embed_dir.display());
        for (_, file_path) in &entries {
            println!("cargo:rerun-if-changed={}", file_path.display());
        }
        println!(
            "cargo:rustc-env=SCRYER_EMBED_UI_DIR={}",
            embed_dir.display()
        );
    } else {
        output.push_str("pub const HAS_EMBEDDED_WEB_UI: bool = false;\n");
        output.push_str("pub static EMBEDDED_WEB_FILES: &[(&str, &[u8])] = &[];\n");
    }

    let mut output_file = fs::File::create(&output_path).expect("create embedded asset index");
    output_file
        .write_all(output.as_bytes())
        .expect("write embedded asset index");
    println!("cargo:rerun-if-env-changed=SCRYER_EMBED_UI_DIR");

    // SMG build-time assets (registration secret, CA cert, gateway URL)
    let smg_secret = env::var("SCRYER_SMG_REGISTRATION_SECRET").unwrap_or_default();
    let smg_ca = env::var("SCRYER_SMG_CA_CERT").unwrap_or_default();
    let smg_url = env::var("SCRYER_SMG_GRAPHQL_URL").unwrap_or_default();

    let smg_path = Path::new(&out_dir).join("smg_build_assets.rs");
    let smg_secret_val = if smg_secret.is_empty() {
        "None".to_string()
    } else {
        format!("Some({:?})", smg_secret)
    };
    let smg_ca_val = if smg_ca.is_empty() {
        "None".to_string()
    } else {
        format!("Some({:?})", smg_ca)
    };
    let smg_url_val = if smg_url.is_empty() {
        "None".to_string()
    } else {
        format!("Some({:?})", smg_url)
    };
    let smg_code = format!(
        "#[allow(dead_code)]\npub const SMG_REGISTRATION_SECRET: Option<&str> = {};\n\
         #[allow(dead_code)]\npub const SMG_CA_CERT: Option<&str> = {};\n\
         #[allow(dead_code)]\npub const SMG_GRAPHQL_URL: Option<&str> = {};\n",
        smg_secret_val, smg_ca_val, smg_url_val
    );
    fs::write(&smg_path, smg_code).expect("write smg_build_assets.rs");
    println!("cargo:rerun-if-env-changed=SCRYER_SMG_REGISTRATION_SECRET");
    println!("cargo:rerun-if-env-changed=SCRYER_SMG_CA_CERT");
    println!("cargo:rerun-if-env-changed=SCRYER_SMG_GRAPHQL_URL");
}

fn collect_files(root: &Path) -> Result<Vec<(String, PathBuf)>, io::Error> {
    let mut output = Vec::new();
    collect_files_recursive(root, root, &mut output)?;
    Ok(output)
}

fn collect_files_recursive(
    root: &Path,
    current: &Path,
    output: &mut Vec<(String, PathBuf)>,
) -> Result<(), io::Error> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let entry_path = entry.path();
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            collect_files_recursive(root, &entry_path, output)?;
            continue;
        }

        if !metadata.is_file() {
            continue;
        }

        let rel_path = entry_path
            .strip_prefix(root)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
            .to_string_lossy()
            .replace('\\', "/")
            .trim_start_matches('/')
            .to_string();
        output.push((rel_path, entry_path));
    }
    Ok(())
}
