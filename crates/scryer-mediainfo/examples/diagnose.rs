//! Opt-in bounded header diagnostics; never used to accept automatic imports.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or("usage: diagnose <media-path>")?;
    let result = scryer_mediainfo::diagnostics::diagnose_file(
        std::path::Path::new(&path),
        Default::default(),
    )?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
