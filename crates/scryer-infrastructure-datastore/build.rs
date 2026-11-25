// disk-guard: test, example and build-script code may call the filesystem directly.
#![allow(clippy::disallowed_methods)]

use std::{
    env, fs,
    path::{Path, PathBuf},
};

#[path = "src/migrations/assets.rs"]
mod migration_assets;
#[path = "src/migrations/hook_ids.rs"]
mod migration_hook_ids;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    compile_migration_bundle(&manifest_dir);
}

fn compile_migration_bundle(manifest_dir: &Path) {
    let db_root = manifest_dir.join("../scryer/src/db");
    println!(
        "cargo:rerun-if-changed={}",
        db_root.join("migrations").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        db_root.join("postgres/migrations").display()
    );
    watch_tree(&db_root);

    let bundle = migration_assets::compile_source_bundle(&db_root)
        .unwrap_or_else(|error| panic!("failed to compile migration bundle: {error}"));
    let catalog_bytes = migration_assets::encode_catalog(&bundle.catalog)
        .unwrap_or_else(|error| panic!("failed to encode migration catalog: {error}"));
    let compressed_payload = zstd::stream::encode_all(
        bundle.payload_bytes.as_slice(),
        zstd::zstd_safe::max_c_level(),
    )
    .unwrap_or_else(|error| panic!("failed to compress migration payload: {error}"));

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let compressed_catalog =
        zstd::stream::encode_all(catalog_bytes.as_slice(), zstd::zstd_safe::max_c_level())
            .unwrap_or_else(|error| panic!("failed to compress migration catalog: {error}"));
    fs::write(
        out_dir.join("migration_catalog.json.zst"),
        compressed_catalog,
    )
    .expect("failed to write migration catalog");
    fs::write(
        out_dir.join("migration_payload.bin.zst"),
        compressed_payload,
    )
    .expect("failed to write migration payload");
}

fn watch_tree(root: &Path) {
    if !root.exists() {
        return;
    }

    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        println!("cargo:rerun-if-changed={}", dir.display());
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
    }
}
