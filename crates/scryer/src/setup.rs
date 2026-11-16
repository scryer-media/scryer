#![cfg_attr(windows, windows_subsystem = "windows")]
// disk-guard: test, example and build-script code may call the filesystem directly.
#![cfg_attr(test, allow(clippy::disallowed_methods))]

#[cfg(windows)]
mod windows_installer_recovery;
#[cfg(windows)]
mod windows_setup;

#[cfg(windows)]
fn main() {
    windows_setup::run(|| {
        tokio::runtime::Runtime::new()
            .map_err(|e| e.to_string())?
            .block_on(windows_installer_recovery::install(env!(
                "CARGO_PKG_VERSION"
            )))
    });
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Scryer Setup is available on Windows only.");
}
