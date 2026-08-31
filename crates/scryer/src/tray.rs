#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod windows_startup;

#[cfg(any(windows, test))]
use std::ffi::OsStr;

#[cfg(any(windows, test))]
const SCRYER_ENV_PREFIX: &str = "SCRYER_";
#[cfg(any(windows, test))]
const AUTH_ENABLED_ENV: &str = "SCRYER_AUTH_ENABLED";

#[cfg(any(windows, test))]
fn should_remove_inherited_scryer_env(name: &OsStr, value: &OsStr) -> bool {
    let name = name.to_string_lossy();
    let is_scryer_env = name
        .get(..SCRYER_ENV_PREFIX.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(SCRYER_ENV_PREFIX));
    if !is_scryer_env {
        return false;
    }

    if !name.eq_ignore_ascii_case(AUTH_ENABLED_ENV) {
        return true;
    }

    !value.to_str().is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "y" | "on"
        )
    })
}

#[cfg(not(windows))]
fn main() {
    eprintln!("scryer-tray is only supported on Windows");
    std::process::exit(1);
}

#[cfg(windows)]
fn main() {
    if let Err(error) = windows::run() {
        windows::show_error("Scryer", &error);
        std::process::exit(1);
    }
}

#[cfg(windows)]
mod windows {
    use std::ffi::c_void;
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command};
    use std::ptr;
    use std::thread;
    use std::time::{Duration, Instant};

    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, LPARAM, LRESULT, POINT,
        WPARAM,
    };
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Threading::{CREATE_NO_WINDOW, CreateMutexW};
    use windows_sys::Win32::UI::Shell::{
        NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
        ShellExecuteW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
        DispatchMessageW, FindWindowW, GWLP_USERDATA, GetCursorPos, GetMessageW, GetWindowLongPtrW,
        HMENU, KillTimer, LoadIconW, MB_ICONERROR, MB_OK, MF_CHECKED, MF_SEPARATOR, MF_STRING,
        MF_UNCHECKED, MSG, MessageBoxW, PostMessageW, PostQuitMessage, RegisterClassW,
        SW_SHOWNORMAL, SetForegroundWindow, SetTimer, SetWindowLongPtrW, TPM_RETURNCMD,
        TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WM_APP, WM_DESTROY, WM_LBUTTONUP,
        WM_RBUTTONUP, WM_TIMER, WNDCLASSW,
    };

    const DEFAULT_PORT: u16 = 8080;
    const CLASS_NAME: &str = "ScryerMedia.Scryer.Desktop.v1.Tray";
    const MUTEX_NAMESPACE: &str = "Global\\ScryerMedia.Scryer.Desktop.v1.Tray.";
    const TRAY_CALLBACK_MESSAGE: u32 = WM_APP + 1;
    const OPEN_WINDOW_MESSAGE: u32 = WM_APP + 2;
    const SHUTDOWN_MESSAGE: u32 = WM_APP + 3;
    const ICON_RETRY_TIMER_ID: usize = 1;
    const SCRYER_ICON_RESOURCE_ID: usize = 1;

    const MENU_OPEN: u32 = 1;
    const MENU_START: u32 = 2;
    const MENU_STOP: u32 = 3;
    const MENU_RESTART: u32 = 4;
    const MENU_OPEN_LOGS: u32 = 5;
    const MENU_TOGGLE_STARTUP: u32 = 6;
    const MENU_EXIT: u32 = 7;

    enum LaunchMode {
        Interactive,
        Login,
        Shutdown,
        UnregisterStartup,
    }

    pub(super) fn run() -> Result<(), String> {
        let mode = launch_mode()?;
        match mode {
            LaunchMode::UnregisterStartup => return unregister_startup(),
            LaunchMode::Shutdown => return shutdown_existing_instance(),
            LaunchMode::Interactive | LaunchMode::Login => {}
        }

        let instance = InstanceGuard::acquire()?;
        if !instance.is_primary() {
            // A new interactive invocation brings the existing UI forward. A login-start
            // invocation must remain quiet, including after a delayed second Run-key launch.
            if matches!(mode, LaunchMode::Interactive) {
                let _ = signal_existing_instance(OPEN_WINDOW_MESSAGE);
            }
            return Ok(());
        }

        let profile_dir = desktop_profile_dir()?;
        std::fs::create_dir_all(profile_dir.join("logs")).map_err(|error| {
            format!(
                "failed to create Scryer desktop profile at {}: {error}",
                profile_dir.display()
            )
        })?;

        let class_name = wide(CLASS_NAME);
        // SAFETY: A null module name retrieves this executable's module handle.
        let module = unsafe { GetModuleHandleW(ptr::null()) };
        let window_class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            lpszClassName: class_name.as_ptr(),
            hInstance: module,
            ..Default::default()
        };
        // SAFETY: The class name and callback remain valid for the process lifetime.
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
                module,
                ptr::null(),
            )
        };
        if window.is_null() {
            return Err(format!(
                "failed to create Scryer tray window: {}",
                std::io::Error::last_os_error()
            ));
        }

        let state = Box::new(TrayState::new(
            profile_dir,
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

    fn launch_mode() -> Result<LaunchMode, String> {
        let mut args = std::env::args_os();
        let _program = args.next();
        match args.next() {
            None => Ok(LaunchMode::Interactive),
            Some(value) if value == "--login-start" => Ok(LaunchMode::Login),
            Some(value) if value == "--shutdown" => Ok(LaunchMode::Shutdown),
            Some(value) if value == "--unregister-startup" => Ok(LaunchMode::UnregisterStartup),
            Some(value) if value == "--version" || value == "-V" => {
                println!("scryer-tray {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            Some(value) => Err(format!(
                "unrecognized scryer-tray argument: {}",
                value.to_string_lossy()
            )),
        }
    }

    fn desktop_profile_dir() -> Result<PathBuf, String> {
        let local_app_data = std::env::var_os("LOCALAPPDATA").ok_or_else(|| {
            "LOCALAPPDATA is not set; cannot locate Scryer desktop data".to_string()
        })?;
        Ok(desktop_profile_dir_from(Path::new(&local_app_data)))
    }

    fn desktop_profile_dir_from(local_app_data: &Path) -> PathBuf {
        local_app_data.join("ScryerMedia").join("Scryer")
    }

    fn tray_mutex_name() -> Result<String, String> {
        let username = std::env::var("USERNAME").map_err(|_| {
            "USERNAME is not set; cannot scope the Scryer tray instance".to_string()
        })?;
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

    fn shutdown_existing_instance() -> Result<(), String> {
        let instance = InstanceGuard::acquire()?;
        if instance.is_primary() {
            return Ok(());
        }
        signal_existing_instance(SHUTDOWN_MESSAGE)?;

        let class_name = wide(CLASS_NAME);
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            // SAFETY: The class name is a valid nul-terminated UTF-16 string.
            if unsafe { FindWindowW(class_name.as_ptr(), ptr::null()) }.is_null() {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(100));
        }
        Err("timed out waiting for the existing Scryer tray to stop".to_string())
    }

    struct TrayState {
        profile_dir: PathBuf,
        login_start: bool,
        server: Option<Child>,
        icon_added: bool,
    }

    impl TrayState {
        fn new(profile_dir: PathBuf, login_start: bool) -> Self {
            Self {
                profile_dir,
                login_start,
                server: None,
                icon_added: false,
            }
        }

        unsafe fn initialize(&mut self, window: HWND) -> Result<(), String> {
            if self.login_start {
                self.start_server()?;
            } else {
                self.enable_startup()?;
                self.open_scryer()?;
            }

            // Explorer can still be starting when the Run-key entry launches the tray. Keep the
            // local server alive and retry the icon instead of treating that transient shell state
            // as a failed desktop launch.
            if unsafe { self.add_icon(window) }.is_err() {
                // SAFETY: The live tray window owns this retry timer until it is destroyed.
                unsafe { SetTimer(window, ICON_RETRY_TIMER_ID, 1_000, None) };
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
                uID: 1,
                uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
                uCallbackMessage: TRAY_CALLBACK_MESSAGE,
                hIcon: icon,
                ..Default::default()
            };
            write_wide_buffer(&mut data.szTip, "Scryer");
            // SAFETY: `data` is initialized and remains live through the system call.
            if unsafe { Shell_NotifyIconW(NIM_ADD, &data) } == 0 {
                return Err(format!(
                    "failed to add Scryer tray icon: {}",
                    std::io::Error::last_os_error()
                ));
            }
            self.icon_added = true;
            Ok(())
        }

        unsafe fn remove_icon(&mut self, window: HWND) {
            if !self.icon_added {
                return;
            }
            let data = NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: window,
                uID: 1,
                ..Default::default()
            };
            // SAFETY: The notification data identifies the icon added by this process.
            unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
            self.icon_added = false;
        }

        fn open_scryer(&mut self) -> Result<(), String> {
            self.start_server()?;
            if !wait_for_server(DEFAULT_PORT, Duration::from_secs(45)) {
                return Err(
                    "timed out waiting for Scryer to become ready at http://127.0.0.1:8080".into(),
                );
            }
            open_target("http://127.0.0.1:8080/")
        }

        fn start_server(&mut self) -> Result<(), String> {
            if server_ready(DEFAULT_PORT) {
                return Ok(());
            }
            if let Some(child) = self.server.as_mut() {
                match child.try_wait() {
                    Ok(None) => return Ok(()),
                    Ok(Some(_)) => self.server = None,
                    Err(error) => {
                        return Err(format!("failed to check Scryer server status: {error}"));
                    }
                }
            }

            let tray_exe = std::env::current_exe()
                .map_err(|error| format!("failed to resolve scryer-tray.exe path: {error}"))?;
            let scryer_exe = tray_exe.with_file_name("scryer.exe");
            if !scryer_exe.is_file() {
                return Err(format!(
                    "scryer.exe was not found beside scryer-tray.exe at {}",
                    scryer_exe.display()
                ));
            }
            let log_file = self.profile_dir.join("logs").join("scryer.log");
            let mut command = Command::new(&scryer_exe);
            // A desktop profile must not inherit a portable/server instance's database,
            // credentials, bind address, or other SCRYER_* runtime configuration. Preserve a
            // security-strengthening auth override so the tray cannot silently disable auth.
            for (name, value) in std::env::vars_os() {
                if super::should_remove_inherited_scryer_env(&name, &value) {
                    command.env_remove(name);
                }
            }
            let child = command
                .arg("--data-dir")
                .arg(&self.profile_dir)
                .arg("--log-file")
                .arg(&log_file)
                .env("SCRYER_BIND", format!("127.0.0.1:{DEFAULT_PORT}"))
                .env("SCRYER_OPEN_BROWSER", "false")
                .env("SCRYER_TRAY_SUPERVISED", "1")
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
                .map_err(|error| {
                    format!(
                        "failed to start Scryer from {}: {error}",
                        scryer_exe.display()
                    )
                })?;
            self.server = Some(child);
            Ok(())
        }

        fn stop_server(&mut self) -> Result<(), String> {
            let Some(mut child) = self.server.take() else {
                return Ok(());
            };
            if child
                .try_wait()
                .map_err(|error| format!("failed to check Scryer server status: {error}"))?
                .is_none()
            {
                child
                    .kill()
                    .map_err(|error| format!("failed to stop Scryer server: {error}"))?;
                child
                    .wait()
                    .map_err(|error| format!("failed to wait for Scryer server exit: {error}"))?;
            }
            Ok(())
        }

        fn restart_server(&mut self) -> Result<(), String> {
            self.stop_server()?;
            self.start_server()?;
            if wait_for_server(DEFAULT_PORT, Duration::from_secs(45)) {
                Ok(())
            } else {
                Err("timed out waiting for Scryer after restart".to_string())
            }
        }

        fn show_menu(&mut self, window: HWND) -> Result<(), String> {
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
                MENU_OPEN => self.open_scryer(),
                MENU_START => self.start_server(),
                MENU_STOP => self.stop_server(),
                MENU_RESTART => self.restart_server(),
                MENU_OPEN_LOGS => open_target(&self.profile_dir.join("logs").to_string_lossy()),
                MENU_TOGGLE_STARTUP => {
                    if startup_enabled()? {
                        unregister_startup()
                    } else {
                        self.enable_startup()
                    }
                }
                MENU_EXIT => {
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
            // SAFETY: The message loop serializes access to the state on this UI thread.
            let state = unsafe { &mut *state };
            let result = match message {
                TRAY_CALLBACK_MESSAGE if lparam as u32 == WM_LBUTTONUP => state.open_scryer(),
                TRAY_CALLBACK_MESSAGE if lparam as u32 == WM_RBUTTONUP => state.show_menu(window),
                OPEN_WINDOW_MESSAGE => state.open_scryer(),
                SHUTDOWN_MESSAGE => {
                    // SAFETY: This is the live window associated with the tray state.
                    unsafe { DestroyWindow(window) };
                    Ok(())
                }
                WM_TIMER if wparam == ICON_RETRY_TIMER_ID => {
                    // SAFETY: This retries notification-area registration for the live tray window.
                    if unsafe { state.add_icon(window) }.is_ok() {
                        // SAFETY: The icon is registered, so no further retries are needed.
                        unsafe { KillTimer(window, ICON_RETRY_TIMER_ID) };
                    }
                    Ok(())
                }
                WM_DESTROY => {
                    // SAFETY: The icon belongs to this window and is being removed during teardown.
                    unsafe { state.remove_icon(window) };
                    let _ = state.stop_server();
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

    fn wait_for_server(port: u16, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if server_ready(port) {
                return true;
            }
            thread::sleep(Duration::from_millis(250));
        }
        false
    }

    fn server_ready(port: u16) -> bool {
        let address = SocketAddr::from(([127, 0, 0, 1], port));
        let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(250))
        else {
            return false;
        };
        let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
        let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
        if stream
            .write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
            .is_err()
        {
            return false;
        }
        let mut response = [0u8; 128];
        let Ok(read) = stream.read(&mut response) else {
            return false;
        };
        response[..read].starts_with(b"HTTP/1.1 200")
            || response[..read].starts_with(b"HTTP/1.0 200")
    }

    // The per-user Run value is also read and restored by the temporary upgrade
    // helper after an MSI major upgrade, so its registry code is shared.
    fn register_startup(executable: &Path) -> Result<(), String> {
        crate::windows_startup::register_startup(executable)
    }

    fn unregister_startup() -> Result<(), String> {
        crate::windows_startup::unregister_startup()
    }

    fn startup_enabled() -> Result<bool, String> {
        crate::windows_startup::startup_enabled()
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
                "Windows could not open the requested target; ShellExecute error {result}"
            ));
        }
        Ok(())
    }

    pub(super) fn show_error(title: &str, message: &str) {
        if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
            let log_dir = PathBuf::from(local_app_data)
                .join("ScryerMedia")
                .join("Scryer")
                .join("logs");
            if std::fs::create_dir_all(&log_dir).is_ok() {
                if let Ok(mut log) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(log_dir.join("tray.log"))
                {
                    let _ = writeln!(log, "{title}: {message}");
                }
            }
        }
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
        use std::path::Path;

        use super::{desktop_profile_dir_from, tray_mutex_name_for_user};

        #[test]
        fn desktop_profile_is_isolated_from_legacy_portable_state() {
            assert_eq!(
                desktop_profile_dir_from(Path::new(concat!(
                    r"C:\",
                    r"Users\example\AppData\Local"
                ))),
                Path::new(concat!(
                    r"C:\",
                    r"Users\example\AppData\Local\ScryerMedia\Scryer"
                ))
            );
        }

        #[test]
        fn tray_mutex_is_global_but_scoped_to_one_windows_user() {
            assert_eq!(
                tray_mutex_name_for_user("SYLIX", "ai"),
                "Global\\ScryerMedia.Scryer.Desktop.v1.Tray.00530059004C00490058005C00610069"
            );
        }
    }
}

#[cfg(test)]
mod inherited_env_tests {
    use std::ffi::OsStr;

    use super::should_remove_inherited_scryer_env;

    #[test]
    fn preserves_security_strengthening_auth_override() {
        for value in ["1", "true", "TRUE", " yes ", "y", "on"] {
            assert!(!should_remove_inherited_scryer_env(
                OsStr::new("scryer_auth_enabled"),
                OsStr::new(value)
            ));
        }
    }

    #[test]
    fn removes_auth_overrides_that_do_not_enable_auth() {
        for value in ["0", "false", "no", "n", "off", "invalid", ""] {
            assert!(should_remove_inherited_scryer_env(
                OsStr::new("SCRYER_AUTH_ENABLED"),
                OsStr::new(value)
            ));
        }
    }

    #[test]
    fn isolates_other_scryer_environment_variables() {
        assert!(should_remove_inherited_scryer_env(
            OsStr::new("scryer_bind"),
            OsStr::new("0.0.0.0:8080")
        ));
        assert!(!should_remove_inherited_scryer_env(
            OsStr::new("PATH"),
            OsStr::new("example")
        ));
    }
}
