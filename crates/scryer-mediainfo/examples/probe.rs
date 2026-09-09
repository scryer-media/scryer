//! Inspect the canonical native analysis contract without external probing tools.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or("usage: probe <media-path>")?;
    let analysis = scryer_mediainfo::analyze_catalog_file(std::path::Path::new(&path))?;
    println!("{}", serde_json::to_string_pretty(&analysis)?);
    Ok(())
}
