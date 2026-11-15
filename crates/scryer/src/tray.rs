//! `scryer-tray`: the desktop wrapper around the Scryer server.
//!
//! The wrapper is not a second implementation of Scryer. It starts and stops
//! the same `scryer` binary that ships beside it, and shows the same web UI a
//! browser would — the app window and the browser can be pointed at the same
//! running server at the same time. Everything portable about that lives in
//! [`shared`]; the platform modules own only the window, the menu, and the
//! system integration each platform requires.
#![cfg_attr(windows, windows_subsystem = "windows")]
// disk-guard: this crate still calls the filesystem directly; remove this allow when it is converted to scryer_application::disk.
#![allow(clippy::disallowed_methods)]

/// The window class and messages `scryer.exe` posts to this binary. Both
/// binaries compile this file so they cannot drift apart.
#[cfg(windows)]
#[path = "tray_ipc.rs"]
mod tray_ipc;

/// The exit status `scryer` uses to ask this binary to relaunch the replaced
/// application bundle. Both binaries compile this file so they cannot drift
/// apart.
mod bundle_relaunch;

/// The per-user "start Scryer when I sign in" registration, shared verbatim
/// with `scryer.exe` and its MSI upgrade helper.
#[cfg(windows)]
mod windows_startup;

// Only the desktop platforms link the shared wrapper; Linux and the BSDs run
// Scryer as a service and get the stub `main` below. The tests still compile
// it everywhere, which is what keeps the portable core under test on CI's
// Linux runners.
#[cfg(any(windows, target_os = "macos", test))]
#[path = "tray/shared.rs"]
mod shared;

#[cfg(windows)]
#[path = "tray/windows.rs"]
mod windows;

#[cfg(target_os = "macos")]
#[path = "tray/macos.rs"]
mod macos;

#[cfg(windows)]
fn main() {
    if let Err(error) = windows::run() {
        windows::show_error("Scryer", &error);
        std::process::exit(1);
    }
}

#[cfg(target_os = "macos")]
fn main() {
    if let Err(error) = macos::run() {
        macos::show_error("Scryer", &error);
        std::process::exit(1);
    }
}

// The packaged Linux and BSD builds ship one set of binaries, so this one is
// built there too; it has nothing to do.
#[cfg(not(any(windows, target_os = "macos")))]
fn main() {
    eprintln!("scryer-tray is only supported on Windows and macOS");
    std::process::exit(1);
}
