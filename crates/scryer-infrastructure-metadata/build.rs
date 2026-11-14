use std::{env, fs, path::Path};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    validate_graphql_documents(
        Path::new(&manifest_dir).join("src/metadata/gateway/metadata_gateway"),
    );
}

fn validate_graphql_documents(root: impl AsRef<Path>) {
    let mut stack = vec![root.as_ref().to_path_buf()];
    while let Some(directory) = stack.pop() {
        let entries = fs::read_dir(&directory)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", directory.display()));
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path
                .extension()
                .is_none_or(|extension| extension != "graphql")
            {
                continue;
            }
            println!("cargo:rerun-if-changed={}", path.display());
            let document = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
            async_graphql_parser::parse_query(&document).unwrap_or_else(|error| {
                panic!("invalid GraphQL document {}: {error}", path.display())
            });
        }
    }
}
