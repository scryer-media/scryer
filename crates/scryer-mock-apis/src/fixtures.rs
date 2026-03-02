use std::path::{Path, PathBuf};

/// Resolves the fixtures directory relative to the workspace root.
/// Falls back to `SCRYER_FIXTURES_DIR` env var if set.
pub fn fixtures_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("SCRYER_FIXTURES_DIR") {
        return PathBuf::from(dir);
    }
    // Default: workspace_root/tests/fixtures
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    Path::new(manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests")
        .join("fixtures")
}

/// Load a fixture file as a String.
pub fn load_fixture(relative_path: &str) -> String {
    let path = fixtures_dir().join(relative_path);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to load fixture {}: {e}", path.display()))
}
