use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=build.rs");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let migrations_dir = manifest_dir.join("../scryer/src/db/migrations");

    if migrations_dir.exists() {
        let mut stack = vec![migrations_dir];

        while let Some(dir) = stack.pop() {
            println!("cargo:rerun-if-changed={}", dir.display());

            let entries = match fs::read_dir(dir) {
                Ok(entries) => entries,
                Err(_) => continue,
            };

            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }

                if path.extension().is_some_and(|ext| ext == "sql") {
                    println!("cargo:rerun-if-changed={}", path.display());
                }
            }
        }
    }
}
