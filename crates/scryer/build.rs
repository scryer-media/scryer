use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use scryer_runtime_info::{determine_build_lane, validate_build_lane_assertion};

fn main() {
    let compiled_build_lane = compiled_build_lane();
    compile_windows_resources();
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR is not set");
    let output_path = Path::new(&out_dir).join("embedded_ui_assets.rs");
    let blob_path = Path::new(&out_dir).join("embedded_ui_assets.bin");
    let mut output = String::new();
    let mut blob = Vec::new();

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

        let entries_by_path: HashMap<String, PathBuf> = entries.iter().cloned().collect();
        for (path, _) in &entries {
            if !requires_brotli_sidecar(path) {
                continue;
            }

            let brotli_variant = format!("{path}.br");
            if !entries_by_path.contains_key(&brotli_variant) {
                panic!(
                    "SCRYER_EMBED_UI_DIR is missing required Brotli sidecar '{}' for '{}'",
                    brotli_variant, path
                );
            }
        }

        // Embed only the Brotli sidecar for compressible assets. Raw copies are
        // omitted from the binary and the server derives gzip/raw variants on demand.
        entries.retain(|(path, _)| {
            if path.ends_with(".gz") {
                return false;
            }

            if path.ends_with(".br") {
                return true;
            }

            !requires_brotli_sidecar(path)
        });

        entries.sort_by(|(a, _), (b, _)| a.cmp(b));

        output.push_str("pub const HAS_EMBEDDED_WEB_UI: bool = true;\n");
        output.push_str("pub static EMBEDDED_WEB_FILES: &[EmbeddedWebAsset] = &[\n");
        for (asset_path, asset_source) in &entries {
            let bytes = fs::read(asset_source).unwrap_or_else(|error| {
                panic!(
                    "failed to read embedded web asset {}: {error}",
                    asset_source.display()
                )
            });
            let offset = blob.len();
            let length = bytes.len();
            blob.extend_from_slice(&bytes);
            output.push_str(&format!(
                "    EmbeddedWebAsset {{ path: {asset_path:?}, offset: {offset}, length: {length} }},\n"
            ));
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
        output.push_str("pub static EMBEDDED_WEB_FILES: &[EmbeddedWebAsset] = &[];\n");
    }

    output.push_str(
        "pub static EMBEDDED_WEB_BLOB: &[u8] = \
         include_bytes!(concat!(env!(\"OUT_DIR\"), \"/embedded_ui_assets.bin\"));\n",
    );

    let mut output_file = fs::File::create(&output_path).expect("create embedded asset index");
    output_file
        .write_all(output.as_bytes())
        .expect("write embedded asset index");
    fs::write(&blob_path, blob).expect("write embedded asset blob");
    println!("cargo:rerun-if-env-changed=SCRYER_EMBED_UI_DIR");

    // SMG build-time assets (registration secret, gateway URL)
    let smg_secret = env::var("SCRYER_SMG_REGISTRATION_SECRET").unwrap_or_default();
    let smg_url = env::var("SCRYER_SMG_GRAPHQL_URL").unwrap_or_default();

    let smg_path = Path::new(&out_dir).join("smg_build_assets.rs");
    let smg_secret_val = if smg_secret.is_empty() {
        "None".to_string()
    } else {
        format!("Some({:?})", smg_secret)
    };
    let smg_url_val = if smg_url.is_empty() {
        "None".to_string()
    } else {
        format!("Some({:?})", smg_url)
    };
    let smg_code = format!(
        "#[allow(dead_code)]\npub const SMG_REGISTRATION_SECRET: Option<&str> = {};\n\
         #[allow(dead_code)]\npub const SMG_GRAPHQL_URL: Option<&str> = {};\n",
        smg_secret_val, smg_url_val
    );
    fs::write(&smg_path, smg_code).expect("write smg_build_assets.rs");
    println!("cargo:rerun-if-env-changed=SCRYER_SMG_REGISTRATION_SECRET");
    println!("cargo:rerun-if-env-changed=SCRYER_SMG_GRAPHQL_URL");
    println!(
        "cargo:rustc-env=SCRYER_COMPILED_BUILD_LANE={}",
        compiled_build_lane.as_str()
    );
    println!("cargo:rerun-if-env-changed=SCRYER_BUILD_LANE");
}

fn compile_windows_resources() {
    println!("cargo:rerun-if-changed=resources/windows/scryer.rc");
    println!("cargo:rerun-if-changed=resources/windows/scryer.exe.manifest");
    println!("cargo:rerun-if-changed=resources/windows/scryer.ico");

    // Cross-compiled lints never link, and llvm-rc cannot resolve the manifest
    // from the preprocessed resource script.
    println!("cargo:rerun-if-env-changed=SCRYER_SKIP_WINDOWS_RESOURCES");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
        || env::var_os("SCRYER_SKIP_WINDOWS_RESOURCES").is_some()
    {
        return;
    }

    let version = env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION is set by Cargo");
    let resource_macros = windows_resource_macros(&version);

    embed_resource::compile("resources/windows/scryer.rc", &resource_macros)
        .manifest_required()
        .unwrap_or_else(|error| panic!("failed to embed Windows application manifest: {error}"));
}

fn windows_resource_macros(version: &str) -> [String; 4] {
    let version_parts = version
        .split('.')
        .map(|part| part.split(['-', '+']).next().unwrap_or(part))
        .map(|part| part.parse::<u16>().unwrap_or(0))
        .chain(std::iter::repeat(0))
        .take(3)
        .collect::<Vec<_>>();

    [
        format!("SCRYER_VERSION_MAJOR={}", version_parts[0]),
        format!("SCRYER_VERSION_MINOR={}", version_parts[1]),
        format!("SCRYER_VERSION_PATCH={}", version_parts[2]),
        format!("SCRYER_VERSION_STR=\"{version}\""),
    ]
}

fn compiled_build_lane() -> scryer_runtime_info::BinaryLane {
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let target_features = env::var("CARGO_CFG_TARGET_FEATURE").unwrap_or_default();
    let derived_lane = determine_build_lane(&target_arch, &target_features);
    validate_build_lane_assertion(env::var("SCRYER_BUILD_LANE").ok().as_deref(), derived_lane)
        .unwrap_or_else(|error| panic!("{error}"))
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

fn requires_brotli_sidecar(path: &str) -> bool {
    if path.ends_with(".br") || path.ends_with(".gz") {
        return false;
    }

    if Path::new(path).file_name().and_then(|name| name.to_str()) == Some("service-worker.js") {
        return false;
    }

    matches!(
        Path::new(path).extension().and_then(|ext| ext.to_str()),
        Some("js" | "css" | "svg" | "webmanifest" | "json")
    )
}
