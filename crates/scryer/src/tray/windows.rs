//! The Windows desktop wrapper: a notification-area icon and a WebView2 window.
//!
//! The tray window itself is invisible and exists only to own the shell icon,
//! the popup menu, and the messages `scryer.exe` posts back to its supervisor.
//! The app window is separate, created on first use, and hidden rather than
//! destroyed when the user closes it — which is what makes reopening instant
//! and what keeps the tray alive after the last window is gone.

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_KEY_EVENT_KIND, COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN,
    COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN, CreateCoreWebView2EnvironmentWithOptions,
    ICoreWebView2Controller, ICoreWebView2EnvironmentOptions,
};
use webview2_com::{
    AcceleratorKeyPressedEventHandler, CoTaskMemPWSTR,
    CreateCoreWebView2ControllerCompletedHandler, CreateCoreWebView2EnvironmentCompletedHandler,
    NavigationCompletedEventHandler, NavigationStartingEventHandler,
    NewWindowRequestedEventHandler,
};
use windows::Win32::Foundation::{E_POINTER, HWND as ComHwnd, RECT as ComRect};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::core::{PCWSTR, PWSTR};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, LPARAM, LRESULT, POINT, RECT,
    WPARAM,
};
use windows_sys::Win32::Graphics::Dwm::{
    DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontIndirectW,
    CreateSolidBrush, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DeleteDC, DeleteObject,
    DrawTextW, EndPaint, FW_BOLD, FillRect, GetMonitorInfoW, GetTextMetricsW, HDC, HFONT,
    InvalidateRect, LOGFONTW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, PAINTSTRUCT,
    SRCCOPY, SelectObject, SetBkMode, SetTextColor, TEXTMETRICW, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, RegCloseKey, RegOpenKeyExW, RegQueryValueExW,
};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;
use windows_sys::Win32::UI::HiDpi::{GetDpiForSystem, SystemParametersInfoForDpi};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_CONTROL,
};
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_SETVERSION, NIN_POPUPCLOSE,
    NIN_POPUPOPEN, NIN_SELECT, NOTIFYICON_VERSION_4, NOTIFYICONDATAW, NOTIFYICONIDENTIFIER,
    Shell_NotifyIconGetRect, Shell_NotifyIconW, ShellExecuteW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CREATESTRUCTW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
    DestroyWindow, DispatchMessageW, FindWindowW, GWLP_USERDATA, GetClientRect, GetCursorPos,
    GetMessageW, GetSystemMetrics, GetWindowLongPtrW, HMENU, HWND_TOPMOST, IDC_ARROW,
    IsWindowVisible, KillTimer, LoadCursorW, LoadIconW, MB_ICONERROR, MB_OK, MF_CHECKED,
    MF_SEPARATOR, MF_STRING, MF_UNCHECKED, MSG, MessageBoxW, NONCLIENTMETRICSW, PostMessageW,
    PostQuitMessage, RegisterClassW, SM_CXSCREEN, SM_CYSCREEN, SPI_GETNONCLIENTMETRICS, SW_HIDE,
    SW_SHOW, SW_SHOWNOACTIVATE, SW_SHOWNORMAL, SWP_NOACTIVATE, SWP_NOZORDER, SetForegroundWindow,
    SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON,
    TrackPopupMenu, TranslateMessage, WM_CLOSE, WM_CONTEXTMENU, WM_CREATE, WM_DESTROY,
    WM_DPICHANGED, WM_ENDSESSION, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT,
    WM_QUERYENDSESSION, WM_RBUTTONUP, WM_SIZE, WM_TIMER, WNDCLASSW, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_OVERLAPPEDWINDOW, WS_POPUP,
};

// The window class and message ids are shared with scryer.exe, which posts
// the restart message to this window.
use super::tray_ipc::{
    CLASS_NAME, OPEN_WINDOW_MESSAGE, RESTART_MESSAGE, SHUTDOWN_MESSAGE, TRAY_CALLBACK_MESSAGE,
    WEBVIEW_FAILED_MESSAGE,
};

use super::shared::{
    self, DEFAULT_PORT, INSTALLED_EXECUTABLES, PopoverContent, PopoverRow, SMOKE_SUCCESS_LINE,
    SMOKE_TIMEOUT, ServerSupervisor,
};
use application_updater::windows_process::{StopPolicy, stop_product_processes};
// The "start Scryer when I sign in" registration is shared verbatim with the
// MSI upgrade helper, which has to observe and restore it around a major
// upgrade. The tray owns the preference; it does not own a second copy of the
// registry code.
use crate::windows_startup::{register_startup, startup_enabled, unregister_startup};

const MUTEX_NAMESPACE: &str = "Global\\ScryerMedia.Scryer.Desktop.v1.Tray.";
const SCRYER_ICON_RESOURCE_ID: usize = 1;
/// The notification icon's id within this window. There is only ever one, and
/// every `NOTIFYICONDATAW` and `NOTIFYICONIDENTIFIER` has to name the same one.
const TRAY_ICON_ID: u32 = 1;
/// The app window's own class. It is private to this process, and versioned
/// like the tray class so a future layout change cannot be handed a window
/// created by an older build.
const APP_WINDOW_CLASS: &str = "ScryerMedia.Scryer.Desktop.v1.AppWindow";
/// The hover flyout's own class, versioned for the same reason.
const FLYOUT_WINDOW_CLASS: &str = "ScryerMedia.Scryer.Desktop.v1.Flyout";
/// How long the tray waits for a server that asked to be restarted to
/// actually exit before it stops waiting and kills the child.
const SERVER_EXIT_TIMEOUT: Duration = Duration::from_secs(30);

/// How long `--shutdown` waits for a running tray to stop itself (its own
/// server's graceful stop takes up to ten seconds of this), and how it stops
/// whatever is still running from the installation afterwards. Together with
/// finding the tray's window they keep `--shutdown` near twenty seconds at
/// worst, well inside what an installer waits on a custom action.
const SHUTDOWN_TRAY_EXIT_TIMEOUT: Duration = Duration::from_secs(12);
const SHUTDOWN_STOP_POLICY: StopPolicy = StopPolicy {
    ask_attempts: 8,
    ask_interval: Duration::from_millis(500),
    terminate_wait: Duration::from_secs(2),
};

/// How the tray stops a server it did not start when it exits. Short, because
/// it runs after the tray's own server has been given its graceful stop, and
/// because `--shutdown` is waiting on this tray while it runs.
const EXIT_STOP_POLICY: StopPolicy = StopPolicy {
    ask_attempts: 4,
    ask_interval: Duration::from_millis(500),
    terminate_wait: Duration::from_secs(1),
};

/// The app window's default size, and the smallest window WebView2 is asked to
/// lay the UI out in. These are logical (96-DPI) pixels; `create_app_window`
/// scales them by the system DPI so the window never opens below this size on
/// a high-density display.
const APP_WINDOW_WIDTH: i32 = 1280;
const APP_WINDOW_HEIGHT: i32 = 800;

/// Caption colors mirror the web UI's `--background`/`--foreground` tokens
/// (apps/scryer-web/app/globals.css) so the title bar reads as part of the
/// page. COLORREF byte order is 0x00BBGGRR.
const CAPTION_DARK_BACKGROUND: u32 = 0x001C_0D07; // #070d1c
const CAPTION_DARK_TEXT: u32 = 0x00FF_E5DB; // #dbe5ff
const CAPTION_LIGHT_BACKGROUND: u32 = 0x00FF_FFFF; // #ffffff
const CAPTION_LIGHT_TEXT: u32 = 0x0050_382D; // #2d3850

/// The WebView2 profile lives under the desktop profile directory, never
/// beside the executable: the wrapper is installed into Program Files, where
/// WebView2's default user-data folder would be unwritable and environment
/// creation would fail for every user.
const WEBVIEW_USER_DATA_DIR: &str = "WebView2";

/// The hover flyout's geometry, in logical (96-DPI) pixels. Every value is the
/// macOS popover's: the same 300-point width, its 14-point inset, the 10 points
/// between its blocks and the 3 points inside a row. `flyout_metrics` scales
/// them for the display.
const FLYOUT_WIDTH: i32 = shared::POPOVER_WIDTH as i32;
const FLYOUT_PADDING: i32 = 14;
const FLYOUT_BLOCK_GAP: i32 = 10;
const FLYOUT_ROW_GAP: i32 = 3;
/// The progress bar's height, and how far the flyout sits from the icon.
const FLYOUT_BAR_HEIGHT: i32 = 6;
const FLYOUT_ICON_GAP: i32 = 8;

/// Timer ids. The poll timer runs on the tray window and the close timer on the
/// flyout, so they could share an id; they do not, because a single id space
/// is what makes a stray `KillTimer` impossible to get wrong. The smoke
/// watchdog's id 1 belongs to a window neither of these ever sees.
const FLYOUT_POLL_TIMER: usize = 100;
const FLYOUT_CLOSE_TIMER: usize = 101;

/// The notification-area registration retry, on the tray window. Shipped
/// Scryer builds used this id and this interval; it is kept so the behavior is
/// unchanged for a user upgrading in place.
const ICON_RETRY_TIMER: usize = 1;
const ICON_RETRY_INTERVAL_MS: u32 = 1_000;

/// How often the tray looks for a finished fetch while the flyout is up, and
/// how often it asks the server again. Nothing is polled while the flyout is
/// hidden, so an idle tray makes no requests at all.
const FLYOUT_POLL_INTERVAL_MS: u32 = 200;
const FLYOUT_REFRESH_INTERVAL: Duration = Duration::from_secs(2);
/// How long the flyout survives the pointer leaving it. The gap between the
/// icon and the flyout is crossed with the pointer inside neither, so closing
/// immediately would make the flyout unreachable.
const FLYOUT_CLOSE_GRACE_MS: u32 = 250;

/// What the flyout shows before the first fetch has answered — the same line
/// the macOS popover shows in the same moment.
const FLYOUT_PLACEHOLDER: &str = "Checking Scryer…";

/// `NIN_KEYSELECT`, which windows-sys does not name: the notification the shell
/// sends when the icon is chosen from the keyboard rather than the mouse. It is
/// `NIN_SELECT + 1` in `shellapi.h` and has been since the notification area
/// gained keyboard access.
const NIN_KEYSELECT: u32 = NIN_SELECT + 1;

/// The virtual key for Ctrl+W, which hides the app window. `VK_W` has no name
/// in the Windows headers either: the letter keys are their ASCII codes.
const HIDE_WINDOW_VIRTUAL_KEY: u32 = b'W' as u32;

const MENU_OPEN: u32 = 1;
const MENU_START: u32 = 2;
const MENU_STOP: u32 = 3;
const MENU_RESTART: u32 = 4;
const MENU_OPEN_LOGS: u32 = 5;
const MENU_TOGGLE_STARTUP: u32 = 6;
const MENU_EXIT: u32 = 7;
const MENU_OPEN_BROWSER: u32 = 8;

enum LaunchMode {
    Interactive,
    Login,
    Shutdown,
    UnregisterStartup,
    UninstallCleanup,
    WebviewSmoke,
}

pub(super) fn run() -> Result<(), String> {
    match launch_mode()? {
        LaunchMode::UnregisterStartup => return unattended(unregister_startup()),
        LaunchMode::Shutdown => return unattended(shutdown_existing_instance()),
        LaunchMode::UninstallCleanup => {
            uninstall_cleanup();
            return Ok(());
        }
        LaunchMode::WebviewSmoke => return smoke::run(),
        LaunchMode::Interactive | LaunchMode::Login => {}
    }

    let instance = InstanceGuard::acquire()?;
    if !instance.is_primary() {
        // A login start (the Run key, or Restart Manager relaunching the tray
        // after an install) is not a request to see the window; only a user's
        // own launch is. FindWindowW is session-local. A same-session
        // invocation opens the existing window; a second session for the same
        // user simply leaves the owner alone.
        if !matches!(launch_mode()?, LaunchMode::Login) {
            let _ = signal_existing_instance(OPEN_WINDOW_MESSAGE);
        }
        return Ok(());
    }
    register_for_restart();

    let profile_dir = shared::desktop_profile_dir()?;
    let supervisor = ServerSupervisor::new(profile_dir, DEFAULT_PORT);
    supervisor.ensure_profile_dirs()?;

    // WebView2 is a COM API and this is the thread that will own it. The tray
    // works without it — the fallback is the user's browser — so a failure
    // here is reported and then ignored.
    initialize_com();

    let class_name = wide(CLASS_NAME);
    let window_class = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        lpszClassName: class_name.as_ptr(),
        ..Default::default()
    };
    // SAFETY: The class name and callback remain valid for process lifetime.
    if unsafe { RegisterClassW(&window_class) } == 0 {
        return Err(format!(
            "failed to register Scryer tray window class: {}",
            std::io::Error::last_os_error()
        ));
    }

    // SAFETY: The registered class name is valid and this creates an invisible top-level window.
    let window = unsafe {
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            ptr::null(),
            0,
            0,
            0,
            0,
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null(),
        )
    };
    if window.is_null() {
        return Err(format!(
            "failed to create Scryer tray window: {}",
            std::io::Error::last_os_error()
        ));
    }

    let mode = launch_mode()?;
    let state = Box::new(TrayState::new(
        supervisor,
        matches!(mode, LaunchMode::Login),
    ));
    let state = Box::into_raw(state);
    // SAFETY: The Box allocation remains alive until after the message loop exits.
    unsafe { SetWindowLongPtrW(window, GWLP_USERDATA, state.cast::<c_void>() as isize) };

    // SAFETY: State is initialized and uniquely owned by this UI thread.
    let startup_result = unsafe { (&mut *state).initialize(window) };
    if let Err(error) = startup_result {
        // SAFETY: The window is ours and the Box was allocated above.
        unsafe {
            DestroyWindow(window);
            drop(Box::from_raw(state));
        }
        return Err(error);
    }

    let mut message = MSG::default();
    loop {
        // SAFETY: `message` is valid writable storage for the duration of the call.
        let result = unsafe { GetMessageW(&mut message, ptr::null_mut(), 0, 0) };
        if result == -1 {
            // SAFETY: The Box was allocated above and is no longer accessed after this drop.
            unsafe { drop(Box::from_raw(state)) };
            return Err(format!(
                "failed to receive Scryer tray message: {}",
                std::io::Error::last_os_error()
            ));
        }
        if result == 0 {
            break;
        }
        // SAFETY: The message was populated by GetMessageW.
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }

    // SAFETY: The state is no longer reachable once the window was destroyed.
    unsafe { drop(Box::from_raw(state)) };
    drop(instance);
    Ok(())
}

/// Finish a mode the installer runs. `main` reports an error in a dialog, and
/// a dialog nobody is there to dismiss would hang a silent install or
/// uninstall on this process, so these modes report to stderr and leave the
/// exit code to say the rest.
fn unattended(result: Result<(), String>) -> Result<(), String> {
    if let Err(error) = result {
        eprintln!("Scryer: {error}");
        std::process::exit(1);
    }
    Ok(())
}

fn launch_mode() -> Result<LaunchMode, String> {
    let mut args = std::env::args_os();
    let _program = args.next();
    match args.next() {
        None => Ok(LaunchMode::Interactive),
        Some(value) if value == "--login-start" => Ok(LaunchMode::Login),
        Some(value) if value == "--shutdown" => Ok(LaunchMode::Shutdown),
        Some(value) if value == "--unregister-startup" => Ok(LaunchMode::UnregisterStartup),
        Some(value) if value == "--uninstall-cleanup" => Ok(LaunchMode::UninstallCleanup),
        Some(value) if value == "--webview-smoke" => Ok(LaunchMode::WebviewSmoke),
        Some(value) if value == "--version" || value == "-V" => {
            // The shipped format, kept verbatim: the packaging validation
            // script reads this line.
            println!("scryer-tray {}", env!("CARGO_PKG_VERSION"));
            std::process::exit(0);
        }
        Some(value) => Err(format!(
            "unrecognized scryer-tray argument: {}",
            value.to_string_lossy()
        )),
    }
}

/// Put this thread into a single-threaded apartment for WebView2.
///
/// The result is deliberately discarded: already-initialized is not a
/// failure, and a genuinely broken COM apartment surfaces moments later as a
/// WebView2 environment-creation error, which already falls back to the
/// browser.
fn initialize_com() {
    // SAFETY: This is the tray's only thread and it stays in this apartment
    // for the process lifetime.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
}

fn tray_mutex_name() -> Result<String, String> {
    let username = std::env::var("USERNAME")
        .map_err(|_| "USERNAME is not set; cannot scope the Scryer tray instance".to_string())?;
    let domain = std::env::var("USERDOMAIN").unwrap_or_default();
    Ok(tray_mutex_name_for_user(&domain, &username))
}

fn tray_mutex_name_for_user(domain: &str, username: &str) -> String {
    let identity = if domain.is_empty() {
        username.to_string()
    } else {
        format!("{domain}\\{username}")
    };
    let encoded = identity
        .encode_utf16()
        .map(|unit| format!("{unit:04X}"))
        .collect::<String>();
    format!("{MUTEX_NAMESPACE}{encoded}")
}

struct InstanceGuard(HANDLE, bool);

impl InstanceGuard {
    fn acquire() -> Result<Self, String> {
        let name = wide(&tray_mutex_name()?);
        // SAFETY: `name` is nul-terminated and remains live for this call.
        let handle = unsafe { CreateMutexW(ptr::null(), 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(format!(
                "failed to create Scryer tray instance mutex: {}",
                std::io::Error::last_os_error()
            ));
        }
        // SAFETY: GetLastError reads the result associated with CreateMutexW above.
        let is_primary = unsafe { GetLastError() } != ERROR_ALREADY_EXISTS;
        Ok(Self(handle, is_primary))
    }

    fn is_primary(&self) -> bool {
        self.1
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        // SAFETY: This guard owns the mutex handle returned by CreateMutexW.
        unsafe { CloseHandle(self.0) };
    }
}

fn signal_existing_instance(message: u32) -> Result<(), String> {
    let class_name = wide(CLASS_NAME);
    for _ in 0..40 {
        // SAFETY: The class name is a valid nul-terminated UTF-16 string.
        let window = unsafe { FindWindowW(class_name.as_ptr(), ptr::null()) };
        if !window.is_null() {
            // SAFETY: The target is a same-user Scryer tray window identified by its private class.
            if unsafe { PostMessageW(window, message, 0, 0) } == 0 {
                return Err(format!(
                    "failed to signal existing Scryer tray instance: {}",
                    std::io::Error::last_os_error()
                ));
            }
            return Ok(());
        }
        thread::sleep(Duration::from_millis(50));
    }
    Err("another Scryer tray instance is starting but did not create its window".to_string())
}

/// Ask Restart Manager to relaunch the tray as a login start after it closes
/// the tray for an install. Crash, hang and reboot restarts are excluded: the
/// Run key already starts the tray at sign-in, and a crash loop must not be
/// automated. Failure only costs the relaunch, so it is ignored.
fn register_for_restart() {
    use windows_sys::Win32::System::Recovery::{
        RESTART_NO_CRASH, RESTART_NO_HANG, RESTART_NO_REBOOT, RegisterApplicationRestart,
    };

    let command_line = wide("--login-start");
    // SAFETY: The command line is a NUL-terminated UTF-16 buffer that outlives the call.
    unsafe {
        RegisterApplicationRestart(
            command_line.as_ptr(),
            RESTART_NO_CRASH | RESTART_NO_HANG | RESTART_NO_REBOOT,
        )
    };
}

/// `--shutdown`: stop this installation's Scryer entirely, for an installer
/// that is about to replace its files.
///
/// The running tray is asked first, because it stops the server it owns
/// gracefully. Then every `scryer.exe` and `scryer-tray.exe` still running
/// from this installation — a server the tray adopted, one orphaned by a tray
/// that was killed, or a tray that did not exit — is asked to stop and
/// terminated if it does not. Succeeds only when nothing is left running.
fn shutdown_existing_instance() -> Result<(), String> {
    let supervisor = ServerSupervisor::new(shared::desktop_profile_dir()?, DEFAULT_PORT);
    supervisor.ensure_restore_idle()?;
    let tray_result = stop_running_tray();
    supervisor.ensure_restore_idle()?;
    let install_dir = shared::installation_dir()?;
    let report = stop_product_processes(
        &INSTALLED_EXECUTABLES,
        &install_dir,
        &[std::process::id()],
        SHUTDOWN_STOP_POLICY,
    );
    if !report.all_stopped() {
        return Err(format!(
            "Scryer processes {:?} in {} are still running after being asked to stop and terminated",
            report.remaining,
            install_dir.display()
        ));
    }
    // A tray that timed out but was then terminated has stopped; the
    // installation is free either way.
    if let Err(error) = tray_result {
        eprintln!("Scryer: {error}; it was stopped directly");
    }
    Ok(())
}

/// Ask the running tray of this user, if any, to exit, and wait for it.
fn stop_running_tray() -> Result<(), String> {
    let instance = InstanceGuard::acquire()?;
    if instance.is_primary() {
        return Ok(());
    }
    drop(instance);
    signal_existing_instance(SHUTDOWN_MESSAGE)?;

    let class_name = wide(CLASS_NAME);
    let deadline = Instant::now() + SHUTDOWN_TRAY_EXIT_TIMEOUT;
    while Instant::now() < deadline {
        // SAFETY: The class name is a valid nul-terminated UTF-16 string.
        if unsafe { FindWindowW(class_name.as_ptr(), ptr::null()) }.is_null() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err("timed out waiting for the existing Scryer tray to stop".to_string())
}

/// How often, and how far apart, an uninstall tries to remove the profile.
/// WebView2's browser processes outlive the tray by a moment and hold its
/// data open until they exit.
const PROFILE_REMOVAL_ATTEMPTS: u32 = 20;
const PROFILE_REMOVAL_RETRY_DELAY: Duration = Duration::from_millis(500);

/// Remove what Scryer keeps for this Windows user: the profile's database,
/// logs and WebView2 data, and the Credential Manager key. The MSI runs this
/// on uninstall once the tray has stopped. The uninstall may be silent, so
/// nothing here may raise a dialog, and a leftover file must not fail it.
fn uninstall_cleanup() {
    let profile_dir = match shared::desktop_profile_dir() {
        Ok(profile_dir) => profile_dir,
        Err(error) => {
            eprintln!("Scryer: {error}");
            return;
        }
    };
    for attempt in 1..=PROFILE_REMOVAL_ATTEMPTS {
        let problems = shared::remove_desktop_profile(&profile_dir);
        if problems.is_empty() {
            break;
        }
        if attempt == PROFILE_REMOVAL_ATTEMPTS {
            for problem in problems {
                eprintln!("Scryer: {problem}");
            }
            break;
        }
        thread::sleep(PROFILE_REMOVAL_RETRY_DELAY);
    }
    // The key goes only once the database it encrypts is gone. A database
    // that could not be removed would otherwise be unreadable after a
    // reinstall, and the server refuses to start rather than replace a lost
    // key.
    if profile_dir.join("scryer.db").exists() {
        eprintln!("Scryer: the database is still present, keeping its Credential Manager key");
        return;
    }
    // `platform_keystores` is the same call the server makes to find the key,
    // so the credential this deletes is exactly the one the server wrote —
    // including the desktop profile's dedicated Credential Manager namespace,
    // which is selected from the profile path rather than named here.
    for store in
        scryer_infrastructure_datastore::keystore::platform_keystores(Some(profile_dir.clone()))
    {
        if let Err(error) = store.delete_key() {
            eprintln!(
                "Scryer: failed to remove the {} entry: {error}",
                store.name()
            );
        }
    }
}

struct TrayState {
    supervisor: ServerSupervisor,
    login_start: bool,
    icon_added: bool,
    /// The app window, once it has been created. It is created hidden and
    /// shown by the WebView2 controller callback, so the window exists for a
    /// moment before there is anything in it — reopening during that moment
    /// shows an empty frame, which is what every WebView2 host does and is
    /// still better than the alternative of ignoring the user's click.
    app_window: Option<HWND>,
    /// Set once the app window class has been registered with the system,
    /// which may only happen once per process.
    app_class_registered: bool,
    /// The hover flyout, created on first hover and then hidden and reused.
    /// Destroying it per hover would throw away its fonts and its window every
    /// time the pointer crossed the icon.
    flyout: Option<HWND>,
    flyout_class_registered: bool,
    /// Where the last hover happened, so a flyout that grows or shrinks with a
    /// new snapshot stays anchored to the icon it was opened from.
    flyout_anchor: POINT,
    /// Whether the poll timer is running. `SetTimer` on a live id restarts it
    /// rather than failing, so this is what keeps a second hover from resetting
    /// the refresh clock.
    status_polling: bool,
    /// Where a finished fetch leaves its result for the message loop to draw.
    status_result: Arc<Mutex<Option<PopoverContent>>>,
    status_fetching: Arc<AtomicBool>,
    status_fetched_at: Option<Instant>,
    /// Set once a fetch has answered, so a reopened flyout shows the last
    /// status rather than the placeholder again.
    status_answered: bool,
    /// Whether the shell accepted `NOTIFYICON_VERSION_4` for the icon, which
    /// decides which member of each click's message pair `tray_callback`
    /// answers.
    version4: bool,
    /// Whether `TrackPopupMenu`'s modal loop is running. That loop dispatches
    /// queued messages, so a second menu request arriving during it must be
    /// dropped rather than opening a menu under the menu.
    menu_open: bool,
}

impl TrayState {
    fn new(supervisor: ServerSupervisor, login_start: bool) -> Self {
        Self {
            supervisor,
            login_start,
            icon_added: false,
            app_window: None,
            app_class_registered: false,
            flyout: None,
            flyout_class_registered: false,
            flyout_anchor: POINT { x: 0, y: 0 },
            status_polling: false,
            status_result: Arc::new(Mutex::new(None)),
            status_fetching: Arc::new(AtomicBool::new(false)),
            status_fetched_at: None,
            status_answered: false,
            version4: false,
            menu_open: false,
        }
    }

    unsafe fn initialize(&mut self, window: HWND) -> Result<(), String> {
        // Explorer can still be starting when the Run-key entry launches the
        // tray. Keep the local server alive and retry the icon rather than
        // treating that transient shell state as a failed desktop launch.
        // SAFETY: The window is live for the duration of tray initialization,
        // and it owns the retry timer until it is destroyed.
        if unsafe { self.add_icon(window) }.is_err() {
            // SAFETY: As above.
            unsafe { SetTimer(window, ICON_RETRY_TIMER, ICON_RETRY_INTERVAL_MS, None) };
        }
        let started = if self.login_start {
            self.supervisor.start()
        } else {
            if let Err(error) = self.enable_startup() {
                show_error("Scryer", &error);
            }
            self.open_scryer(window)
        };
        // A server that cannot start leaves the tray up: its menu is how the
        // user tries again and finds the log, and a tray that vanished would
        // leave them neither.
        if let Err(error) = started {
            show_error(
                "Scryer",
                &format!(
                    "{error}\n\nScryer is still in the notification area: choose Open Scryer to try again, or Open Logs."
                ),
            );
        }
        Ok(())
    }

    unsafe fn add_icon(&mut self, window: HWND) -> Result<(), String> {
        // SAFETY: Resource ID 1 is the application-owned multi-resolution Scryer icon.
        let icon = unsafe {
            LoadIconW(
                GetModuleHandleW(ptr::null()),
                SCRYER_ICON_RESOURCE_ID as *const u16,
            )
        };
        if icon.is_null() {
            return Err(format!(
                "failed to load Scryer tray icon: {}",
                std::io::Error::last_os_error()
            ));
        }
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: window,
            uID: TRAY_ICON_ID,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: TRAY_CALLBACK_MESSAGE,
            hIcon: icon,
            ..Default::default()
        };
        // `szTip` still names the icon in the overflow flyout and to a screen
        // reader. `NIF_SHOWTIP` is deliberately absent: under
        // `NOTIFYICON_VERSION_4` it restores the plain textual tooltip, and the
        // whole point of the version is that the shell suppresses that tooltip
        // and sends `NIN_POPUPOPEN` so the application can draw a richer one.
        write_wide_buffer(&mut data.szTip, "Scryer");
        // SAFETY: `data` is initialized and remains live through the system call.
        if unsafe { Shell_NotifyIconW(NIM_ADD, &data) } == 0 {
            return Err(format!(
                "failed to add Scryer tray icon: {}",
                std::io::Error::last_os_error()
            ));
        }
        self.icon_added = true;

        // Version 4 is what delivers hover notifications, and it re-encodes the
        // callback: the event moves into the low word of `lparam` and the
        // anchor point into `wparam`. `window_proc` decodes both. A shell that
        // refuses the version leaves the icon on the legacy encoding, which
        // that decode still understands, so this is not fatal — but
        // `tray_callback` has to know which encoding is live, because a
        // version-4 shell sends a click as both its own notification and the
        // legacy mouse message.
        data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        // SAFETY: As above; `NIM_SETVERSION` reads `cbSize`, the window, the id
        // and the version union, all of which are set.
        self.version4 = unsafe { Shell_NotifyIconW(NIM_SETVERSION, &data) } != 0;
        Ok(())
    }

    unsafe fn remove_icon(&mut self, window: HWND) {
        if !self.icon_added {
            return;
        }
        let data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: window,
            uID: TRAY_ICON_ID,
            ..Default::default()
        };
        // SAFETY: The notification data identifies the icon added by this process.
        unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
        self.icon_added = false;
    }

    /// Show the Scryer UI in the app window, starting the server first.
    fn open_scryer(&mut self, tray_window: HWND) -> Result<(), String> {
        // The window the user just asked for would come up over the flyout the
        // same pointer opened, so the flyout goes first — and without its
        // grace period, because the pointer is not coming back.
        self.hide_flyout(tray_window);
        self.wait_for_ready_server()?;

        if let Some(window) = self.app_window {
            show_and_focus(window);
            return Ok(());
        }

        match self.create_app_window(tray_window) {
            Ok(()) => Ok(()),
            Err(error) => {
                // A machine without the WebView2 runtime is a supported
                // machine: it gets the browser, exactly as every build before
                // this one did.
                eprintln!("Scryer: {error}; opening the browser instead");
                self.open_in_browser()
            }
        }
    }

    /// Open the Scryer UI in the user's default browser. This is also the
    /// fallback whenever the embedded browser cannot be started.
    fn open_in_browser(&mut self) -> Result<(), String> {
        self.wait_for_ready_server()?;
        open_target(&shared::app_url(self.supervisor.port()))
    }

    fn wait_for_ready_server(&mut self) -> Result<(), String> {
        self.supervisor.start()?;
        self.supervisor.wait_until_ready()
    }

    /// Create the app window and start WebView2 in it.
    ///
    /// Everything after the window itself is asynchronous: WebView2 delivers
    /// its environment and controller through the message loop, so the tray
    /// state is never borrowed across a nested message pump.
    fn create_app_window(&mut self, tray_window: HWND) -> Result<(), String> {
        self.register_app_window_class()?;
        let user_data_folder = self.supervisor.profile_dir().join(WEBVIEW_USER_DATA_DIR);
        std::fs::create_dir_all(&user_data_folder).map_err(|error| {
            format!(
                "failed to create the WebView2 profile at {}: {error}",
                user_data_folder.display()
            )
        })?;

        let window = create_app_window(APP_WINDOW_WIDTH, APP_WINDOW_HEIGHT, "Scryer")?;
        let url = shared::app_url(self.supervisor.port());
        let origin = shared::app_origin(self.supervisor.port());
        if let Err(error) = start_webview(tray_window, window, &user_data_folder, url, origin) {
            // SAFETY: The window was created above and has not been handed out.
            unsafe { DestroyWindow(window) };
            return Err(error);
        }
        self.app_window = Some(window);
        Ok(())
    }

    fn register_app_window_class(&mut self) -> Result<(), String> {
        if self.app_class_registered {
            return Ok(());
        }
        let class_name = wide(APP_WINDOW_CLASS);
        // SAFETY: Resource ID 1 is the application-owned multi-resolution Scryer icon,
        // and IDC_ARROW is a system cursor that is always present.
        let (icon, cursor, instance) = unsafe {
            let instance = GetModuleHandleW(ptr::null());
            (
                LoadIconW(instance, SCRYER_ICON_RESOURCE_ID as *const u16),
                LoadCursorW(ptr::null_mut(), IDC_ARROW),
                instance,
            )
        };
        let window_class = WNDCLASSW {
            lpfnWndProc: Some(app_window_proc),
            lpszClassName: class_name.as_ptr(),
            hInstance: instance,
            hIcon: icon,
            hCursor: cursor,
            ..Default::default()
        };
        // SAFETY: The class name and callback remain valid for process lifetime.
        if unsafe { RegisterClassW(&window_class) } == 0 {
            return Err(format!(
                "failed to register the Scryer app window class: {}",
                std::io::Error::last_os_error()
            ));
        }
        self.app_class_registered = true;
        Ok(())
    }

    /// WebView2 could not be started after the window already existed. Tear
    /// the window down and fall back to the browser.
    fn webview_failed(&mut self) {
        if let Some(window) = self.app_window.take() {
            // SAFETY: The window belongs to this process and nothing else
            // holds it once it is taken out of the state.
            unsafe { DestroyWindow(window) };
        }
        if let Err(error) = self.open_in_browser() {
            show_error("Scryer", &error);
        }
    }

    /// Restart a server that asked to be restarted from its own UI.
    ///
    /// Unlike the menu path, the server is already tearing itself down, so
    /// the old process must be gone before the replacement starts:
    /// `ServerSupervisor::start` returns early while the port still answers,
    /// so starting without waiting would silently leave the user with no
    /// server at all.
    fn restart_requested_by_server(&mut self) -> Result<(), String> {
        self.supervisor.wait_for_exit(SERVER_EXIT_TIMEOUT);
        self.supervisor.start()?;
        self.supervisor.wait_until_ready()
    }

    // -- the hover flyout ----------------------------------------------------

    /// Show the flyout beside the icon the pointer is on.
    ///
    /// A flyout that cannot be created is reported to the log and then
    /// forgotten: a hover is not a request, and a modal dialog every time the
    /// pointer crossed the notification area would be worse than no flyout.
    fn show_flyout(&mut self, tray_window: HWND, anchor: POINT) {
        let flyout = match self.ensure_flyout() {
            Ok(flyout) => flyout,
            Err(error) => {
                eprintln!("Scryer: {error}");
                return;
            }
        };
        // SAFETY: The flyout belongs to this thread; killing a timer that is
        // not set is a no-op.
        unsafe { KillTimer(flyout, FLYOUT_CLOSE_TIMER) };
        self.flyout_anchor = anchor;
        if !self.status_answered {
            set_flyout_content(
                flyout,
                PopoverContent {
                    status: None,
                    rows: Vec::new(),
                    message: Some(FLYOUT_PLACEHOLDER.to_string()),
                },
            );
        }
        self.place_flyout(tray_window, flyout);
        // SAFETY: The flyout is a live window owned by this thread, and
        // `SW_SHOWNOACTIVATE` is what keeps the focused window focused.
        unsafe { ShowWindow(flyout, SW_SHOWNOACTIVATE) };
        self.begin_status_polling(tray_window);
    }

    /// The pointer left the icon. It may be on its way into the flyout, which
    /// cancels this by killing the timer from its own message procedure.
    fn schedule_flyout_close(&self) {
        if let Some(flyout) = self.flyout {
            // SAFETY: The flyout is a live window owned by this thread.
            unsafe { SetTimer(flyout, FLYOUT_CLOSE_TIMER, FLYOUT_CLOSE_GRACE_MS, None) };
        }
    }

    /// Close the flyout now, and stop asking the server anything.
    fn hide_flyout(&mut self, tray_window: HWND) {
        if let Some(flyout) = self.flyout {
            // SAFETY: The flyout is a live window owned by this thread.
            unsafe {
                KillTimer(flyout, FLYOUT_CLOSE_TIMER);
                ShowWindow(flyout, SW_HIDE);
            }
        }
        self.stop_status_polling(tray_window);
    }

    fn ensure_flyout(&mut self) -> Result<HWND, String> {
        if let Some(flyout) = self.flyout {
            return Ok(flyout);
        }
        self.register_flyout_class()?;
        let flyout = create_flyout_window()?;
        self.flyout = Some(flyout);
        Ok(flyout)
    }

    fn register_flyout_class(&mut self) -> Result<(), String> {
        if self.flyout_class_registered {
            return Ok(());
        }
        let class_name = wide(FLYOUT_WINDOW_CLASS);
        // SAFETY: IDC_ARROW is a system cursor that is always present.
        let (cursor, instance) = unsafe {
            (
                LoadCursorW(ptr::null_mut(), IDC_ARROW),
                GetModuleHandleW(ptr::null()),
            )
        };
        let window_class = WNDCLASSW {
            lpfnWndProc: Some(flyout_window_proc),
            lpszClassName: class_name.as_ptr(),
            hInstance: instance,
            hCursor: cursor,
            ..Default::default()
        };
        // SAFETY: The class name and callback remain valid for process lifetime.
        if unsafe { RegisterClassW(&window_class) } == 0 {
            return Err(format!(
                "failed to register the Scryer flyout window class: {}",
                std::io::Error::last_os_error()
            ));
        }
        self.flyout_class_registered = true;
        Ok(())
    }

    /// Size the flyout to whatever it is currently showing and put it beside
    /// the icon. A snapshot with fewer rows than the last one shrinks the
    /// window rather than leaving empty space under the queue.
    fn place_flyout(&self, tray_window: HWND, flyout: HWND) {
        let Some((width, height)) = flyout_size(flyout) else {
            return;
        };
        position_flyout(
            flyout,
            self.flyout_anchor,
            tray_icon_rect(tray_window),
            width,
            height,
        );
    }

    fn begin_status_polling(&mut self, tray_window: HWND) {
        if self.status_polling {
            return;
        }
        // The next tick starts a fetch immediately; a hover must not wait out a
        // refresh interval for its first answer.
        self.status_fetched_at = None;
        // SAFETY: The tray window is live and the timer id is private to it.
        unsafe {
            SetTimer(
                tray_window,
                FLYOUT_POLL_TIMER,
                FLYOUT_POLL_INTERVAL_MS,
                None,
            )
        };
        self.status_polling = true;
    }

    fn stop_status_polling(&mut self, tray_window: HWND) {
        if !self.status_polling {
            return;
        }
        // SAFETY: As above.
        unsafe { KillTimer(tray_window, FLYOUT_POLL_TIMER) };
        self.status_polling = false;
    }

    /// Timer tick: draw whatever the fetch thread has left, and start the next
    /// fetch once the current answer is stale.
    ///
    /// The flyout closes itself after its own grace period, so this is also
    /// where the tray notices that it has, and stops polling.
    fn poll_status(&mut self, tray_window: HWND) {
        let Some(flyout) = self.flyout else {
            self.stop_status_polling(tray_window);
            return;
        };
        // SAFETY: The flyout is a live window owned by this thread.
        if unsafe { IsWindowVisible(flyout) } == 0 {
            self.stop_status_polling(tray_window);
            return;
        }

        let finished = self
            .status_result
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(content) = finished {
            self.status_answered = true;
            set_flyout_content(flyout, content);
            self.place_flyout(tray_window, flyout);
            // SAFETY: The flyout is live, and a null rectangle invalidates all
            // of it.
            unsafe { InvalidateRect(flyout, ptr::null(), 0) };
        }

        let due = self
            .status_fetched_at
            .is_none_or(|started| started.elapsed() >= FLYOUT_REFRESH_INTERVAL);
        if due && !self.status_fetching.load(Ordering::Acquire) {
            self.spawn_status_fetch();
        }
    }

    /// Fetch off the message loop's thread. A tray that stalls while a socket
    /// times out is worse than a flyout that fills in a moment late.
    fn spawn_status_fetch(&mut self) {
        let port = self.supervisor.port();
        let result = Arc::clone(&self.status_result);
        let fetching = Arc::clone(&self.status_fetching);

        fetching.store(true, Ordering::Release);
        self.status_fetched_at = Some(Instant::now());
        thread::spawn(move || {
            let content = shared::fetch_popover_content(port);
            *result
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(content);
            // Released after the result is visible, so the timer never starts a
            // second fetch over an answer it has not drawn yet.
            fetching.store(false, Ordering::Release);
        });
    }

    fn show_menu(&mut self, window: HWND) -> Result<(), String> {
        if self.menu_open {
            return Ok(());
        }
        self.menu_open = true;
        let result = self.show_menu_inner(window);
        self.menu_open = false;
        result
    }

    fn show_menu_inner(&mut self, window: HWND) -> Result<(), String> {
        // The menu comes up where the flyout already is, so the flyout goes.
        self.hide_flyout(window);
        // SAFETY: CreatePopupMenu creates a menu owned by this function until DestroyMenu.
        let menu = unsafe { CreatePopupMenu() };
        if menu.is_null() {
            return Err(format!(
                "failed to create Scryer tray menu: {}",
                std::io::Error::last_os_error()
            ));
        }

        let result = (|| {
            append_menu(menu, MENU_OPEN, "Open Scryer", MF_STRING)?;
            append_menu(menu, MENU_OPEN_BROWSER, "Open in Browser", MF_STRING)?;
            append_menu(menu, MENU_START, "Start Scryer", MF_STRING)?;
            append_menu(menu, MENU_STOP, "Stop Scryer", MF_STRING)?;
            append_menu(menu, MENU_RESTART, "Restart Scryer", MF_STRING)?;
            // SAFETY: A separator does not use its string argument.
            unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null()) };
            append_menu(menu, MENU_OPEN_LOGS, "Open Logs", MF_STRING)?;
            let startup_flags = if startup_enabled()? {
                MF_STRING | MF_CHECKED
            } else {
                MF_STRING | MF_UNCHECKED
            };
            append_menu(menu, MENU_TOGGLE_STARTUP, "Start at sign-in", startup_flags)?;
            // SAFETY: A separator does not use its string argument.
            unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null()) };
            append_menu(menu, MENU_EXIT, "Exit", MF_STRING)?;

            let mut point = POINT::default();
            // SAFETY: `point` is writable storage for the system call.
            if unsafe { GetCursorPos(&mut point) } == 0 {
                return Err(format!(
                    "failed to get cursor position for Scryer tray menu: {}",
                    std::io::Error::last_os_error()
                ));
            }
            // SAFETY: The tray window is owned by this process and the menu remains live.
            unsafe { SetForegroundWindow(window) };
            // SAFETY: The menu and owner window remain valid through the call.
            let command = unsafe {
                TrackPopupMenu(
                    menu,
                    TPM_RETURNCMD | TPM_RIGHTBUTTON,
                    point.x,
                    point.y,
                    0,
                    window,
                    ptr::null(),
                )
            };
            self.handle_menu_command(window, command as u32)
        })();

        // SAFETY: This function owns the menu created above.
        unsafe { DestroyMenu(menu) };
        result
    }

    fn handle_menu_command(&mut self, window: HWND, command: u32) -> Result<(), String> {
        match command {
            0 => Ok(()),
            MENU_OPEN => self.open_scryer(window),
            MENU_OPEN_BROWSER => self.open_in_browser(),
            MENU_START => self.supervisor.start(),
            MENU_STOP => self.supervisor.stop(),
            MENU_RESTART => self.supervisor.restart(),
            MENU_OPEN_LOGS => open_target(&self.supervisor.logs_dir().to_string_lossy()),
            MENU_TOGGLE_STARTUP => {
                if startup_enabled()? {
                    unregister_startup()
                } else {
                    self.enable_startup()
                }
            }
            MENU_EXIT => {
                self.supervisor.ensure_restore_idle()?;
                // SAFETY: `window` is the live tray window for this state.
                unsafe { DestroyWindow(window) };
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn enable_startup(&self) -> Result<(), String> {
        let executable = std::env::current_exe()
            .map_err(|error| format!("failed to resolve scryer-tray.exe path: {error}"))?;
        register_startup(&executable)
    }

    /// Tear down everything the tray owns. The app window goes first so its
    /// WebView2 controller is closed while the apartment is still alive.
    unsafe fn shut_down(&mut self, window: HWND) {
        self.stop_status_polling(window);
        // Harmless if it was never set, which is the usual case: the icon
        // registers on the first try whenever Explorer is already up.
        // SAFETY: The window is live and owns this timer.
        unsafe { KillTimer(window, ICON_RETRY_TIMER) };
        if let Some(flyout) = self.flyout.take() {
            // SAFETY: The flyout belongs to this process and is not referenced
            // anywhere else once it is taken out of the state.
            unsafe { DestroyWindow(flyout) };
        }
        if let Some(app_window) = self.app_window.take() {
            // SAFETY: The window belongs to this process and is not referenced
            // anywhere else once it is taken out of the state.
            unsafe { DestroyWindow(app_window) };
        }
        // SAFETY: The icon belongs to this window and is being removed during teardown.
        unsafe { self.remove_icon(window) };
        let _ = self.supervisor.stop();
        // A server the tray adopted rather than started is still Scryer
        // running from this installation, and leaving it behind is what kept
        // the installed executables locked after "closing" Scryer.
        let _ = self.supervisor.stop_unowned_servers(EXIT_STOP_POLICY);
    }
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: The pointer was installed from a live Box immediately after window creation.
    let state = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) as *mut TrayState };
    if !state.is_null() {
        // SAFETY: The message loop serializes access to the state on this UI thread,
        // and nothing this function calls pumps messages, so no second borrow can
        // exist while this one is live.
        let state = unsafe { &mut *state };
        let result = match message {
            TRAY_CALLBACK_MESSAGE => tray_callback(state, window, wparam, lparam),
            OPEN_WINDOW_MESSAGE => state.open_scryer(window),
            RESTART_MESSAGE => state.restart_requested_by_server(),
            WEBVIEW_FAILED_MESSAGE => {
                state.webview_failed();
                Ok(())
            }
            WM_TIMER if wparam == FLYOUT_POLL_TIMER => {
                state.poll_status(window);
                Ok(())
            }
            WM_TIMER if wparam == ICON_RETRY_TIMER => {
                // SAFETY: This retries notification-area registration for the
                // live tray window.
                if unsafe { state.add_icon(window) }.is_ok() {
                    // SAFETY: The icon is registered, so no further retries are
                    // needed.
                    unsafe { KillTimer(window, ICON_RETRY_TIMER) };
                }
                Ok(())
            }
            SHUTDOWN_MESSAGE => {
                if state.supervisor.ensure_restore_idle().is_err() {
                    return 0;
                }
                // SAFETY: This is the live window associated with the tray state.
                unsafe { DestroyWindow(window) };
                Ok(())
            }
            // An application-controlled exit must not close the job handle
            // and terminate a restore worker before its safe handoff.
            WM_QUERYENDSESSION => {
                return isize::from(state.supervisor.ensure_restore_idle().is_ok());
            }
            // `wparam` is nonzero when the session really ends, or when
            // Restart Manager closes the tray (`ENDSESSION_CLOSEAPP` in
            // `lparam`). Either way the tray stops its server and exits now:
            // the process may be ended as soon as this returns.
            WM_ENDSESSION => {
                if wparam != 0 {
                    // SAFETY: This is the live window associated with the
                    // tray state; WM_DESTROY runs the teardown synchronously.
                    unsafe { DestroyWindow(window) };
                }
                return 0;
            }
            WM_DESTROY => {
                // SAFETY: The tray owns every handle released here.
                unsafe { state.shut_down(window) };
                // SAFETY: Ends the GetMessageW loop in this process.
                unsafe { PostQuitMessage(0) };
                return 0;
            }
            _ => {
                // SAFETY: Default processing is required for messages the tray does not own.
                return unsafe { DefWindowProcW(window, message, wparam, lparam) };
            }
        };
        if let Err(error) = result {
            show_error("Scryer", &error);
        }
        return 0;
    }

    // SAFETY: The window has not yet had its state attached, so default handling is correct.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

/// Decode one shell notification-icon callback.
///
/// Under `NOTIFYICON_VERSION_4` the event is the low word of `lparam` and the
/// icon id is the high word; under the legacy encoding the whole of `lparam` is
/// the event and the high word is zero, so masking to the low word reads both.
/// That matters because `NIM_SETVERSION` is allowed to fail.
///
/// Left-click arrives as `WM_LBUTTONUP` on the legacy encoding and as
/// `NIN_SELECT` on version 4, and right-click as `WM_RBUTTONUP` and
/// `WM_CONTEXTMENU` respectively — and a version-4 shell sends both members of
/// each pair per click. The legacy member is therefore only answered while the
/// icon is actually on the legacy encoding. Answering both would run each
/// action twice, and for the menu twice is not idempotent: `TrackPopupMenu`
/// pumps messages, so the second callback would re-enter the menu under the
/// first one.
fn tray_callback(
    state: &mut TrayState,
    window: HWND,
    wparam: WPARAM,
    lparam: LPARAM,
) -> Result<(), String> {
    match (lparam as u32) & 0xFFFF {
        WM_LBUTTONUP if state.version4 => Ok(()),
        WM_RBUTTONUP if state.version4 => Ok(()),
        WM_LBUTTONUP | NIN_SELECT | NIN_KEYSELECT => state.open_scryer(window),
        WM_RBUTTONUP | WM_CONTEXTMENU => state.show_menu(window),
        NIN_POPUPOPEN => {
            state.show_flyout(window, callback_anchor(wparam));
            Ok(())
        }
        NIN_POPUPCLOSE => {
            state.schedule_flyout_close();
            Ok(())
        }
        _ => Ok(()),
    }
}

/// The screen point version 4 packs into `wparam`. The coordinates are signed:
/// a monitor left of or above the primary one has negative ones.
fn callback_anchor(wparam: WPARAM) -> POINT {
    POINT {
        x: i32::from(wparam as u16 as i16),
        y: i32::from((wparam >> 16) as u16 as i16),
    }
}

// ---------------------------------------------------------------------------
// The app window
// ---------------------------------------------------------------------------

/// What the app window owns on behalf of WebView2.
///
/// The controller has to be reachable from the window procedure — it is what
/// resizes the browser when the window resizes — and it has to be released
/// before the window goes away, so the window itself owns it.
struct AppWindowState {
    controller: Option<ICoreWebView2Controller>,
}

/// Create the app window, hidden and centred on the primary display.
///
/// It stays hidden until WebView2 has something to show in it: a visible empty
/// frame while the browser starts looks like a broken app.
fn create_app_window(width: i32, height: i32, title: &str) -> Result<HWND, String> {
    let class_name = wide(APP_WINDOW_CLASS);
    let title = wide(title);
    // The process is per-monitor-DPI-aware, so both the metrics below and the
    // window size are physical pixels. Scaling by the system DPI keeps the
    // logical default; WM_DPICHANGED re-fits the window if it then moves to a
    // monitor with a different density.
    // SAFETY: GetDpiForSystem has no failure mode.
    let dpi = unsafe { GetDpiForSystem() } as i32;
    let width = width * dpi / 96;
    let height = height * dpi / 96;
    // SAFETY: Both metrics are documented to be available on every display
    // configuration; a headless session reports zero, which centres at 0,0.
    let (screen_width, screen_height) =
        unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
    // A display smaller than the scaled default still gets a window it can
    // show whole.
    let width = if screen_width > 0 {
        width.min(screen_width)
    } else {
        width
    };
    let height = if screen_height > 0 {
        height.min(screen_height)
    } else {
        height
    };
    let x = ((screen_width - width) / 2).max(0);
    let y = ((screen_height - height) / 2).max(0);

    let state = Box::into_raw(Box::new(AppWindowState { controller: None }));
    // SAFETY: The class is registered, and the state pointer is handed to the
    // window procedure through WM_CREATE, which installs it as the window's
    // user data and takes ownership of it from there.
    let window = unsafe {
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPEDWINDOW,
            x,
            y,
            width,
            height,
            ptr::null_mut(),
            ptr::null_mut(),
            GetModuleHandleW(ptr::null()),
            state.cast::<c_void>(),
        )
    };
    if window.is_null() {
        // SAFETY: WM_CREATE never ran, so this function still owns the Box.
        unsafe { drop(Box::from_raw(state)) };
        return Err(format!(
            "failed to create the Scryer app window: {}",
            std::io::Error::last_os_error()
        ));
    }
    apply_window_chrome(window);
    Ok(window)
}

/// Match the window chrome to the web UI: the caption bar in Scryer's
/// background color, caption buttons in the matching light or dark style, and
/// rounded corners while the window is not maximized (DWM squares them itself
/// on maximize). Every attribute is best-effort — Windows 10's DWM rejects
/// the newer ones, and the stock chrome is an acceptable fallback.
fn apply_window_chrome(window: HWND) {
    let dark = !apps_use_light_theme();
    let dark_mode: i32 = i32::from(dark);
    let (background, text) = if dark {
        (CAPTION_DARK_BACKGROUND, CAPTION_DARK_TEXT)
    } else {
        (CAPTION_LIGHT_BACKGROUND, CAPTION_LIGHT_TEXT)
    };
    let corners = DWMWCP_ROUND;
    // SAFETY: Every call passes a pointer to a live local together with that
    // local's exact size; DWM copies the value before returning.
    unsafe {
        DwmSetWindowAttribute(
            window,
            DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
            (&dark_mode as *const i32).cast(),
            std::mem::size_of::<i32>() as u32,
        );
        DwmSetWindowAttribute(
            window,
            DWMWA_CAPTION_COLOR as u32,
            (&background as *const u32).cast(),
            std::mem::size_of::<u32>() as u32,
        );
        DwmSetWindowAttribute(
            window,
            DWMWA_TEXT_COLOR as u32,
            (&text as *const u32).cast(),
            std::mem::size_of::<u32>() as u32,
        );
        DwmSetWindowAttribute(
            window,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            (&corners as *const i32).cast(),
            std::mem::size_of::<i32>() as u32,
        );
    }
}

/// Whether Windows is set to light app mode. The web UI follows the same
/// switch through `prefers-color-scheme`, so the chrome tracking it keeps the
/// caption and the page in one theme. A missing value means light, matching
/// Windows' own default.
fn apps_use_light_theme() -> bool {
    let mut key: HKEY = ptr::null_mut();
    let key_path = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
    // SAFETY: The registry path and output key pointer are valid for the call.
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            key_path.as_ptr(),
            0,
            KEY_QUERY_VALUE,
            &mut key,
        )
    };
    if status != 0 {
        return true;
    }
    let value_name = wide("AppsUseLightTheme");
    let mut value: u32 = 1;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: The key is open, and the buffer and size describe the same
    // four-byte local.
    let status = unsafe {
        RegQueryValueExW(
            key,
            value_name.as_ptr(),
            ptr::null(),
            ptr::null_mut(),
            (&mut value as *mut u32).cast(),
            &mut size,
        )
    };
    // SAFETY: This function owns the registry handle returned above.
    unsafe { RegCloseKey(key) };
    status != 0 || value != 0
}

/// Whether Ctrl is held right now. The high bit of the key state is the one
/// that means "down"; the low bit is the toggle state, which Ctrl does not have.
fn control_is_down() -> bool {
    // SAFETY: GetKeyState has no failure mode.
    (unsafe { GetKeyState(i32::from(VK_CONTROL)) } as u16) & 0x8000 != 0
}

fn show_and_focus(window: HWND) {
    // SAFETY: The window belongs to this process and is still alive; both
    // calls are no-ops on a window that is already visible and focused.
    unsafe {
        ShowWindow(window, SW_SHOW);
        SetForegroundWindow(window);
    }
}

/// Ask WebView2 for an environment and a controller for `window`.
///
/// Both steps are asynchronous. The completion handlers run from the tray's
/// own message loop, so they can safely reach back into the window's state —
/// but not into the tray's, which is why a failure is reported by posting a
/// message rather than by touching `TrayState` from here.
fn start_webview(
    tray_window: HWND,
    window: HWND,
    user_data_folder: &Path,
    url: String,
    origin: String,
) -> Result<(), String> {
    let user_data_folder = wide(&user_data_folder.to_string_lossy());
    let handler = CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(
        move |result, environment| {
            let environment = match (result, environment) {
                (Ok(()), Some(environment)) => environment,
                (Err(error), _) => {
                    report_webview_failure(tray_window, &format!("WebView2 failed: {error}"));
                    return Ok(());
                }
                (Ok(()), None) => {
                    report_webview_failure(tray_window, "WebView2 returned no environment");
                    return Ok(());
                }
            };

            let controller_handler = CreateCoreWebView2ControllerCompletedHandler::create(
                Box::new(move |result, controller| {
                    let controller = match (result, controller) {
                        (Ok(()), Some(controller)) => controller,
                        (Err(error), _) => {
                            report_webview_failure(
                                tray_window,
                                &format!("WebView2 failed: {error}"),
                            );
                            return Ok(());
                        }
                        (Ok(()), None) => {
                            report_webview_failure(tray_window, "WebView2 returned no controller");
                            return Ok(());
                        }
                    };
                    if let Err(error) = attach_controller(window, &controller, &url, &origin) {
                        report_webview_failure(tray_window, &format!("WebView2 failed: {error}"));
                    }
                    Ok(())
                }),
            );

            // SAFETY: `window` is a live top-level window owned by this thread.
            if let Err(error) = unsafe {
                environment.CreateCoreWebView2Controller(to_com_hwnd(window), &controller_handler)
            } {
                report_webview_failure(tray_window, &format!("WebView2 failed: {error}"));
            }
            Ok(())
        },
    ));

    // SAFETY: Both wide strings live until the call returns, and the handler is
    // reference counted by WebView2 for as long as it needs it.
    unsafe {
        CreateCoreWebView2EnvironmentWithOptions(
            PCWSTR::null(),
            PCWSTR(user_data_folder.as_ptr()),
            None::<&ICoreWebView2EnvironmentOptions>,
            &handler,
        )
    }
    .map_err(|error| format!("the WebView2 runtime could not be started: {error}"))
}

/// Wire a freshly created controller into the app window and navigate it.
fn attach_controller(
    window: HWND,
    controller: &ICoreWebView2Controller,
    url: &str,
    origin: &str,
) -> windows::core::Result<()> {
    // SAFETY: The pointer was installed by WM_CREATE and is cleared by
    // WM_DESTROY, so a null here means the window is already gone.
    let state = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) as *mut AppWindowState };
    if state.is_null() {
        // SAFETY: Nothing else has a reference to this controller yet.
        unsafe { controller.Close() }?;
        return Ok(());
    }

    // SAFETY: The controller was just created and belongs to this window.
    unsafe {
        controller.SetBounds(client_rect(window))?;
        controller.SetIsVisible(true)?;
    }
    // SAFETY: As above.
    let webview = unsafe { controller.CoreWebView2() }?;

    let external_origin = origin.to_string();
    let navigation_handler =
        NavigationStartingEventHandler::create(Box::new(move |_webview, arguments| {
            let Some(arguments) = arguments else {
                return Ok(());
            };
            // SAFETY: The URI is owned by the event arguments until it is read
            // out into COM task memory, which CoTaskMemPWSTR then frees.
            let uri = unsafe {
                let mut uri = PWSTR::null();
                arguments.Uri(&mut uri)?;
                CoTaskMemPWSTR::from(uri).to_string()
            };
            if shared::opens_in_external_browser(&external_origin, &uri) {
                // SAFETY: Cancelling is only valid from inside this event.
                unsafe { arguments.SetCancel(true) }?;
                let _ = open_target(&uri);
            }
            Ok(())
        }));
    let new_window_handler =
        NewWindowRequestedEventHandler::create(Box::new(move |_webview, arguments| {
            let Some(arguments) = arguments else {
                return Ok(());
            };
            // SAFETY: As above.
            let uri = unsafe {
                let mut uri = PWSTR::null();
                arguments.Uri(&mut uri)?;
                CoTaskMemPWSTR::from(uri).to_string()
            };
            // Every `target="_blank"` leaves the app: the app window is one
            // window, and a second frameless WebView2 with no chrome would be
            // a worse browser than the user's own.
            // SAFETY: Marking the request handled is only valid from inside
            // this event.
            unsafe { arguments.SetHandled(true) }?;
            let _ = open_target(&uri);
            Ok(())
        }));

    // Ctrl+W hides the window, the same as its close button. WebView2 offers
    // the host every accelerator before the page sees it; without this the page
    // swallows the keystroke and the shortcut works only while the window
    // itself has focus.
    let accelerator_handler =
        AcceleratorKeyPressedEventHandler::create(Box::new(move |_controller, arguments| {
            let Some(arguments) = arguments else {
                return Ok(());
            };
            // SAFETY: Both values are read out of the event arguments while the
            // event is being delivered.
            let (kind, key) = unsafe {
                let mut kind = COREWEBVIEW2_KEY_EVENT_KIND::default();
                arguments.KeyEventKind(&mut kind)?;
                let mut key = 0u32;
                arguments.VirtualKey(&mut key)?;
                (kind, key)
            };
            let pressed = kind == COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN
                || kind == COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN;
            if pressed && key == HIDE_WINDOW_VIRTUAL_KEY && control_is_down() {
                // SAFETY: Marking the key handled is only valid from inside
                // this event.
                unsafe { arguments.SetHandled(true) }?;
                // The window hides itself from WM_CLOSE, so the button, Alt+F4
                // and this all end in the same place.
                // SAFETY: The window is owned by this thread, which is the one
                // this callback is delivered on.
                unsafe { PostMessageW(window, WM_CLOSE, 0, 0) };
            }
            Ok(())
        }));

    // SAFETY: The webview and the controller outlive the handlers they
    // reference count.
    unsafe {
        let mut token = 0;
        webview.add_NavigationStarting(&navigation_handler, &mut token)?;
        let mut token = 0;
        webview.add_NewWindowRequested(&new_window_handler, &mut token)?;
        let mut token = 0;
        controller.add_AcceleratorKeyPressed(&accelerator_handler, &mut token)?;
        webview.Navigate(&windows::core::HSTRING::from(url))?;
    }

    // SAFETY: The state pointer is valid and this thread is the only one that
    // reaches it.
    unsafe { (*state).controller = Some(controller.clone()) };
    show_and_focus(window);
    Ok(())
}

fn report_webview_failure(tray_window: HWND, message: &str) {
    eprintln!("Scryer: {message}");
    // SAFETY: The tray window is owned by this thread and outlives every
    // WebView2 callback, which are all delivered to this thread's message loop.
    unsafe { PostMessageW(tray_window, WEBVIEW_FAILED_MESSAGE, 0, 0) };
}

fn client_rect(window: HWND) -> ComRect {
    let mut rect = RECT::default();
    // SAFETY: `rect` is writable storage and the window belongs to this process.
    unsafe { GetClientRect(window, &mut rect) };
    ComRect {
        left: 0,
        top: 0,
        right: rect.right,
        bottom: rect.bottom,
    }
}

fn to_com_hwnd(window: HWND) -> ComHwnd {
    ComHwnd(window.cast::<c_void>())
}

unsafe extern "system" fn app_window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_CREATE {
        // SAFETY: `lparam` is the CREATESTRUCTW the system built for this
        // window, and its creation parameter is the Box leaked in
        // `create_app_window`.
        unsafe {
            let create = &*(lparam as *const CREATESTRUCTW);
            SetWindowLongPtrW(window, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        // SAFETY: Default processing completes window creation.
        return unsafe { DefWindowProcW(window, message, wparam, lparam) };
    }

    // SAFETY: The pointer was installed by WM_CREATE above.
    let state = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) as *mut AppWindowState };
    if state.is_null() {
        // SAFETY: Messages before WM_CREATE and after WM_DESTROY get default handling.
        return unsafe { DefWindowProcW(window, message, wparam, lparam) };
    }

    match message {
        WM_SIZE => {
            // SAFETY: The state pointer is valid and this is the only thread
            // that reads it.
            if let Some(controller) = unsafe { (*state).controller.as_ref() } {
                // SAFETY: The controller belongs to this window.
                let _ = unsafe { controller.SetBounds(client_rect(window)) };
            }
            0
        }
        WM_DPICHANGED => {
            // The system hands us the frame the window should occupy on the
            // display it moved to; anything else leaves the window the wrong
            // physical size on a mixed-DPI desktop.
            // SAFETY: On WM_DPICHANGED `lparam` points at a RECT owned by the
            // system for the duration of the message.
            unsafe {
                let suggested = &*(lparam as *const RECT);
                SetWindowPos(
                    window,
                    ptr::null_mut(),
                    suggested.left,
                    suggested.top,
                    suggested.right - suggested.left,
                    suggested.bottom - suggested.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            0
        }
        // Ctrl+W with the page focused is answered by the webview's own
        // accelerator handler; this is the same shortcut for every other focus
        // the window can have, and both end in WM_CLOSE.
        WM_KEYDOWN if wparam as u32 == HIDE_WINDOW_VIRTUAL_KEY && control_is_down() => {
            // SAFETY: The window is owned by this thread.
            unsafe { PostMessageW(window, WM_CLOSE, 0, 0) };
            0
        }
        WM_CLOSE => {
            // The tray keeps running, so closing the window hides it. The
            // browser process, the session and the scroll position all
            // survive, which is what makes reopening instant.
            // SAFETY: The window belongs to this process.
            unsafe { ShowWindow(window, SW_HIDE) };
            0
        }
        WM_DESTROY => {
            // SAFETY: The Box was leaked in `create_app_window` and installed
            // by WM_CREATE; this is the only place that takes it back.
            unsafe {
                SetWindowLongPtrW(window, GWLP_USERDATA, 0);
                let state = Box::from_raw(state);
                if let Some(controller) = state.controller.as_ref() {
                    let _ = controller.Close();
                }
            }
            0
        }
        _ => {
            // SAFETY: Default processing is required for everything else.
            unsafe { DefWindowProcW(window, message, wparam, lparam) }
        }
    }
}

// ---------------------------------------------------------------------------
// The hover flyout
// ---------------------------------------------------------------------------

/// What the flyout window owns: the snapshot it draws, and the fonts and
/// measurements it draws it with.
///
/// The tray writes the snapshot in and then invalidates the window; it never
/// holds a reference to this across a call that could reach the window
/// procedure, which is what keeps the two from aliasing it.
struct FlyoutState {
    content: PopoverContent,
    fonts: FlyoutFonts,
    metrics: FlyoutMetrics,
}

struct FlyoutFonts {
    status: HFONT,
    name: HFONT,
    detail: HFONT,
}

/// Every distance the flyout is laid out with, in physical pixels for the
/// display it was created on.
#[derive(Clone, Copy)]
struct FlyoutMetrics {
    width: i32,
    padding: i32,
    block_gap: i32,
    row_gap: i32,
    bar: i32,
    icon_gap: i32,
    status_line: i32,
    name_line: i32,
    detail_line: i32,
}

/// One drawn element. Rows past the end of the queue produce no block at all,
/// which is what takes them out of the height as well as out of the paint.
enum FlyoutBlock<'a> {
    Status(&'a str),
    Message(&'a str),
    Row(&'a PopoverRow),
}

impl FlyoutBlock<'_> {
    fn height(&self, metrics: &FlyoutMetrics) -> i32 {
        match self {
            Self::Status(_) => metrics.status_line,
            Self::Message(_) => metrics.detail_line,
            Self::Row(_) => {
                metrics.name_line
                    + metrics.row_gap
                    + metrics.bar
                    + metrics.row_gap
                    + metrics.detail_line
            }
        }
    }
}

fn flyout_blocks(content: &PopoverContent) -> Vec<FlyoutBlock<'_>> {
    let mut blocks = Vec::new();
    if let Some(status) = content.status.as_deref() {
        blocks.push(FlyoutBlock::Status(status));
    }
    if let Some(message) = content.message.as_deref() {
        blocks.push(FlyoutBlock::Message(message));
    }
    blocks.extend(
        content
            .rows
            .iter()
            .take(shared::POPOVER_ROWS)
            .map(FlyoutBlock::Row),
    );
    blocks
}

fn flyout_height(content: &PopoverContent, metrics: &FlyoutMetrics) -> i32 {
    let blocks = flyout_blocks(content);
    let blocks_height: i32 = blocks.iter().map(|block| block.height(metrics)).sum();
    let gaps = metrics.block_gap * (blocks.len().saturating_sub(1)) as i32;
    metrics.padding * 2 + blocks_height + gaps
}

/// Create the flyout, hidden, sized to nothing. Every later show sizes it to
/// the snapshot it is about to draw.
fn create_flyout_window() -> Result<HWND, String> {
    let class_name = wide(FLYOUT_WINDOW_CLASS);
    // The process is per-monitor-DPI-aware, so every metric below is in
    // physical pixels and has to be scaled. The system DPI is the one
    // `create_app_window` scales by; the notification area lives on the primary
    // display, which is the display that DPI describes.
    // SAFETY: GetDpiForSystem has no failure mode.
    let dpi = unsafe { GetDpiForSystem() } as i32;
    let (fonts, metrics) = flyout_metrics(dpi);
    let state = Box::into_raw(Box::new(FlyoutState {
        content: PopoverContent {
            status: None,
            rows: Vec::new(),
            message: Some(FLYOUT_PLACEHOLDER.to_string()),
        },
        fonts,
        metrics,
    }));

    // `WS_EX_NOACTIVATE` is what makes this a surface rather than a window: it
    // never takes focus, so the window the user was typing in keeps it, and the
    // notification icon does not lose its own hover state to it.
    // SAFETY: The class is registered, and the state pointer is handed to the
    // window procedure through WM_CREATE, which takes ownership of it.
    let window = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            class_name.as_ptr(),
            ptr::null(),
            WS_POPUP,
            0,
            0,
            metrics.width,
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            GetModuleHandleW(ptr::null()),
            state.cast::<c_void>(),
        )
    };
    if window.is_null() {
        // SAFETY: WM_CREATE never ran, so this function still owns the Box.
        unsafe { drop(Box::from_raw(state)) };
        return Err(format!(
            "failed to create the Scryer flyout window: {}",
            std::io::Error::last_os_error()
        ));
    }

    // Rounded corners, so the flyout reads as one of Windows 11's own surfaces.
    // Best-effort, exactly like the app window's chrome: Windows 10's DWM
    // rejects the attribute and leaves square corners, which is acceptable.
    let corners = DWMWCP_ROUND;
    // SAFETY: The pointer and size describe the same live local, which DWM
    // copies before returning.
    unsafe {
        DwmSetWindowAttribute(
            window,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            (&corners as *const i32).cast(),
            std::mem::size_of::<i32>() as u32,
        );
    }
    Ok(window)
}

/// The fonts and distances the flyout draws with on a display of `dpi`.
///
/// `SystemParametersInfoForDpi` returns the shell's own message font already
/// scaled for that density, which is what makes the flyout read as part of
/// Windows rather than as an application's idea of a tooltip. The bold status
/// line and the dimmer detail line are that font at the macOS popover's
/// weights and relative sizes.
fn flyout_metrics(dpi: i32) -> (FlyoutFonts, FlyoutMetrics) {
    let mut non_client = NONCLIENTMETRICSW {
        cbSize: std::mem::size_of::<NONCLIENTMETRICSW>() as u32,
        ..Default::default()
    };
    // SAFETY: The pointer and `cbSize` describe the same live local.
    let described = unsafe {
        SystemParametersInfoForDpi(
            SPI_GETNONCLIENTMETRICS,
            non_client.cbSize,
            (&mut non_client as *mut NONCLIENTMETRICSW).cast::<c_void>(),
            0,
            dpi as u32,
        )
    };
    let base = if described == 0 {
        // A shell that will not describe itself still gets a flyout, in the
        // default UI font at the size the message font usually is.
        LOGFONTW {
            lfHeight: -(12 * dpi / 96),
            ..Default::default()
        }
    } else {
        non_client.lfMessageFont
    };

    let mut bold = base;
    bold.lfWeight = FW_BOLD as i32;
    let mut small = base;
    // The macOS popover's 11-point detail line against its 13-point title.
    small.lfHeight = base.lfHeight * 11 / 13;

    // SAFETY: Each LOGFONTW is a live local for the duration of its call, and
    // the returned fonts are owned by the flyout until WM_DESTROY.
    let fonts = unsafe {
        FlyoutFonts {
            status: CreateFontIndirectW(&bold),
            name: CreateFontIndirectW(&base),
            detail: CreateFontIndirectW(&small),
        }
    };

    // A font that could not be created measures nothing, so the line heights
    // fall back to the nominal cell height for the requested size.
    let nominal = |height: i32| (height.abs() * 4 / 3).max(1);
    let metrics = FlyoutMetrics {
        width: scale(FLYOUT_WIDTH, dpi),
        padding: scale(FLYOUT_PADDING, dpi),
        block_gap: scale(FLYOUT_BLOCK_GAP, dpi),
        row_gap: scale(FLYOUT_ROW_GAP, dpi),
        bar: scale(FLYOUT_BAR_HEIGHT, dpi),
        icon_gap: scale(FLYOUT_ICON_GAP, dpi),
        status_line: line_height(fonts.status, nominal(bold.lfHeight)),
        name_line: line_height(fonts.name, nominal(base.lfHeight)),
        detail_line: line_height(fonts.detail, nominal(small.lfHeight)),
    };
    (fonts, metrics)
}

fn scale(value: i32, dpi: i32) -> i32 {
    value * dpi / 96
}

/// How tall one line of `font` is, measured in a device context of its own.
fn line_height(font: HFONT, fallback: i32) -> i32 {
    if font.is_null() {
        return fallback;
    }
    // SAFETY: The memory device context is created and released here, and the
    // font is selected out of it before it is.
    let measured = unsafe {
        let device = CreateCompatibleDC(ptr::null_mut());
        if device.is_null() {
            return fallback;
        }
        let previous = SelectObject(device, font);
        let mut text = TEXTMETRICW::default();
        let described = GetTextMetricsW(device, &mut text);
        SelectObject(device, previous);
        DeleteDC(device);
        if described == 0 { 0 } else { text.tmHeight }
    };
    if measured > 0 { measured } else { fallback }
}

/// The size the flyout's current snapshot needs.
///
/// The borrow of the window's state ends before the caller moves or repaints
/// the window, so nothing the window procedure does can alias it.
fn flyout_size(flyout: HWND) -> Option<(i32, i32)> {
    // SAFETY: The pointer was installed by WM_CREATE and cleared by WM_DESTROY,
    // so a null here means the window is already gone.
    let state = unsafe { GetWindowLongPtrW(flyout, GWLP_USERDATA) as *const FlyoutState };
    if state.is_null() {
        return None;
    }
    // SAFETY: As above, and this thread is the only one that reaches it.
    let state = unsafe { &*state };
    Some((
        state.metrics.width,
        flyout_height(&state.content, &state.metrics),
    ))
}

fn set_flyout_content(flyout: HWND, content: PopoverContent) {
    // SAFETY: As in `flyout_size`.
    let state = unsafe { GetWindowLongPtrW(flyout, GWLP_USERDATA) as *mut FlyoutState };
    if state.is_null() {
        return;
    }
    // SAFETY: As above. The write is over before anything can repaint.
    unsafe { (*state).content = content };
}

/// Put the flyout beside the notification icon, inside the work area of the
/// monitor the pointer is on.
///
/// Above the icon when there is room — the taskbar is at the bottom of nearly
/// every desktop — and below it otherwise, which is what a taskbar docked to
/// the top needs. `Shell_NotifyIconGetRect` knows where the icon actually is,
/// including inside the overflow flyout; the callback's own anchor point is the
/// fallback for the shells that will not say.
fn position_flyout(flyout: HWND, anchor: POINT, icon: Option<RECT>, width: i32, height: i32) {
    let gap = icon_gap(flyout);
    let (center, above, below) = match icon {
        Some(rect) => ((rect.left + rect.right) / 2, rect.top, rect.bottom),
        None => (anchor.x, anchor.y, anchor.y),
    };
    let work = work_area(anchor);
    let x = (center - width / 2).clamp(work.left, (work.right - width).max(work.left));
    let above_y = above - gap - height;
    let y = if above_y >= work.top {
        above_y
    } else {
        (below + gap).min((work.bottom - height).max(work.top))
    };
    // SAFETY: The flyout is a live window owned by this thread, and
    // `SWP_NOACTIVATE` keeps the focused window focused.
    unsafe { SetWindowPos(flyout, HWND_TOPMOST, x, y, width, height, SWP_NOACTIVATE) };
}

fn icon_gap(flyout: HWND) -> i32 {
    // SAFETY: As in `flyout_size`.
    let state = unsafe { GetWindowLongPtrW(flyout, GWLP_USERDATA) as *const FlyoutState };
    if state.is_null() {
        return FLYOUT_ICON_GAP;
    }
    // SAFETY: As above.
    unsafe { (*state).metrics.icon_gap }
}

fn work_area(point: POINT) -> RECT {
    // SAFETY: `MONITOR_DEFAULTTONEAREST` never returns null for a live desktop,
    // and the output structure describes its own size.
    let described = unsafe {
        let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        (!monitor.is_null() && GetMonitorInfoW(monitor, &mut info) != 0).then_some(info.rcWork)
    };
    described.unwrap_or_else(|| {
        // SAFETY: Both metrics are available on every display configuration.
        let (width, height) =
            unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
        RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        }
    })
}

/// Where the shell is currently drawing this process's notification icon.
fn tray_icon_rect(tray_window: HWND) -> Option<RECT> {
    let identifier = NOTIFYICONIDENTIFIER {
        cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
        hWnd: tray_window,
        uID: TRAY_ICON_ID,
        ..Default::default()
    };
    let mut rect = RECT::default();
    // SAFETY: Both structures are live locals for the duration of the call.
    let status = unsafe { Shell_NotifyIconGetRect(&identifier, &mut rect) };
    (status == 0).then_some(rect)
}

/// The flyout's colors, from the same caption tokens as the app window's
/// chrome, so the two surfaces are one theme.
struct FlyoutTheme {
    background: u32,
    text: u32,
    dim: u32,
    bar_track: u32,
    bar_fill: u32,
}

impl FlyoutTheme {
    fn current() -> Self {
        let (background, text) = if apps_use_light_theme() {
            (CAPTION_LIGHT_BACKGROUND, CAPTION_LIGHT_TEXT)
        } else {
            (CAPTION_DARK_BACKGROUND, CAPTION_DARK_TEXT)
        };
        // The bar is the text color at two intensities rather than an accent of
        // its own: five of them stacked in a 300-pixel surface is a lot of
        // color for a glance.
        Self {
            background,
            text,
            dim: blend(text, background, 55),
            bar_track: blend(text, background, 18),
            bar_fill: blend(text, background, 80),
        }
    }
}

/// Mix two colors channel by channel; `weight` is the percentage of
/// `foreground` in the result. COLORREF byte order is 0x00BBGGRR, so the
/// channels are taken and put back in that same order.
fn blend(foreground: u32, background: u32, weight: u32) -> u32 {
    let weight = weight.min(100);
    let mut mixed = 0u32;
    for shift in [0u32, 8, 16] {
        let front = (foreground >> shift) & 0xFF;
        let back = (background >> shift) & 0xFF;
        mixed |= ((front * weight + back * (100 - weight)) / 100) << shift;
    }
    mixed
}

unsafe extern "system" fn flyout_window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_CREATE {
        // SAFETY: `lparam` is the CREATESTRUCTW the system built for this
        // window, and its creation parameter is the Box leaked in
        // `create_flyout_window`.
        unsafe {
            let create = &*(lparam as *const CREATESTRUCTW);
            SetWindowLongPtrW(window, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        // SAFETY: Default processing completes window creation.
        return unsafe { DefWindowProcW(window, message, wparam, lparam) };
    }

    // SAFETY: The pointer was installed by WM_CREATE above.
    let state = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) as *mut FlyoutState };
    if state.is_null() {
        // SAFETY: Messages before WM_CREATE and after WM_DESTROY get default handling.
        return unsafe { DefWindowProcW(window, message, wparam, lparam) };
    }

    match message {
        // Every pixel is painted below, so erasing first would only flash the
        // background between the two.
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            // SAFETY: The state pointer is valid, this thread is the only one
            // that reaches it, and painting pumps no messages, so nothing can
            // take a second reference while this one is live.
            unsafe { paint_flyout(window, &*state) };
            0
        }
        // The pointer is inside the flyout, so the close the icon asked for is
        // cancelled — and the flyout has to ask to be told when the pointer
        // leaves again, which only TrackMouseEvent arranges.
        WM_MOUSEMOVE => {
            let mut track = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: window,
                dwHoverTime: 0,
            };
            // SAFETY: The window is live and `track` is writable storage for
            // the call.
            unsafe {
                KillTimer(window, FLYOUT_CLOSE_TIMER);
                TrackMouseEvent(&mut track);
            }
            0
        }
        WM_MOUSELEAVE => {
            // SAFETY: The window is live and the timer id is private to it.
            unsafe { SetTimer(window, FLYOUT_CLOSE_TIMER, FLYOUT_CLOSE_GRACE_MS, None) };
            0
        }
        WM_TIMER if wparam == FLYOUT_CLOSE_TIMER => {
            // Hidden, not destroyed: the next hover reuses the window and the
            // fonts it already measured. The tray's own poll timer notices the
            // window is gone from the screen and stops asking the server.
            // SAFETY: The window is live and owns this timer.
            unsafe {
                KillTimer(window, FLYOUT_CLOSE_TIMER);
                ShowWindow(window, SW_HIDE);
            }
            0
        }
        WM_DESTROY => {
            // SAFETY: The Box was leaked in `create_flyout_window` and
            // installed by WM_CREATE; this is the only place that takes it
            // back, and the fonts belong to it.
            unsafe {
                SetWindowLongPtrW(window, GWLP_USERDATA, 0);
                KillTimer(window, FLYOUT_CLOSE_TIMER);
                let state = Box::from_raw(state);
                DeleteObject(state.fonts.status);
                DeleteObject(state.fonts.name);
                DeleteObject(state.fonts.detail);
            }
            0
        }
        _ => {
            // SAFETY: Default processing is required for everything else.
            unsafe { DefWindowProcW(window, message, wparam, lparam) }
        }
    }
}

/// Paint the flyout through a bitmap of its own.
///
/// The flyout repaints under the pointer every time a fetch answers, and a
/// surface drawn straight onto the screen flashes its background once per
/// repaint.
unsafe fn paint_flyout(window: HWND, state: &FlyoutState) {
    let mut paint = PAINTSTRUCT::default();
    // SAFETY: `paint` is writable storage and the window is live.
    let screen = unsafe { BeginPaint(window, &mut paint) };
    if screen.is_null() {
        return;
    }
    let mut client = RECT::default();
    // SAFETY: `client` is writable storage and the window belongs to this process.
    unsafe { GetClientRect(window, &mut client) };
    let (width, height) = (client.right, client.bottom);

    // SAFETY: Every object created here is selected out and deleted before the
    // function returns.
    unsafe {
        let buffer = CreateCompatibleDC(screen);
        let bitmap = if buffer.is_null() {
            ptr::null_mut()
        } else {
            CreateCompatibleBitmap(screen, width, height)
        };
        if bitmap.is_null() {
            // No buffer to draw into: a flicker is better than a blank flyout.
            if !buffer.is_null() {
                DeleteDC(buffer);
            }
            draw_flyout(screen, width, height, state);
        } else {
            let previous = SelectObject(buffer, bitmap);
            draw_flyout(buffer, width, height, state);
            BitBlt(screen, 0, 0, width, height, buffer, 0, 0, SRCCOPY);
            SelectObject(buffer, previous);
            DeleteObject(bitmap);
            DeleteDC(buffer);
        }
        EndPaint(window, &paint);
    }
}

fn draw_flyout(device: HDC, width: i32, height: i32, state: &FlyoutState) {
    let theme = FlyoutTheme::current();
    let full = RECT {
        left: 0,
        top: 0,
        right: width,
        bottom: height,
    };
    // SAFETY: The brush is created and deleted here, and the rectangle is a
    // live local for the call.
    unsafe {
        let brush = CreateSolidBrush(theme.background);
        FillRect(device, &full, brush);
        DeleteObject(brush);
        SetBkMode(device, TRANSPARENT as i32);
    }

    let metrics = &state.metrics;
    let left = metrics.padding;
    let right = width - metrics.padding;
    let mut top = metrics.padding;
    for block in flyout_blocks(&state.content) {
        let block_height = block.height(metrics);
        match block {
            FlyoutBlock::Status(text) => draw_line(
                device,
                state.fonts.status,
                theme.text,
                RECT {
                    left,
                    top,
                    right,
                    bottom: top + metrics.status_line,
                },
                text,
            ),
            FlyoutBlock::Message(text) => draw_line(
                device,
                state.fonts.detail,
                theme.dim,
                RECT {
                    left,
                    top,
                    right,
                    bottom: top + metrics.detail_line,
                },
                text,
            ),
            FlyoutBlock::Row(row) => {
                draw_line(
                    device,
                    state.fonts.name,
                    theme.text,
                    RECT {
                        left,
                        top,
                        right,
                        bottom: top + metrics.name_line,
                    },
                    &row.name,
                );
                let bar_top = top + metrics.name_line + metrics.row_gap;
                draw_progress(
                    device,
                    RECT {
                        left,
                        top: bar_top,
                        right,
                        bottom: bar_top + metrics.bar,
                    },
                    row.progress_percent,
                    &theme,
                );
                let detail_top = bar_top + metrics.bar + metrics.row_gap;
                draw_line(
                    device,
                    state.fonts.detail,
                    theme.dim,
                    RECT {
                        left,
                        top: detail_top,
                        right,
                        bottom: detail_top + metrics.detail_line,
                    },
                    &shared::row_detail(row),
                );
            }
        }
        top += block_height + metrics.block_gap;
    }
}

/// One line of text, clipped to its rectangle with an ellipsis. Queue titles
/// are longer than any flyout, so this is what keeps a release name from
/// deciding the layout.
fn draw_line(device: HDC, font: HFONT, color: u32, mut bounds: RECT, text: &str) {
    let text = wide(text);
    // SAFETY: The font belongs to the flyout, the text is nul-terminated and
    // both it and the rectangle outlive the call.
    unsafe {
        let previous = SelectObject(device, font);
        SetTextColor(device, color);
        DrawTextW(
            device,
            text.as_ptr(),
            -1,
            &mut bounds,
            DT_LEFT | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
        );
        SelectObject(device, previous);
    }
}

fn draw_progress(device: HDC, bounds: RECT, percent: f64, theme: &FlyoutTheme) {
    let width = bounds.right - bounds.left;
    let filled = if percent.is_finite() {
        ((f64::from(width) * percent.clamp(0.0, 100.0)) / 100.0) as i32
    } else {
        0
    };
    // SAFETY: Each brush is created and deleted here, and each rectangle is a
    // live local for its call.
    unsafe {
        let track = CreateSolidBrush(theme.bar_track);
        FillRect(device, &bounds, track);
        DeleteObject(track);
        if filled > 0 {
            let fill = RECT {
                right: bounds.left + filled.min(width),
                ..bounds
            };
            let brush = CreateSolidBrush(theme.bar_fill);
            FillRect(device, &fill, brush);
            DeleteObject(brush);
        }
    }
}

// ---------------------------------------------------------------------------
// `--webview-smoke`
// ---------------------------------------------------------------------------

/// Prove that this binary can start WebView2 and load a real network document,
/// on a machine where no Scryer server is installed or running.
///
/// This is what catches a WebView2 runtime that is missing, a loader that did
/// not link, or a user-data folder the process cannot write.
mod smoke {
    use super::*;

    const WATCHDOG_TIMER_ID: usize = 1;
    const WATCHDOG_INTERVAL_MS: u32 = 1_000;

    pub(super) fn run() -> Result<(), String> {
        super::initialize_com();
        let port = shared::start_smoke_server().unwrap_or_else(|error| fail(&error));
        let url = shared::app_url(port);

        let user_data_folder = std::env::temp_dir().join("scryer-webview-smoke");
        std::fs::create_dir_all(&user_data_folder).unwrap_or_else(|error| {
            fail(&format!(
                "failed to create the smoke user data folder at {}: {error}",
                user_data_folder.display()
            ))
        });

        // The smoke test owns its whole thread, so it may create the webview
        // with the pumping helper: there is no other state on this thread for
        // a nested message loop to reenter.
        let window = create_smoke_window();
        let environment = {
            let user_data_folder = wide(&user_data_folder.to_string_lossy());
            let (sender, receiver) = std::sync::mpsc::channel();
            CreateCoreWebView2EnvironmentCompletedHandler::wait_for_async_operation(
                Box::new(move |handler| {
                    // SAFETY: The user data folder string outlives this call.
                    unsafe {
                        CreateCoreWebView2EnvironmentWithOptions(
                            PCWSTR::null(),
                            PCWSTR(user_data_folder.as_ptr()),
                            None::<&ICoreWebView2EnvironmentOptions>,
                            &handler,
                        )
                    }
                    .map_err(webview2_com::Error::WindowsError)
                }),
                Box::new(move |result, environment| {
                    result?;
                    sender
                        .send(environment.ok_or_else(|| windows::core::Error::from(E_POINTER)))
                        .expect("send over mpsc channel");
                    Ok(())
                }),
            )
            .unwrap_or_else(|error| {
                fail(&format!("WebView2 environment creation failed: {error}"))
            });
            receiver
                .recv()
                .unwrap_or_else(|error| {
                    fail(&format!(
                        "WebView2 environment was never delivered: {error}"
                    ))
                })
                .unwrap_or_else(|error| {
                    fail(&format!("WebView2 environment creation failed: {error}"))
                })
        };

        let controller = {
            let (sender, receiver) = std::sync::mpsc::channel();
            CreateCoreWebView2ControllerCompletedHandler::wait_for_async_operation(
                Box::new(move |handler| {
                    // SAFETY: `window` is a live top-level window on this thread.
                    unsafe {
                        environment.CreateCoreWebView2Controller(to_com_hwnd(window), &handler)
                    }
                    .map_err(webview2_com::Error::WindowsError)
                }),
                Box::new(move |result, controller| {
                    result?;
                    sender
                        .send(controller.ok_or_else(|| windows::core::Error::from(E_POINTER)))
                        .expect("send over mpsc channel");
                    Ok(())
                }),
            )
            .unwrap_or_else(|error| fail(&format!("WebView2 controller creation failed: {error}")));
            receiver
                .recv()
                .unwrap_or_else(|error| {
                    fail(&format!("WebView2 controller was never delivered: {error}"))
                })
                .unwrap_or_else(|error| {
                    fail(&format!("WebView2 controller creation failed: {error}"))
                })
        };

        // SAFETY: The controller was just created for this window.
        let webview = unsafe { controller.CoreWebView2() }
            .unwrap_or_else(|error| fail(&format!("WebView2 was not available: {error}")));

        let handler = NavigationCompletedEventHandler::create(Box::new(|_webview, arguments| {
            let Some(arguments) = arguments else {
                fail("WebView2 reported a navigation with no result");
            };
            // SAFETY: Both values are read out of the event arguments while
            // the event is being delivered.
            let (succeeded, status) = unsafe {
                let mut succeeded = windows::core::BOOL(0);
                arguments.IsSuccess(&mut succeeded)?;
                let mut status = Default::default();
                arguments.WebErrorStatus(&mut status)?;
                (succeeded.as_bool(), status)
            };
            if succeeded {
                report_success();
                std::process::exit(0);
            }
            fail(&format!(
                "WebView2 could not load the test page: web error status {}",
                status.0
            ));
        }));

        // SAFETY: The webview outlives the handler it reference counts, and
        // the navigation target is a loopback URL this process is serving.
        unsafe {
            let mut token = 0;
            webview
                .add_NavigationCompleted(&handler, &mut token)
                .unwrap_or_else(|error| {
                    fail(&format!("WebView2 event registration failed: {error}"))
                });
            webview
                .Navigate(&windows::core::HSTRING::from(&url))
                .unwrap_or_else(|error| fail(&format!("WebView2 navigation failed: {error}")));
        }

        // SAFETY: The window is live and the timer id is private to it.
        unsafe { SetTimer(window, WATCHDOG_TIMER_ID, WATCHDOG_INTERVAL_MS, None) };

        let deadline = Instant::now() + SMOKE_TIMEOUT;
        let mut message = MSG::default();
        loop {
            // SAFETY: `message` is valid writable storage for the call.
            let result = unsafe { GetMessageW(&mut message, ptr::null_mut(), 0, 0) };
            if result <= 0 {
                fail("the smoke test event loop ended before the page loaded");
            }
            if message.message == WM_TIMER && Instant::now() >= deadline {
                // SAFETY: The timer was set on this window above.
                unsafe { KillTimer(window, WATCHDOG_TIMER_ID) };
                fail("timed out waiting for WebView2 to load the test page");
            }
            // SAFETY: The message was populated by GetMessageW.
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }

    /// A real top-level window that is never shown. WebView2 needs a window to
    /// parent its browser to; it does not need anyone to see it, which is what
    /// makes this runnable on a build machine.
    fn create_smoke_window() -> HWND {
        let class_name = wide("ScryerMedia.Scryer.Desktop.v1.SmokeWindow");
        let window_class = WNDCLASSW {
            lpfnWndProc: Some(DefWindowProcW),
            lpszClassName: class_name.as_ptr(),
            ..Default::default()
        };
        // SAFETY: The class name and callback remain valid for process lifetime.
        if unsafe { RegisterClassW(&window_class) } == 0 {
            fail(&format!(
                "failed to register the smoke window class: {}",
                std::io::Error::last_os_error()
            ));
        }
        let title = wide("Scryer webview smoke");
        // SAFETY: The class is registered and both strings outlive the call.
        let window = unsafe {
            CreateWindowExW(
                0,
                class_name.as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                800,
                600,
                ptr::null_mut(),
                ptr::null_mut(),
                GetModuleHandleW(ptr::null()),
                ptr::null(),
            )
        };
        if window.is_null() {
            fail(&format!(
                "failed to create the smoke window: {}",
                std::io::Error::last_os_error()
            ));
        }
        window
    }

    /// Report a smoke failure and stop. The exit code is what CI reads; the
    /// message is what a human reads when CI goes red.
    ///
    /// Both of these write through `writeln!` rather than `println!` because
    /// `scryer-tray.exe` is a GUI-subsystem binary: it only has usable
    /// standard handles when the process that started it supplied them, and a
    /// panicking print would replace the real result with a confusing one.
    fn fail(reason: &str) -> ! {
        use std::io::Write;

        let mut stderr = std::io::stderr();
        let _ = writeln!(stderr, "webview-smoke: {reason}");
        let _ = stderr.flush();
        std::process::exit(1);
    }

    fn report_success() {
        use std::io::Write;

        let mut stdout = std::io::stdout();
        let _ = writeln!(stdout, "{SMOKE_SUCCESS_LINE}");
        let _ = stdout.flush();
    }
}

fn append_menu(menu: HMENU, id: u32, label: &str, flags: u32) -> Result<(), String> {
    let label = wide(label);
    // SAFETY: The menu is owned by the caller and the UTF-16 label remains live for the call.
    if unsafe { AppendMenuW(menu, flags, id as usize, label.as_ptr()) } == 0 {
        return Err(format!(
            "failed to add Scryer tray menu item: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

fn open_target(target: &str) -> Result<(), String> {
    let verb = wide("open");
    let target = wide(target);
    // SAFETY: Both strings are nul-terminated and remain live through the shell call.
    let result = unsafe {
        ShellExecuteW(
            ptr::null_mut(),
            verb.as_ptr(),
            target.as_ptr(),
            ptr::null(),
            ptr::null(),
            SW_SHOWNORMAL,
        )
    } as isize;
    if result <= 32 {
        return Err(format!(
            "Windows could not open {target:?}; ShellExecute error {result}"
        ));
    }
    Ok(())
}

pub(super) fn show_error(title: &str, message: &str) {
    let title = wide(title);
    let message = wide(message);
    // SAFETY: The message buffers are nul-terminated and remain live through the dialog call.
    unsafe {
        MessageBoxW(
            ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_ICONERROR | MB_OK,
        );
    }
}

fn wide(value: &str) -> Vec<u16> {
    std::ffi::OsStr::new(value)
        .encode_wide()
        .chain(Some(0))
        .collect()
}

fn write_wide_buffer(buffer: &mut [u16], value: &str) {
    let encoded = std::ffi::OsStr::new(value).encode_wide();
    for (slot, value) in buffer.iter_mut().zip(encoded) {
        *slot = value;
    }
}

#[cfg(test)]
mod tests {
    use windows_sys::Win32::UI::Shell::{NIN_POPUPCLOSE, NIN_POPUPOPEN, NIN_SELECT};
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_CONTEXTMENU, WM_LBUTTONUP, WM_RBUTTONUP};

    use super::{
        APP_WINDOW_CLASS, FLYOUT_CLOSE_TIMER, FLYOUT_POLL_TIMER, FLYOUT_WINDOW_CLASS,
        HIDE_WINDOW_VIRTUAL_KEY, MENU_EXIT, MENU_OPEN, MENU_OPEN_BROWSER, MENU_OPEN_LOGS,
        MENU_RESTART, MENU_START, MENU_STOP, MENU_TOGGLE_STARTUP, NIN_KEYSELECT, POINT,
        callback_anchor, tray_mutex_name_for_user,
    };

    #[test]
    fn tray_mutex_is_global_but_scoped_to_one_windows_user() {
        assert_eq!(
            tray_mutex_name_for_user("EXAMPLE-PC", "example"),
            "Global\\ScryerMedia.Scryer.Desktop.v1.Tray.004500580041004D0050004C0045002D00500043005C006500780061006D0070006C0065"
        );
    }

    #[test]
    fn every_menu_command_has_its_own_id() {
        let commands = [
            MENU_OPEN,
            MENU_OPEN_BROWSER,
            MENU_START,
            MENU_STOP,
            MENU_RESTART,
            MENU_OPEN_LOGS,
            MENU_TOGGLE_STARTUP,
            MENU_EXIT,
        ];
        let mut seen = commands.to_vec();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), commands.len(), "duplicate tray menu command id");
        // Zero is what TrackPopupMenu returns when the user dismisses the
        // menu, so no command may use it.
        assert!(!commands.contains(&0));
    }

    #[test]
    fn the_app_window_class_is_not_the_tray_class() {
        assert_ne!(APP_WINDOW_CLASS, super::CLASS_NAME);
        assert_ne!(FLYOUT_WINDOW_CLASS, super::CLASS_NAME);
        assert_ne!(FLYOUT_WINDOW_CLASS, APP_WINDOW_CLASS);
    }

    /// Every notification-icon event the tray answers has to reach a different
    /// arm. `NIN_KEYSELECT` is spelled out here rather than imported because
    /// windows-sys does not name it.
    #[test]
    fn every_tray_callback_event_is_distinct() {
        let events = [
            WM_LBUTTONUP,
            WM_RBUTTONUP,
            WM_CONTEXTMENU,
            NIN_SELECT,
            NIN_KEYSELECT,
            NIN_POPUPOPEN,
            NIN_POPUPCLOSE,
        ];
        let mut seen = events.to_vec();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), events.len(), "duplicate tray callback event");
        assert_eq!(NIN_KEYSELECT, NIN_SELECT + 1);
    }

    #[test]
    fn the_flyout_timers_do_not_collide() {
        assert_ne!(FLYOUT_POLL_TIMER, FLYOUT_CLOSE_TIMER);
        // The smoke watchdog's id, on a window neither timer is set on.
        assert_ne!(FLYOUT_POLL_TIMER, 1);
        assert_ne!(FLYOUT_CLOSE_TIMER, 1);
    }

    /// Version 4 packs the anchor into `wparam` as two signed 16-bit
    /// coordinates: a monitor left of or above the primary one has negative
    /// ones, and reading them unsigned would put the flyout off the desktop.
    #[test]
    fn the_callback_anchor_is_signed() {
        let packed = ((-40i16 as u16 as usize) << 16) | (-1200i16 as u16 as usize);
        let POINT { x, y } = callback_anchor(packed);
        assert_eq!((x, y), (-1200, -40));
    }

    #[test]
    fn ctrl_w_is_the_letter_w() {
        assert_eq!(HIDE_WINDOW_VIRTUAL_KEY, 0x57);
    }
}
