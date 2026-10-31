//! The macOS desktop wrapper: a menu-bar status item and a `WKWebView` window.
//!
//! This mirrors the Windows tray one behaviour at a time — the same menu, the
//! same supervised server, the same rule about which links leave the window —
//! with two deliberate differences that come from the platform:
//!
//! * There is no restart IPC. On Unix the server restarts by replacing its own
//!   process image, so its PID never changes and this wrapper never sees an
//!   exit to react to. A message channel would have nothing to carry.
//! * There is no "start at sign-in" item. macOS login items are registered
//!   through a service that has its own approval UI, and a checkbox that
//!   silently did nothing would be worse than no checkbox.

use std::cell::{Cell, RefCell};
use std::fs::File;
use std::os::fd::AsRawFd;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use block2::{DynBlock, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, ProtocolObject, Sel};
use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel,
};
use objc2_app_kit::{
    NSAccessibility, NSAlert, NSAlertFirstButtonReturn, NSAlertSecondButtonReturn, NSAppearance,
    NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSApplication, NSApplicationActivationPolicy,
    NSApplicationDelegate, NSApplicationTerminateReply, NSAutoresizingMaskOptions,
    NSBackingStoreType, NSBitmapImageRep, NSColor, NSControlSize, NSEvent, NSFont, NSImage,
    NSLayoutAttribute, NSLineBreakMode, NSMenu, NSMenuDelegate, NSMenuItem, NSModalResponse,
    NSOpenPanel, NSPopover, NSPopoverBehavior, NSProgressIndicator, NSProgressIndicatorStyle,
    NSSquareStatusItemLength, NSStackView, NSStatusBar, NSStatusItem, NSTextField, NSTrackingArea,
    NSTrackingAreaOptions, NSUserInterfaceLayoutOrientation, NSView, NSViewController, NSWindow,
    NSWindowDelegate, NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{
    NSArray, NSData, NSDistributedNotificationCenter, NSEdgeInsets, NSNotification,
    NSObjectProtocol, NSPoint, NSRect, NSRectEdge, NSSize, NSString, NSTimer, NSURL, NSURLRequest,
};
use objc2_web_kit::{
    WKFrameInfo, WKNavigation, WKNavigationAction, WKNavigationActionPolicy, WKNavigationDelegate,
    WKOpenPanelParameters, WKUIDelegate, WKWebView, WKWebViewConfiguration, WKWindowFeatures,
};

use super::shared::{
    self, DEFAULT_PORT, POPOVER_WIDTH, PopoverContent, SERVER_READY_TIMEOUT, SMOKE_SUCCESS_LINE,
    SMOKE_TIMEOUT, ServerSupervisor, SupervisedServer,
};

/// The notification a second invocation posts so the running instance shows
/// its window. Launch Services already routes a second `.app` launch to the
/// running process; this covers the case where the binary inside the bundle is
/// run directly.
const OPEN_NOTIFICATION: &str = "media.scryer.app.open";

/// The lock the primary instance holds for as long as it runs.
const INSTANCE_LOCK_FILE: &str = "scryer-tray.lock";

/// How often the wrapper checks whether the server has come up.
const READY_POLL_INTERVAL: f64 = 0.25;

/// How often the wrapper checks on the server it started.
///
/// This only has to notice a bundle upgrade's relaunch request promptly enough
/// that the user does not sit looking at a stopped app; a second is well inside
/// that and costs a `waitpid` poll.
const SUPERVISION_POLL_INTERVAL: f64 = 1.0;

/// The app window's default size. Scryer's UI is a dense table layout, so the
/// first-run window is sized for it rather than for the smallest usable frame.
const WINDOW_WIDTH: f64 = 1280.0;
const WINDOW_HEIGHT: f64 = 800.0;
const WINDOW_MIN_WIDTH: f64 = 720.0;
const WINDOW_MIN_HEIGHT: f64 = 480.0;

/// Result of the background readiness probe, read by the main-thread timer.
const PROBE_PENDING: u8 = 0;
const PROBE_READY: u8 = 1;
const PROBE_TIMED_OUT: u8 = 2;

/// The menu-bar glyph: one drawing per menu-bar appearance, at both
/// backing-store scales. The files are named for the appearance they serve —
/// the dark-named drawing is the white one, the light-named one is black. Both
/// come from the single-colour macOS mark, matching Weaver's menu-bar glyphs;
/// see `packaging/macos/assets/generate-assets.sh`.
const MENU_BAR_ICON_LIGHT: &[u8] = include_bytes!("../../resources/macos/menubar-light.png");
const MENU_BAR_ICON_LIGHT_2X: &[u8] = include_bytes!("../../resources/macos/menubar-light@2x.png");
const MENU_BAR_ICON_DARK: &[u8] = include_bytes!("../../resources/macos/menubar-dark.png");
const MENU_BAR_ICON_DARK_2X: &[u8] = include_bytes!("../../resources/macos/menubar-dark@2x.png");

/// The glyph's size in points. Both representations are declared at this size;
/// the 2x one simply has twice the pixels.
const MENU_BAR_ICON_POINTS: f64 = 18.0;

/// How far the popover's content sits from its edges, and the width its rows
/// are laid out at.
const POPOVER_INSET: f64 = 14.0;
const POPOVER_CONTENT_WIDTH: f64 = POPOVER_WIDTH - 2.0 * POPOVER_INSET;

/// How often the popover's timer looks for a finished fetch. This is the
/// latency of a result appearing, not how often the server is asked.
const POPOVER_POLL_INTERVAL: f64 = 0.2;

/// How often the popover asks the server again while it stays open. Nothing is
/// fetched at all while it is closed.
const POPOVER_REFRESH_INTERVAL: Duration = Duration::from_secs(2);

/// How long the popover survives the pointer leaving it. The gap between the
/// status item and the popover's own view is crossed with the pointer inside
/// neither, so closing immediately would make the popover unreachable.
const POPOVER_CLOSE_GRACE: f64 = 0.25;

/// AppKit's `NSModalResponseOK`. It is a header macro, so the bindings carry
/// only its siblings; the value is contractual.
const MODAL_RESPONSE_OK: NSModalResponse = 1;

enum LaunchMode {
    Interactive,
    WebviewSmoke,
}

pub(super) fn run() -> Result<(), String> {
    match launch_mode()? {
        LaunchMode::WebviewSmoke => run_webview_smoke(),
        LaunchMode::Interactive => run_interactive(),
    }
}

fn launch_mode() -> Result<LaunchMode, String> {
    let mut mode = LaunchMode::Interactive;
    for argument in std::env::args_os().skip(1) {
        if argument == "--version" || argument == "-V" {
            // The same line the shipped Windows tray prints, so anything that
            // already parses it keeps working on both platforms.
            println!("scryer-tray {}", env!("CARGO_PKG_VERSION"));
            std::process::exit(0);
        } else if argument == "--webview-smoke" {
            mode = LaunchMode::WebviewSmoke;
        } else if argument
            .to_string_lossy()
            .starts_with(PROCESS_SERIAL_NUMBER_PREFIX)
        {
            // Launch Services has historically appended a process serial
            // number when opening a bundle. It is not an argument the wrapper
            // chose to receive, so it must not be an error.
        } else {
            return Err(format!(
                "unrecognized scryer-tray argument: {}",
                argument.to_string_lossy()
            ));
        }
    }
    Ok(mode)
}

const PROCESS_SERIAL_NUMBER_PREFIX: &str = "-psn_";

fn run_interactive() -> Result<(), String> {
    let profile_dir = shared::desktop_profile_dir()?;
    let supervisor = ServerSupervisor::new(profile_dir, DEFAULT_PORT);
    supervisor.ensure_profile_dirs()?;

    let lock = match acquire_instance_lock(supervisor.profile_dir())? {
        Some(lock) => lock,
        None => {
            // Another wrapper owns the window. Ask it to come forward and go
            // away quietly: two menu-bar items for one server would be a bug
            // the user cannot fix.
            post_open_notification();
            return Ok(());
        }
    };
    // The lock is released by the kernel when this process exits; holding the
    // file open for the whole run is the entire mechanism.
    std::mem::forget(lock);

    let mtm = MainThreadMarker::new()
        .ok_or_else(|| "the Scryer wrapper must start on the main thread".to_string())?;
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);

    let delegate = ScryerDelegate::new(mtm, supervisor);
    let protocol = ProtocolObject::from_ref(&*delegate);
    app.setDelegate(Some(protocol));
    app.run();
    Ok(())
}

/// Take the single-instance lock, or report that another instance holds it.
///
/// `flock` is used rather than a pid file because the kernel releases it on
/// every exit path, including a crash — a stale pid file would leave the user
/// with a wrapper that refuses to start and no way to tell why.
fn acquire_instance_lock(profile_dir: &Path) -> Result<Option<File>, String> {
    let path = profile_dir.join(INSTANCE_LOCK_FILE);
    let file = File::options()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .map_err(|error| {
            format!(
                "failed to open the Scryer instance lock at {}: {error}",
                path.display()
            )
        })?;
    // SAFETY: The descriptor is owned by `file`, which outlives this call.
    let locked = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if locked == 0 {
        return Ok(Some(file));
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::EWOULDBLOCK) {
        return Ok(None);
    }
    Err(format!(
        "failed to take the Scryer instance lock at {}: {error}",
        path.display()
    ))
}

fn post_open_notification() {
    let center = NSDistributedNotificationCenter::defaultCenter();
    let name = NSString::from_str(OPEN_NOTIFICATION);
    // SAFETY: The notification carries no object and no user info, so there is
    // nothing for the receiving process to decode.
    unsafe { center.postNotificationName_object(&name, None) };
}

/// The views the popover updates in place. Rebuilding them per refresh would
/// throw away the layout and flicker; rows the queue does not fill are hidden,
/// which is what takes them out of the stack's layout too.
struct PopoverRowViews {
    row: Retained<NSStackView>,
    name: Retained<NSTextField>,
    progress: Retained<NSProgressIndicator>,
    detail: Retained<NSTextField>,
}

struct PopoverViews {
    stack: Retained<NSStackView>,
    status: Retained<NSTextField>,
    message: Retained<NSTextField>,
    rows: Vec<PopoverRowViews>,
}

/// State the delegate owns for the whole run.
struct DelegateState {
    supervisor: RefCell<ServerSupervisor>,
    window: RefCell<Option<Retained<NSWindow>>>,
    webview: RefCell<Option<Retained<WKWebView>>>,
    status_item: RefCell<Option<Retained<NSStatusItem>>>,
    ready_timer: RefCell<Option<Retained<NSTimer>>>,
    ready_probe: RefCell<Option<Arc<AtomicU8>>>,
    /// Watches the supervised server for the bundle-upgrade relaunch request.
    supervision_timer: RefCell<Option<Retained<NSTimer>>>,
    popover: RefCell<Option<Retained<NSPopover>>>,
    popover_views: RefCell<Option<PopoverViews>>,
    popover_timer: RefCell<Option<Retained<NSTimer>>>,
    popover_close_timer: RefCell<Option<Retained<NSTimer>>>,
    /// Where a finished fetch leaves its result for the main thread to draw.
    status_result: Arc<Mutex<Option<PopoverContent>>>,
    status_fetching: Arc<AtomicBool>,
    status_fetched_at: Cell<Option<Instant>>,
    /// Set once a fetch has answered, so a reopened popover shows the last
    /// status rather than the placeholder again.
    status_answered: Cell<bool>,
    /// Set by the status-item's own Quit before it asks the app to terminate.
    /// Every other quit path — Cmd+Q, the Dock — prompts first.
    quit_confirmed: Cell<bool>,
    /// Set once the app URL has been loaded, so a later show does not throw
    /// the user back to the splash.
    showing_app: Cell<bool>,
    origin: String,
    url: String,
}

define_class!(
    // SAFETY:
    // - NSObject imposes no subclassing requirements.
    // - The class is main-thread only, which every AppKit and WebKit delegate
    //   callback below relies on, and it does not implement Drop.
    #[unsafe(super(objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ScryerMediaDesktopDelegate"]
    #[ivars = DelegateState]
    struct ScryerDelegate;

    impl ScryerDelegate {
        #[unsafe(method(openScryer:))]
        fn menu_open_scryer(&self, _sender: Option<&AnyObject>) {
            self.report(self.open_scryer());
        }

        #[unsafe(method(openInBrowser:))]
        fn menu_open_in_browser(&self, _sender: Option<&AnyObject>) {
            let result = self
                .ensure_server_ready()
                .and_then(|()| open_external(&self.ivars().url));
            self.report(result);
        }

        #[unsafe(method(startScryer:))]
        fn menu_start_scryer(&self, _sender: Option<&AnyObject>) {
            let result = self.ivars().supervisor.borrow_mut().start();
            self.report(result);
        }

        #[unsafe(method(stopScryer:))]
        fn menu_stop_scryer(&self, _sender: Option<&AnyObject>) {
            let result = self.ivars().supervisor.borrow_mut().stop();
            self.report(result);
        }

        #[unsafe(method(restartScryer:))]
        fn menu_restart_scryer(&self, _sender: Option<&AnyObject>) {
            let result = self.ivars().supervisor.borrow_mut().restart();
            self.report(result);
        }

        #[unsafe(method(openLogs:))]
        fn menu_open_logs(&self, _sender: Option<&AnyObject>) {
            let logs = self.ivars().supervisor.borrow().logs_dir();
            let url = NSURL::fileURLWithPath(&NSString::from_str(&logs.to_string_lossy()));
            if !NSWorkspace::sharedWorkspace().openURL(&url) {
                self.report(Err(format!(
                    "macOS could not open the Scryer log folder at {}",
                    logs.display()
                )));
            }
        }

        #[unsafe(method(quitScryer:))]
        fn menu_quit_scryer(&self, _sender: Option<&AnyObject>) {
            self.ivars().quit_confirmed.set(true);
            NSApplication::sharedApplication(self.mtm()).terminate(None);
        }

        /// The distributed notification a second invocation posts.
        #[unsafe(method(showWindowFromNotification:))]
        fn show_window_from_notification(&self, _notification: &NSNotification) {
            self.report(self.open_scryer());
        }

        /// Timer callback: has the server come up yet?
        #[unsafe(method(pollServerReady:))]
        fn poll_server_ready(&self, _timer: &NSTimer) {
            let state = self
                .ivars()
                .ready_probe
                .borrow()
                .as_ref()
                .map_or(PROBE_TIMED_OUT, |probe| probe.load(Ordering::Relaxed));
            match state {
                PROBE_PENDING => {}
                PROBE_READY => {
                    self.stop_ready_polling();
                    self.load_app();
                }
                _ => {
                    self.stop_ready_polling();
                    self.load_html(&startup_failure_html());
                }
            }
        }

        /// The pointer reached the status item, or the popover itself. Both
        /// mean the same thing — stay open — and reshowing an open popover is
        /// a no-op, so neither area needs to be told apart from the other.
        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, _event: &NSEvent) {
            self.cancel_popover_close();
            self.show_popover();
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            self.schedule_popover_close();
        }

        #[unsafe(method(closePopoverAfterGrace:))]
        fn close_popover_after_grace(&self, _timer: &NSTimer) {
            self.hide_popover();
        }

        /// Timer callback: has the server asked for the app to be relaunched?
        ///
        /// A bundle upgrade replaces this wrapper's own binary, so the running
        /// wrapper cannot become the new build. When its server reports the
        /// swap succeeded, the wrapper hands off to a fresh instance launched
        /// from the replaced bundle and quits.
        #[unsafe(method(pollSupervisedServer:))]
        fn poll_supervised_server(&self, _timer: &NSTimer) {
            let observed = self.ivars().supervisor.borrow_mut().poll_supervised_server();
            match observed {
                Ok(SupervisedServer::RelaunchRequested) => self.relaunch_after_bundle_upgrade(),
                Ok(_) => {}
                Err(error) => eprintln!("failed to check the Scryer server: {error}"),
            }
        }

        /// Timer callback: draw whatever the fetch thread has left, and start
        /// the next fetch once the current answer is stale.
        #[unsafe(method(refreshPopover:))]
        fn refresh_popover(&self, _timer: &NSTimer) {
            let finished = self
                .ivars()
                .status_result
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take();
            if let Some(content) = finished {
                self.ivars().status_answered.set(true);
                self.render_popover(&content);
            }
            let due = self
                .ivars()
                .status_fetched_at
                .get()
                .is_none_or(|started| started.elapsed() >= POPOVER_REFRESH_INTERVAL);
            if due && !self.ivars().status_fetching.load(Ordering::Acquire) {
                self.spawn_status_fetch();
            }
        }
    }

    unsafe impl NSObjectProtocol for ScryerDelegate {}

    unsafe impl NSMenuDelegate for ScryerDelegate {
        /// A click on the status item opens the menu. The popover the same
        /// pointer just opened would sit under it, so it goes first.
        #[unsafe(method(menuWillOpen:))]
        fn menu_will_open(&self, _menu: &NSMenu) {
            self.hide_popover();
        }
    }

    unsafe impl NSApplicationDelegate for ScryerDelegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn application_did_finish_launching(&self, _notification: &NSNotification) {
            install_main_menu(self.mtm());
            self.install_status_item();
            self.observe_open_notifications();
            self.begin_supervision_polling();
            self.report(self.open_scryer());
        }

        /// The wrapper is a menu-bar app first: quitting is what the status
        /// item's Quit does. Every other trigger — Cmd+Q, the Dock — asks,
        /// because macOS muscle memory fires it at apps that are really
        /// windows, and this window is not the tool.
        #[unsafe(method(applicationShouldTerminate:))]
        fn application_should_terminate(
            &self,
            _sender: &NSApplication,
        ) -> NSApplicationTerminateReply {
            if let Err(error) = self.ivars().supervisor.borrow().ensure_restore_idle() {
                self.report(Err(error));
                return NSApplicationTerminateReply::TerminateCancel;
            }
            if self.ivars().quit_confirmed.get() {
                return NSApplicationTerminateReply::TerminateNow;
            }
            let mtm = self.mtm();
            let alert = NSAlert::new(mtm);
            alert.setMessageText(&NSString::from_str("Quit Scryer?"));
            alert.setInformativeText(&NSString::from_str(
                "Scryer keeps running in the menu bar while the window is closed. \
                 Quitting stops the server and any downloads in progress.",
            ));
            alert.addButtonWithTitle(&NSString::from_str("Close Window"));
            alert.addButtonWithTitle(&NSString::from_str("Quit Scryer"));
            alert.addButtonWithTitle(&NSString::from_str("Cancel"));
            let response = alert.runModal();
            if response == NSAlertSecondButtonReturn {
                return NSApplicationTerminateReply::TerminateNow;
            }
            if response == NSAlertFirstButtonReturn
                && let Some(window) = self.ivars().window.borrow().as_ref()
            {
                window.orderOut(None);
            }
            NSApplicationTerminateReply::TerminateCancel
        }

        /// The wrapper lives in the menu bar, so closing the window is not
        /// quitting.
        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn application_should_terminate_after_last_window_closed(
            &self,
            _sender: &NSApplication,
        ) -> bool {
            false
        }

        /// Clicking the Dock icon brings the window back.
        #[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]
        fn application_should_handle_reopen(
            &self,
            _sender: &NSApplication,
            _has_visible_windows: bool,
        ) -> bool {
            self.report(self.open_scryer());
            true
        }

        /// Every quit path ends here, so this is where the server the wrapper
        /// started is stopped — a wrapper that exited leaving its own child
        /// serving the port would look exactly like a failure to quit.
        #[unsafe(method(applicationWillTerminate:))]
        fn application_will_terminate(&self, _notification: &NSNotification) {
            self.stop_ready_polling();
            self.stop_supervision_polling();
            self.hide_popover();
            let _ = self.ivars().supervisor.borrow_mut().stop();
        }
    }

    unsafe impl NSWindowDelegate for ScryerDelegate {
        #[unsafe(method(windowShouldClose:))]
        fn window_should_close(&self, sender: &NSWindow) -> bool {
            // Hiding rather than closing keeps the webview's process, session
            // and scroll position alive, so reopening is instant.
            sender.orderOut(None);
            false
        }
    }

    unsafe impl WKNavigationDelegate for ScryerDelegate {
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        fn decide_policy_for_navigation_action(
            &self,
            _web_view: &WKWebView,
            navigation_action: &WKNavigationAction,
            decision_handler: &DynBlock<dyn Fn(WKNavigationActionPolicy)>,
        ) {
            if self.leaves_the_app(navigation_action) {
                // SAFETY: `request` and `URL` are the action's own properties.
                let url = unsafe { navigation_action.request().URL() };
                decision_handler.call((WKNavigationActionPolicy::Cancel,));
                if let Some(url) = url {
                    NSWorkspace::sharedWorkspace().openURL(&url);
                }
                return;
            }
            decision_handler.call((WKNavigationActionPolicy::Allow,));
        }
    }

    unsafe impl WKUIDelegate for ScryerDelegate {
        /// `target="_blank"` and `window.open`. Returning nil tells WebKit no
        /// view was created; the link has already been handed to the browser.
        #[unsafe(method_id(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:))]
        fn create_web_view(
            &self,
            _web_view: &WKWebView,
            _configuration: &WKWebViewConfiguration,
            navigation_action: &WKNavigationAction,
            _window_features: &WKWindowFeatures,
        ) -> Option<Retained<WKWebView>> {
            // SAFETY: `request` and `URL` are the action's own properties.
            if let Some(url) = unsafe { navigation_action.request().URL() } {
                NSWorkspace::sharedWorkspace().openURL(&url);
            }
            None
        }

        /// `<input type="file">`. WebKit draws no chooser of its own — without
        /// this method the page's upload buttons are silent no-ops.
        #[unsafe(method(webView:runOpenPanelWithParameters:initiatedByFrame:completionHandler:))]
        fn run_open_panel(
            &self,
            _web_view: &WKWebView,
            parameters: &WKOpenPanelParameters,
            _frame: &WKFrameInfo,
            completion_handler: &DynBlock<dyn Fn(*mut NSArray<NSURL>)>,
        ) {
            let panel = NSOpenPanel::openPanel(self.mtm());
            panel.setCanChooseFiles(true);
            // SAFETY: Plain property reads on the parameters WebKit handed in.
            unsafe {
                panel.setAllowsMultipleSelection(parameters.allowsMultipleSelection());
                panel.setCanChooseDirectories(parameters.allowsDirectories());
            }
            // WebKit's handler must be answered exactly once, whichever way
            // the panel closes; copying moves ownership into the block below.
            let completion = completion_handler.copy();
            let chosen = panel.clone();
            let handler = RcBlock::new(move |response: NSModalResponse| {
                if response == MODAL_RESPONSE_OK {
                    let urls = chosen.URLs();
                    completion.call((Retained::as_ptr(&urls).cast_mut(),));
                } else {
                    completion.call((std::ptr::null_mut(),));
                }
            });
            // A sheet needs a window; a chooser triggered from a window that
            // has since been closed still has to answer, just standalone.
            match self.ivars().window.borrow().as_ref() {
                Some(window) => panel.beginSheetModalForWindow_completionHandler(window, &handler),
                None => panel.beginWithCompletionHandler(&handler),
            }
        }
    }
);

impl ScryerDelegate {
    fn new(mtm: MainThreadMarker, supervisor: ServerSupervisor) -> Retained<Self> {
        let port = supervisor.port();
        let this = Self::alloc(mtm).set_ivars(DelegateState {
            supervisor: RefCell::new(supervisor),
            window: RefCell::new(None),
            webview: RefCell::new(None),
            status_item: RefCell::new(None),
            ready_timer: RefCell::new(None),
            ready_probe: RefCell::new(None),
            supervision_timer: RefCell::new(None),
            popover: RefCell::new(None),
            popover_views: RefCell::new(None),
            popover_timer: RefCell::new(None),
            popover_close_timer: RefCell::new(None),
            status_result: Arc::new(Mutex::new(None)),
            status_fetching: Arc::new(AtomicBool::new(false)),
            status_fetched_at: Cell::new(None),
            status_answered: Cell::new(false),
            quit_confirmed: Cell::new(false),
            showing_app: Cell::new(false),
            origin: shared::app_origin(port),
            url: shared::app_url(port),
        });
        // SAFETY: `init` on the NSObject superclass has no further requirements.
        unsafe { msg_send![super(this), init] }
    }

    fn install_status_item(&self) {
        let mtm = self.mtm();
        let status_item =
            NSStatusBar::systemStatusBar().statusItemWithLength(NSSquareStatusItemLength);
        if let Some(button) = status_item.button(mtm) {
            if let Some(icon) = menu_bar_icon() {
                button.setImage(Some(&icon));
            }
            // The glyph carries no text, so the name has to live somewhere a
            // pointer and a screen reader can each find it.
            let name = NSString::from_str("Scryer");
            button.setToolTip(Some(&name));
            button.setAccessibilityTitle(Some(&name));
            self.install_hover_tracking(&button);
        }
        let menu = self.build_menu(mtm);
        status_item.setMenu(Some(&menu));
        *self.ivars().status_item.borrow_mut() = Some(status_item);
    }

    fn build_menu(&self, mtm: MainThreadMarker) -> Retained<NSMenu> {
        let menu = NSMenu::new(mtm);
        menu.setDelegate(Some(ProtocolObject::from_ref(self)));
        // Items are enabled by this wrapper, not by the responder chain: the
        // status menu has no first responder to validate against.
        menu.setAutoenablesItems(false);

        self.append_item(&menu, mtm, "Open Scryer", sel!(openScryer:));
        self.append_item(&menu, mtm, "Open in Browser", sel!(openInBrowser:));
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        self.append_item(&menu, mtm, "Start Scryer", sel!(startScryer:));
        self.append_item(&menu, mtm, "Stop Scryer", sel!(stopScryer:));
        self.append_item(&menu, mtm, "Restart Scryer", sel!(restartScryer:));
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        self.append_item(&menu, mtm, "Open Logs", sel!(openLogs:));
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        self.append_item(&menu, mtm, "Quit Scryer", sel!(quitScryer:));
        menu
    }

    fn append_item(&self, menu: &NSMenu, mtm: MainThreadMarker, title: &str, action: Sel) {
        let title = NSString::from_str(title);
        let key = NSString::from_str("");
        // SAFETY: The selector is implemented by this class, which is also the
        // target set immediately below.
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &title,
                Some(action),
                &key,
            )
        };
        // SAFETY: The target outlives the menu; both are owned by this delegate.
        unsafe { item.setTarget(Some(self.as_any())) };
        item.setEnabled(true);
        menu.addItem(&item);
    }

    fn observe_open_notifications(&self) {
        let center = NSDistributedNotificationCenter::defaultCenter();
        let name = NSString::from_str(OPEN_NOTIFICATION);
        // SAFETY: The selector is implemented by this class, and the observer
        // (this delegate) is owned by the application for the whole run, so it
        // is never dropped while registered.
        unsafe {
            center.addObserver_selector_name_object(
                self.as_any(),
                sel!(showWindowFromNotification:),
                Some(&name),
                None,
            );
        }
    }

    /// Show the app window, starting the server if it is not up yet.
    ///
    /// The window is shown before the server answers, on purpose: the user
    /// asked for a window and the wait is the interesting part, so the splash
    /// is what they see while the server comes up.
    fn open_scryer(&self) -> Result<(), String> {
        self.ivars().supervisor.borrow_mut().start()?;
        self.show_window();

        let port = self.ivars().supervisor.borrow().port();
        // Re-checking here is what makes "Stop Scryer" followed by "Open
        // Scryer" work: the window is still showing the UI it loaded before
        // the server went away, and that page is now dead.
        if self.ivars().showing_app.get() && shared::server_ready(port) {
            return Ok(());
        }
        self.ivars().showing_app.set(false);
        self.load_html(&splash_html());
        self.begin_ready_polling();
        Ok(())
    }

    /// Used by the menu items that only need a server, not a window.
    fn ensure_server_ready(&self) -> Result<(), String> {
        let mut supervisor = self.ivars().supervisor.borrow_mut();
        supervisor.start()?;
        supervisor.wait_until_ready()
    }

    fn show_window(&self) {
        let window = self.window();
        window.makeKeyAndOrderFront(None);
        NSApplication::sharedApplication(self.mtm()).activate();
    }

    /// The app window, created on first use.
    fn window(&self) -> Retained<NSWindow> {
        if let Some(window) = self.ivars().window.borrow().as_ref() {
            return window.clone();
        }

        let mtm = self.mtm();
        let frame = NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(WINDOW_WIDTH, WINDOW_HEIGHT),
        );
        let style = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable
            | NSWindowStyleMask::Resizable;
        // SAFETY: The window is created and used only on the main thread, and
        // it is retained by this delegate for the rest of the run.
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                frame,
                style,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        window.setTitle(&NSString::from_str("Scryer"));
        window.setMinSize(NSSize::new(WINDOW_MIN_WIDTH, WINDOW_MIN_HEIGHT));
        window.center();
        // AppKit's default is to release a window when it closes; this window
        // is hidden and reused instead, so that default would be a use after
        // free the first time the user closes it.
        // SAFETY: The window is retained by this delegate, so AppKit must not
        // release it when the user closes it.
        unsafe { window.setReleasedWhenClosed(false) };
        window.setDelegate(Some(ProtocolObject::from_ref(self)));
        // The titlebar draws over the window background, which is painted in
        // the web UI's own background color, so the chrome reads as part of
        // the page. The traffic lights follow the system appearance on their
        // own.
        window.setTitlebarAppearsTransparent(true);
        window.setBackgroundColor(Some(&theme_background_color(mtm)));

        let webview = build_webview(mtm, frame);
        // SAFETY: Both delegates are this object, which outlives the webview
        // it is attached to, and both are weak references WebKit only calls on
        // the main thread.
        unsafe {
            webview.setNavigationDelegate(Some(ProtocolObject::from_ref(self)));
            webview.setUIDelegate(Some(ProtocolObject::from_ref(self)));
        }
        webview.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        window.setContentView(Some(&webview));

        *self.ivars().webview.borrow_mut() = Some(webview);
        *self.ivars().window.borrow_mut() = Some(window.clone());
        window
    }

    /// Start watching for the server, showing the splash until it answers.
    ///
    /// The probe itself runs off the main thread: it opens a socket and reads
    /// a response, and doing that on the main thread would freeze the window
    /// it is supposed to be filling.
    fn begin_ready_polling(&self) {
        if self.ivars().ready_timer.borrow().is_some() {
            return;
        }
        let port = self.ivars().supervisor.borrow().port();
        let probe = Arc::new(AtomicU8::new(PROBE_PENDING));
        let worker = Arc::clone(&probe);
        std::thread::spawn(move || {
            let outcome = if shared::wait_for_server(port, SERVER_READY_TIMEOUT) {
                PROBE_READY
            } else {
                PROBE_TIMED_OUT
            };
            worker.store(outcome, Ordering::Relaxed);
        });
        *self.ivars().ready_probe.borrow_mut() = Some(probe);

        // SAFETY: The selector is implemented by this class, and the timer is
        // invalidated before the delegate could go away.
        let timer = unsafe {
            NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                READY_POLL_INTERVAL,
                self.as_any(),
                sel!(pollServerReady:),
                None,
                true,
            )
        };
        *self.ivars().ready_timer.borrow_mut() = Some(timer);
    }

    fn stop_ready_polling(&self) {
        if let Some(timer) = self.ivars().ready_timer.borrow_mut().take() {
            timer.invalidate();
        }
        *self.ivars().ready_probe.borrow_mut() = None;
    }

    // -- the hover popover ---------------------------------------------------

    /// Watch the status item for the pointer.
    ///
    /// `InVisibleRect` rather than a rectangle of this wrapper's own: the
    /// status item moves and resizes whenever the menu bar does, and an area
    /// pinned to a stale rectangle would silently stop firing.
    fn install_hover_tracking(&self, view: &NSView) {
        // SAFETY: The owner is this delegate, which outlives the view it is
        // attached to, and no user info is passed for the class to decode.
        let area = unsafe {
            NSTrackingArea::initWithRect_options_owner_userInfo(
                NSTrackingArea::alloc(),
                NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0)),
                NSTrackingAreaOptions::MouseEnteredAndExited
                    | NSTrackingAreaOptions::ActiveAlways
                    | NSTrackingAreaOptions::InVisibleRect,
                Some(self.as_any()),
                None,
            )
        };
        view.addTrackingArea(&area);
    }

    fn show_popover(&self) {
        let mtm = self.mtm();
        let Some(button) = self
            .ivars()
            .status_item
            .borrow()
            .as_ref()
            .and_then(|item| item.button(mtm))
        else {
            return;
        };
        let popover = self.popover(mtm);
        if !popover.isShown() {
            if !self.ivars().status_answered.get() {
                self.render_popover(&PopoverContent {
                    status: None,
                    rows: Vec::new(),
                    message: Some("Checking Scryer…".to_string()),
                });
            }
            popover.showRelativeToRect_ofView_preferredEdge(
                button.bounds(),
                &button,
                NSRectEdge::MinY,
            );
        }
        self.begin_status_polling();
    }

    fn hide_popover(&self) {
        self.cancel_popover_close();
        self.stop_status_polling();
        if let Some(popover) = self.ivars().popover.borrow().as_ref() {
            popover.close();
        }
    }

    fn schedule_popover_close(&self) {
        self.cancel_popover_close();
        // SAFETY: The selector is implemented by this class, and the timer is
        // invalidated before the delegate could go away.
        let timer = unsafe {
            NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                POPOVER_CLOSE_GRACE,
                self.as_any(),
                sel!(closePopoverAfterGrace:),
                None,
                false,
            )
        };
        *self.ivars().popover_close_timer.borrow_mut() = Some(timer);
    }

    fn cancel_popover_close(&self) {
        if let Some(timer) = self.ivars().popover_close_timer.borrow_mut().take() {
            timer.invalidate();
        }
    }

    /// The popover, created on first hover.
    fn popover(&self, mtm: MainThreadMarker) -> Retained<NSPopover> {
        if let Some(popover) = self.ivars().popover.borrow().as_ref() {
            return popover.clone();
        }

        let views = self.build_popover_views(mtm);
        let controller = NSViewController::new(mtm);
        controller.setView(&views.stack);

        let popover = NSPopover::new(mtm);
        popover.setContentViewController(Some(&controller));
        // The wrapper owns every close, so AppKit must not take the popover
        // away on the first click elsewhere — a transient popover would also
        // make the status item's own menu impossible to reach.
        popover.setBehavior(NSPopoverBehavior::ApplicationDefined);
        // A hover that reopens the popover must not replay an animation.
        popover.setAnimates(false);
        self.install_hover_tracking(&views.stack);

        *self.ivars().popover_views.borrow_mut() = Some(views);
        *self.ivars().popover.borrow_mut() = Some(popover.clone());
        popover
    }

    fn build_popover_views(&self, mtm: MainThreadMarker) -> PopoverViews {
        let stack = NSStackView::new(mtm);
        stack.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
        stack.setAlignment(NSLayoutAttribute::Leading);
        stack.setSpacing(10.0);
        stack.setEdgeInsets(NSEdgeInsets {
            top: POPOVER_INSET,
            left: POPOVER_INSET,
            bottom: POPOVER_INSET,
            right: POPOVER_INSET,
        });
        stack.setTranslatesAutoresizingMaskIntoConstraints(false);

        let status = popover_label(mtm, NSFont::boldSystemFontOfSize(13.0), None);
        stack.addArrangedSubview(&status);
        pin_width(&status, POPOVER_CONTENT_WIDTH);

        let message = popover_label(
            mtm,
            NSFont::systemFontOfSize(11.0),
            Some(&NSColor::secondaryLabelColor()),
        );
        stack.addArrangedSubview(&message);
        pin_width(&message, POPOVER_CONTENT_WIDTH);

        let rows = (0..shared::POPOVER_ROWS)
            .map(|_| {
                let views = build_popover_row(mtm);
                stack.addArrangedSubview(&views.row);
                pin_width(&views.row, POPOVER_CONTENT_WIDTH);
                views
            })
            .collect();

        PopoverViews {
            stack,
            status,
            message,
            rows,
        }
    }

    /// Draw one fetch result. Rows past the end of the queue are hidden rather
    /// than emptied, so they take no space in the stack.
    fn render_popover(&self, content: &PopoverContent) {
        let views = self.ivars().popover_views.borrow();
        let Some(views) = views.as_ref() else {
            return;
        };

        match content.status.as_deref() {
            Some(status) => {
                views.status.setStringValue(&NSString::from_str(status));
                views.status.setHidden(false);
            }
            None => views.status.setHidden(true),
        }
        match content.message.as_deref() {
            Some(message) => {
                views.message.setStringValue(&NSString::from_str(message));
                views.message.setHidden(false);
            }
            None => views.message.setHidden(true),
        }

        for (index, row) in views.rows.iter().enumerate() {
            match content.rows.get(index) {
                Some(item) => {
                    row.name.setStringValue(&NSString::from_str(&item.name));
                    row.detail
                        .setStringValue(&NSString::from_str(&shared::row_detail(item)));
                    row.progress.setDoubleValue(item.progress_percent);
                    row.row.setHidden(false);
                }
                None => row.row.setHidden(true),
            }
        }

        views.stack.layoutSubtreeIfNeeded();
        let height = views.stack.fittingSize().height;
        if let Some(popover) = self.ivars().popover.borrow().as_ref() {
            popover.setContentSize(NSSize::new(POPOVER_WIDTH, height));
        }
    }

    /// Start asking the server for the queue. Nothing polls while the popover
    /// is closed, so an idle menu bar makes no requests at all.
    fn begin_status_polling(&self) {
        if self.ivars().popover_timer.borrow().is_some() {
            return;
        }
        // The next tick starts a fetch immediately; a hover must not wait out a
        // refresh interval for its first answer.
        self.ivars().status_fetched_at.set(None);
        // SAFETY: The selector is implemented by this class, and the timer is
        // invalidated before the delegate could go away.
        let timer = unsafe {
            NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                POPOVER_POLL_INTERVAL,
                self.as_any(),
                sel!(refreshPopover:),
                None,
                true,
            )
        };
        *self.ivars().popover_timer.borrow_mut() = Some(timer);
    }

    fn stop_status_polling(&self) {
        if let Some(timer) = self.ivars().popover_timer.borrow_mut().take() {
            timer.invalidate();
        }
    }

    /// Watch the supervised server for the whole run.
    ///
    /// Unlike the readiness poll this never stops on its own: the request it
    /// waits for can arrive at any point in a session, hours after the window
    /// came up.
    fn begin_supervision_polling(&self) {
        if self.ivars().supervision_timer.borrow().is_some() {
            return;
        }
        // SAFETY: The selector is implemented by this class, and the timer is
        // invalidated before the delegate could go away.
        let timer = unsafe {
            NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                SUPERVISION_POLL_INTERVAL,
                self.as_any(),
                sel!(pollSupervisedServer:),
                None,
                true,
            )
        };
        *self.ivars().supervision_timer.borrow_mut() = Some(timer);
    }

    fn stop_supervision_polling(&self) {
        if let Some(timer) = self.ivars().supervision_timer.borrow_mut().take() {
            timer.invalidate();
        }
    }

    /// Hand off to a fresh instance of the replaced bundle, and quit.
    ///
    /// The server has already exited by the time this runs — that exit *is* the
    /// request — so `stop` here only reaps the bookkeeping. The bundle path
    /// comes from this process's own location, never from the server, so the
    /// relaunch can only target the application the user started.
    fn relaunch_after_bundle_upgrade(&self) {
        self.stop_supervision_polling();
        self.stop_ready_polling();
        self.hide_popover();
        let _ = self.ivars().supervisor.borrow_mut().stop();

        let Some(bundle) = shared::running_app_bundle() else {
            // Not running from a bundle: there is nothing to relaunch, and the
            // server should never have asked. Start it again so the user is not
            // left with a wrapper supervising nothing.
            self.report(self.ivars().supervisor.borrow_mut().start());
            self.begin_supervision_polling();
            return;
        };
        if let Err(error) = shared::relaunch_app_bundle(&bundle) {
            self.report(Err(format!(
                "Scryer was updated, but the updated app could not be started: {error}\n\n\
                 Open {} yourself to finish.",
                bundle.display()
            )));
            return;
        }
        self.ivars().quit_confirmed.set(true);
        NSApplication::sharedApplication(self.mtm()).terminate(None);
    }

    /// Fetch off the main thread. A menu bar that stalls while a socket times
    /// out is worse than a popover that fills in a moment late.
    fn spawn_status_fetch(&self) {
        let port = self.ivars().supervisor.borrow().port();
        let result = Arc::clone(&self.ivars().status_result);
        let fetching = Arc::clone(&self.ivars().status_fetching);

        fetching.store(true, Ordering::Release);
        self.ivars().status_fetched_at.set(Some(Instant::now()));
        std::thread::spawn(move || {
            let content = shared::fetch_popover_content(port);
            *result
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(content);
            // Released after the result is visible, so the timer never starts a
            // second fetch over an answer it has not drawn yet.
            fetching.store(false, Ordering::Release);
        });
    }

    fn load_app(&self) {
        let Some(url) = NSURL::URLWithString(&NSString::from_str(&self.ivars().url)) else {
            self.load_html(&startup_failure_html());
            return;
        };
        let request = NSURLRequest::requestWithURL(&url);
        if let Some(webview) = self.ivars().webview.borrow().as_ref() {
            // SAFETY: The webview belongs to this delegate and is used only on
            // the main thread.
            unsafe { webview.loadRequest(&request) };
            self.ivars().showing_app.set(true);
        }
    }

    fn load_html(&self, html: &str) {
        if let Some(webview) = self.ivars().webview.borrow().as_ref() {
            // SAFETY: The webview belongs to this delegate and is used only on
            // the main thread.
            unsafe { webview.loadHTMLString_baseURL(&NSString::from_str(html), None) };
        }
    }

    /// Whether a navigation should be handed to the user's browser.
    ///
    /// Only main-frame navigations are considered: an iframe or a subresource
    /// pointing somewhere else is the page doing its job, not the user
    /// following a link out of the app.
    fn leaves_the_app(&self, navigation_action: &WKNavigationAction) -> bool {
        // SAFETY: These are the action's own properties, read on the thread
        // WebKit called us on.
        let is_main_frame = unsafe {
            navigation_action
                .targetFrame()
                .is_some_and(|frame| frame.isMainFrame())
        };
        if !is_main_frame {
            return false;
        }
        // SAFETY: As above.
        let Some(url) = (unsafe { navigation_action.request().URL() }) else {
            return false;
        };
        // SAFETY: As above.
        let Some(url) = url.absoluteString() else {
            return false;
        };
        shared::opens_in_external_browser(&self.ivars().origin, &url.to_string())
    }

    fn report(&self, result: Result<(), String>) {
        if let Err(error) = result {
            show_error_on_main_thread(self.mtm(), "Scryer", &error);
        }
    }

    fn as_any(&self) -> &AnyObject {
        // SAFETY: Every Objective-C object is an `AnyObject`.
        unsafe { &*(self as *const Self).cast::<AnyObject>() }
    }
}

/// Build the menu-bar glyph.
///
/// A drawing handler rather than a static image: AppKit runs it with the menu
/// bar's own appearance current, and re-runs it when that appearance changes,
/// so the glyph follows the menu bar without any observation of our own.
fn menu_bar_icon() -> Option<Retained<NSImage>> {
    let size = NSSize::new(MENU_BAR_ICON_POINTS, MENU_BAR_ICON_POINTS);
    let for_light_bar = variant_image(size, MENU_BAR_ICON_LIGHT, MENU_BAR_ICON_LIGHT_2X)?;
    let for_dark_bar = variant_image(size, MENU_BAR_ICON_DARK, MENU_BAR_ICON_DARK_2X)?;
    let handler = RcBlock::new(move |rect: NSRect| {
        let chosen = if appearance_is_dark(&NSAppearance::currentDrawingAppearance()) {
            &for_dark_bar
        } else {
            &for_light_bar
        };
        chosen.drawInRect(rect);
        Bool::YES
    });
    Some(NSImage::imageWithSize_flipped_drawingHandler(
        size, false, &handler,
    ))
}

/// One appearance's drawing, from both shipped scales.
///
/// Each representation is declared at the same point size, which is what tells
/// AppKit the 36-pixel asset is the 18-point glyph on a Retina display rather
/// than a 36-point glyph.
fn variant_image(size: NSSize, base: &[u8], retina: &[u8]) -> Option<Retained<NSImage>> {
    let image = NSImage::initWithSize(NSImage::alloc(), size);
    for bytes in [base, retina] {
        let representation = NSBitmapImageRep::imageRepWithData(&NSData::with_bytes(bytes))?;
        representation.setSize(size);
        image.addRepresentation(&representation);
    }
    image.setSize(size);
    Some(image)
}

/// A non-editable, non-drawn text field: the popover shows text, it does not
/// collect any.
fn popover_label(
    mtm: MainThreadMarker,
    font: Retained<NSFont>,
    color: Option<&NSColor>,
) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&NSString::from_str(""), mtm);
    label.setFont(Some(&font));
    if let Some(color) = color {
        label.setTextColor(Some(color));
    }
    label.setMaximumNumberOfLines(1);
    label.setUsesSingleLineMode(true);
    // Release names differ at the end, not the start, so the middle is the
    // part worth losing.
    label.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
    label
}

fn build_popover_row(mtm: MainThreadMarker) -> PopoverRowViews {
    let row = NSStackView::new(mtm);
    row.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
    row.setAlignment(NSLayoutAttribute::Leading);
    row.setSpacing(3.0);

    let name = popover_label(mtm, NSFont::systemFontOfSize(13.0), None);
    row.addArrangedSubview(&name);
    pin_width(&name, POPOVER_CONTENT_WIDTH);

    let progress = NSProgressIndicator::new(mtm);
    progress.setStyle(NSProgressIndicatorStyle::Bar);
    progress.setControlSize(NSControlSize::Small);
    progress.setIndeterminate(false);
    progress.setMinValue(0.0);
    progress.setMaxValue(100.0);
    row.addArrangedSubview(&progress);
    pin_width(&progress, POPOVER_CONTENT_WIDTH);

    let detail = popover_label(
        mtm,
        NSFont::systemFontOfSize(11.0),
        Some(&NSColor::secondaryLabelColor()),
    );
    row.addArrangedSubview(&detail);
    pin_width(&detail, POPOVER_CONTENT_WIDTH);

    PopoverRowViews {
        row,
        name,
        progress,
        detail,
    }
}

/// Every row spans the popover, so each one is pinned rather than left to an
/// intrinsic width the queue's own titles would otherwise decide.
fn pin_width(view: &NSView, width: f64) {
    view.widthAnchor()
        .constraintEqualToConstant(width)
        .setActive(true);
}

/// Scryer's `--background` token for the current system appearance
/// (apps/scryer-web/app/globals.css): #070d1c in dark mode, #ffffff in light.
/// The web UI follows the same switch through `prefers-color-scheme`.
fn theme_background_color(mtm: MainThreadMarker) -> Retained<NSColor> {
    let app = NSApplication::sharedApplication(mtm);
    let dark = appearance_is_dark(&app.effectiveAppearance());
    let (red, green, blue) = if dark {
        (0x07, 0x0d, 0x1c)
    } else {
        (0xff, 0xff, 0xff)
    };
    NSColor::colorWithSRGBRed_green_blue_alpha(
        f64::from(red) / 255.0,
        f64::from(green) / 255.0,
        f64::from(blue) / 255.0,
        1.0,
    )
}

/// The main menu exists for its key equivalents: without one, Cmd+Q, Cmd+W
/// and the standard editing shortcuts reach nothing. Every item routes
/// through the responder chain (no target), so the webview keeps its own
/// editing behaviour and `terminate:` still funnels through the delegate's
/// quit prompt.
fn install_main_menu(mtm: MainThreadMarker) {
    let main_menu = NSMenu::new(mtm);

    let app_menu = NSMenu::new(mtm);
    app_menu.addItem(&key_item(mtm, "Quit Scryer", sel!(terminate:), "q"));
    main_menu.addItem(&submenu_item(mtm, "Scryer", &app_menu));

    let file_menu = NSMenu::new(mtm);
    file_menu.addItem(&key_item(mtm, "Close Window", sel!(performClose:), "w"));
    main_menu.addItem(&submenu_item(mtm, "File", &file_menu));

    let edit_menu = NSMenu::new(mtm);
    edit_menu.addItem(&key_item(mtm, "Undo", sel!(undo:), "z"));
    // An upper-case key equivalent is how AppKit spells Cmd+Shift.
    edit_menu.addItem(&key_item(mtm, "Redo", sel!(redo:), "Z"));
    edit_menu.addItem(&NSMenuItem::separatorItem(mtm));
    edit_menu.addItem(&key_item(mtm, "Cut", sel!(cut:), "x"));
    edit_menu.addItem(&key_item(mtm, "Copy", sel!(copy:), "c"));
    edit_menu.addItem(&key_item(mtm, "Paste", sel!(paste:), "v"));
    edit_menu.addItem(&key_item(mtm, "Select All", sel!(selectAll:), "a"));
    main_menu.addItem(&submenu_item(mtm, "Edit", &edit_menu));

    NSApplication::sharedApplication(mtm).setMainMenu(Some(&main_menu));
}

/// A first-responder-targeted menu item with a Cmd key equivalent.
fn key_item(mtm: MainThreadMarker, title: &str, action: Sel, key: &str) -> Retained<NSMenuItem> {
    let title = NSString::from_str(title);
    let key = NSString::from_str(key);
    // SAFETY: The selectors are AppKit's own standard actions; with no target
    // set they resolve through the responder chain at dispatch time.
    unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &title,
            Some(action),
            &key,
        )
    }
}

/// A titled item carrying a submenu; the title is what the menu bar shows.
fn submenu_item(mtm: MainThreadMarker, title: &str, submenu: &NSMenu) -> Retained<NSMenuItem> {
    let title = NSString::from_str(title);
    let key = NSString::from_str("");
    // SAFETY: An item with no action is a pure submenu holder.
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm), &title, None, &key)
    };
    submenu.setTitle(&title);
    item.setSubmenu(Some(submenu));
    item
}

/// Whether an appearance resolves to the dark family.
fn appearance_is_dark(appearance: &NSAppearance) -> bool {
    // SAFETY: The appearance statics are plain constants, and matching names
    // has no precondition; every caller is on the main thread.
    unsafe {
        let names = NSArray::from_slice(&[NSAppearanceNameAqua, NSAppearanceNameDarkAqua]);
        appearance
            .bestMatchFromAppearancesWithNames(&names)
            .is_some_and(|name| name.isEqualToString(NSAppearanceNameDarkAqua))
    }
}

fn build_webview(mtm: MainThreadMarker, frame: NSRect) -> Retained<WKWebView> {
    // SAFETY: Both objects are created on the main thread and are used only
    // from it afterwards.
    unsafe {
        let configuration = WKWebViewConfiguration::new(mtm);
        WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), frame, &configuration)
    }
}

/// Shown while the server starts. Inline and asset-free on purpose: it has to
/// render before anything is listening on the port.
fn splash_html() -> String {
    document_html(
        "Starting Scryer…",
        "The desktop app is starting the Scryer server. This window will load the interface as soon as it answers.",
    )
}

fn startup_failure_html() -> String {
    document_html(
        "Scryer did not start",
        "The Scryer server did not answer in time. Choose Open Logs from the Scryer menu-bar item to see why, then choose Restart Scryer.",
    )
}

fn document_html(heading: &str, body: &str) -> String {
    format!(
        "<!doctype html><meta charset=\"utf-8\">\
         <meta name=\"color-scheme\" content=\"dark light\">\
         <title>Scryer</title>\
         <style>\
         html,body{{height:100%;margin:0}}\
         body{{background:#070d1c;color:#dbe5ff;\
         font:15px/1.6 -apple-system,BlinkMacSystemFont,'Helvetica Neue',sans-serif;\
         display:flex;align-items:center;justify-content:center;text-align:center}}\
         main{{max-width:34rem;padding:2rem}}\
         h1{{font-size:1.25rem;font-weight:600;margin:0 0 .75rem}}\
         p{{margin:0;color:#a9b0bd}}\
         </style>\
         <main><h1>{heading}</h1><p>{body}</p></main>"
    )
}

fn open_external(url: &str) -> Result<(), String> {
    let Some(target) = NSURL::URLWithString(&NSString::from_str(url)) else {
        return Err(format!("{url} is not a URL macOS can open"));
    };
    if NSWorkspace::sharedWorkspace().openURL(&target) {
        Ok(())
    } else {
        Err(format!("macOS could not open {url}"))
    }
}

pub(super) fn show_error(title: &str, message: &str) {
    eprintln!("{title}: {message}");
    if let Some(mtm) = MainThreadMarker::new() {
        show_error_on_main_thread(mtm, title, message);
    }
}

fn show_error_on_main_thread(mtm: MainThreadMarker, title: &str, message: &str) {
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(title));
    alert.setInformativeText(&NSString::from_str(message));
    alert.runModal();
}

// ---------------------------------------------------------------------------
// `--webview-smoke`
// ---------------------------------------------------------------------------

/// Prove that this binary can create a WebKit view and load a real network
/// document, without needing Scryer — or any Scryer data — to exist.
///
/// This is the check that would have caught a missing framework, a webview
/// that silently fails to start its content process, or a sandbox that
/// refuses the loopback connection.
fn run_webview_smoke() -> Result<(), String> {
    let port = shared::start_smoke_server()?;
    let url = shared::app_url(port);

    let Some(mtm) = MainThreadMarker::new() else {
        smoke_failure("the smoke test must start on the main thread");
    };
    let app = NSApplication::sharedApplication(mtm);
    // No Dock icon and no activation: this runs on build machines with nobody
    // watching, and it must not steal focus from anything else running there.
    app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);

    let delegate = SmokeDelegate::new(mtm, url);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    app.run();

    // `NSApplication::run` only returns if something stopped it without the
    // smoke reporting a result, which is itself a failure.
    smoke_failure("the smoke test event loop exited before the page loaded");
}

fn smoke_failure(reason: &str) -> ! {
    eprintln!("webview-smoke: {reason}");
    std::process::exit(1);
}

struct SmokeState {
    url: String,
    window: RefCell<Option<Retained<NSWindow>>>,
    webview: RefCell<Option<Retained<WKWebView>>>,
    deadline: Cell<Option<Instant>>,
}

define_class!(
    // SAFETY:
    // - NSObject imposes no subclassing requirements.
    // - The class is main-thread only, as every callback below requires, and
    //   it does not implement Drop.
    #[unsafe(super(objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ScryerMediaDesktopSmokeDelegate"]
    #[ivars = SmokeState]
    struct SmokeDelegate;

    impl SmokeDelegate {
        #[unsafe(method(checkDeadline:))]
        fn check_deadline(&self, _timer: &NSTimer) {
            if let Some(deadline) = self.ivars().deadline.get()
                && Instant::now() >= deadline
            {
                smoke_failure("timed out waiting for the webview to load the test page");
            }
        }
    }

    unsafe impl NSObjectProtocol for SmokeDelegate {}

    unsafe impl NSApplicationDelegate for SmokeDelegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn application_did_finish_launching(&self, _notification: &NSNotification) {
            self.start();
        }
    }

    unsafe impl WKNavigationDelegate for SmokeDelegate {
        #[unsafe(method(webView:didFinishNavigation:))]
        fn did_finish_navigation(&self, _web_view: &WKWebView, _navigation: Option<&WKNavigation>) {
            println!("{SMOKE_SUCCESS_LINE}");
            std::process::exit(0);
        }

        #[unsafe(method(webView:didFailNavigation:withError:))]
        fn did_fail_navigation(
            &self,
            _web_view: &WKWebView,
            _navigation: Option<&WKNavigation>,
            error: &objc2_foundation::NSError,
        ) {
            smoke_failure(&format!("navigation failed: {}", error.localizedDescription()));
        }

        #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
        fn did_fail_provisional_navigation(
            &self,
            _web_view: &WKWebView,
            _navigation: Option<&WKNavigation>,
            error: &objc2_foundation::NSError,
        ) {
            smoke_failure(&format!(
                "provisional navigation failed: {}",
                error.localizedDescription()
            ));
        }
    }
);

impl SmokeDelegate {
    fn new(mtm: MainThreadMarker, url: String) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(SmokeState {
            url,
            window: RefCell::new(None),
            webview: RefCell::new(None),
            deadline: Cell::new(None),
        });
        // SAFETY: `init` on the NSObject superclass has no further requirements.
        unsafe { msg_send![super(this), init] }
    }

    fn start(&self) {
        let mtm = self.mtm();
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(800.0, 600.0));
        // The window is never ordered in. WebKit needs a window to host the
        // view and to run its content process, but the navigation completes
        // whether or not anything is on screen — which is what makes this
        // runnable on a build machine.
        // SAFETY: Created and used only on the main thread.
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                frame,
                NSWindowStyleMask::Borderless,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        // SAFETY: The window is retained by this delegate for the run.
        unsafe { window.setReleasedWhenClosed(false) };

        let webview = build_webview(mtm, frame);
        // SAFETY: The delegate is this object, which outlives the webview.
        unsafe { webview.setNavigationDelegate(Some(ProtocolObject::from_ref(self))) };
        window.setContentView(Some(&webview));

        let Some(url) = NSURL::URLWithString(&NSString::from_str(&self.ivars().url)) else {
            smoke_failure("the smoke test URL could not be parsed");
        };
        let request = NSURLRequest::requestWithURL(&url);
        // SAFETY: The webview was just created on this thread.
        unsafe { webview.loadRequest(&request) };

        self.ivars()
            .deadline
            .set(Some(Instant::now() + SMOKE_TIMEOUT));
        *self.ivars().webview.borrow_mut() = Some(webview);
        *self.ivars().window.borrow_mut() = Some(window);

        // A repeating timer rather than a one-shot at the deadline, so a
        // wedged WebKit process cannot also wedge the watchdog.
        // SAFETY: The selector is implemented by this class, which outlives
        // the timer.
        unsafe {
            NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                WATCHDOG_INTERVAL,
                self.as_any(),
                sel!(checkDeadline:),
                None,
                true,
            );
        }
    }

    fn as_any(&self) -> &AnyObject {
        // SAFETY: Every Objective-C object is an `AnyObject`.
        unsafe { &*(self as *const Self).cast::<AnyObject>() }
    }
}

/// How often the smoke watchdog checks the clock.
const WATCHDOG_INTERVAL: f64 = 1.0;

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{SMOKE_TIMEOUT, document_html, splash_html, startup_failure_html};

    #[test]
    fn the_inline_pages_reference_no_external_assets() {
        for page in [splash_html(), startup_failure_html()] {
            assert!(!page.contains("http://"), "{page}");
            assert!(!page.contains("https://"), "{page}");
            assert!(!page.contains("<script"), "{page}");
        }
    }

    #[test]
    fn inline_pages_carry_their_own_text() {
        let page = document_html("Heading", "Body copy.");
        assert!(page.contains("<h1>Heading</h1>"));
        assert!(page.contains("<p>Body copy.</p>"));
    }

    #[test]
    fn the_smoke_watchdog_outlasts_a_cold_webkit_launch() {
        assert!(SMOKE_TIMEOUT >= Duration::from_secs(60));
    }
}
