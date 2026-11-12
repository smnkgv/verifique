#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::env;
use std::fs::{self, File, TryLockError};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, LogicalSize, Manager, PhysicalPosition, WebviewWindow, WindowEvent, Wry};

/// Clicking the icon while the panel is open first takes focus from it, which hides it. Without this
/// gap the same click would open it again straight away.
const REOPEN_GAP: Duration = Duration::from_millis(250);
const GAP_BELOW_ICON: f64 = 6.0;
#[cfg(target_os = "macos")]
const BLUR_STRENGTH: f64 = 0.72;
/// How far the panel keeps from the edges of the screen where it would otherwise run off them.
#[cfg(not(target_os = "macos"))]
const SCREEN_MARGIN: f64 = 8.0;
/// The idle tick's colour where the tray shows icons as they are, on light and dark bars alike.
const IDLE_GREY: [u8; 3] = [0x90, 0x90, 0x90];
/// The dot that says an update is waiting: blue, the usual sign of something new, which reads on
/// every colour of tick and never as something needing you.
const UPDATE_BLUE: [u8; 3] = [0x0a, 0x84, 0xff];

struct PinItem(CheckMenuItem<Wry>);

const IDLE_ICON: &[u8] = include_bytes!("../icons/tray.png");
const WORKING_ICON: &[u8] = include_bytes!("../icons/tray-working.png");
const PASSED_ICON: &[u8] = include_bytes!("../icons/tray-passed.png");
const NEEDS_ICON: &[u8] = include_bytes!("../icons/tray-needs.png");

static LOOK: Mutex<String> = Mutex::new(String::new());
static PINNED: AtomicBool = AtomicBool::new(false);
static HIDDEN_AT: Mutex<Option<Instant>> = Mutex::new(None);
/// Where the panel's bottom edge stays while it is open above its icon, as it is over a taskbar at
/// the bottom of the screen, so that growing with its content moves its top rather than its bottom.
#[cfg(not(target_os = "macos"))]
static BOTTOM: Mutex<Option<f64>> = Mutex::new(None);
/// The height the page last asked for, which the window can't be relied on to know before it first shows.
#[cfg(not(target_os = "macos"))]
static HEIGHT: Mutex<f64> = Mutex::new(480.0);
/// Set when the panel chose GDK's backend itself, so the programs it opens don't inherit the choice.
#[cfg(target_os = "linux")]
static CHOSE_X11: AtomicBool = AtomicBool::new(false);

/// A pinned panel stays on screen in the corner, on every desktop the OS lets it, and survives a restart.
fn pinned() -> bool {
    PINNED.load(Ordering::Relaxed)
}

fn pin_file() -> PathBuf {
    home().join("panel-pinned")
}

fn show_pinned(window: &WebviewWindow) {
    let _ = window.set_visible_on_all_workspaces(true);
    #[cfg(target_os = "macos")]
    blur_behind(window);
    #[cfg(not(target_os = "macos"))]
    {
        *BOTTOM.lock().unwrap() = None;
    }
    if let Ok(Some(monitor)) = window.current_monitor() {
        let scale = monitor.scale_factor();
        let (x, y) = pinned_corner(&monitor, width_on_screen(window, scale), scale);
        let _ = window.set_position(PhysicalPosition::new(x, y));
    }
    let _ = window.show();
}

/// The panel's width on screen. Elsewhere than macOS the window can't be relied on to know it before
/// it first shows, so there it is the width the panel is made at.
fn width_on_screen(window: &WebviewWindow, scale: f64) -> f64 {
    if cfg!(target_os = "macos") {
        window.outer_size().map(|size| size.width as f64).unwrap_or(360.0 * scale)
    } else {
        360.0 * scale
    }
}

/// The top right corner, below the menu bar.
#[cfg(target_os = "macos")]
fn pinned_corner(monitor: &tauri::Monitor, width: f64, scale: f64) -> (f64, f64) {
    let x = monitor.position().x as f64 + monitor.size().width as f64 - width - 16.0 * scale;
    let y = monitor.position().y as f64 + 36.0 * scale;
    (x, y)
}

/// The top right corner of the part of the screen that taskbars and panels leave free.
#[cfg(not(target_os = "macos"))]
fn pinned_corner(monitor: &tauri::Monitor, width: f64, scale: f64) -> (f64, f64) {
    let free = monitor.work_area();
    let x = free.position.x as f64 + free.size.width as f64 - width - 16.0 * scale;
    let y = free.position.y as f64 + 16.0 * scale;
    (x, y)
}

#[cfg(target_os = "macos")]
fn blur_behind(window: &WebviewWindow) {
    use tauri::window::{Effect, EffectState, EffectsBuilder};
    // Active, because a pinned panel is rarely the focused window and macOS drops the blur otherwise.
    let blur = EffectsBuilder::new().effect(Effect::HudWindow).state(EffectState::Active).radius(12.0).build();
    let _ = window.set_effects(blur);
    let handle = window.clone();
    let _ = window.run_on_main_thread(move || soften_blur(&handle));
}

/// macOS's blur is nearly opaque and takes no strength setting, so the blur view is faded instead,
/// which lets some of what's behind the panel through.
#[cfg(target_os = "macos")]
fn soften_blur(window: &WebviewWindow) {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};
    let Ok(ns_window) = window.ns_window() else { return };
    let Some(blur) = AnyClass::get(c"NSVisualEffectView") else { return };
    // SAFETY: called on the main thread with the live window Tauri handed over, using only NSWindow
    // and NSView messages that exist on every supported macOS.
    unsafe {
        let ns_window = ns_window as *mut AnyObject;
        let content: *mut AnyObject = msg_send![ns_window, contentView];
        let subviews: *mut AnyObject = msg_send![content, subviews];
        let count: usize = msg_send![subviews, count];
        for index in 0..count {
            let view: *mut AnyObject = msg_send![subviews, objectAtIndex: index];
            let is_blur: bool = msg_send![view, isKindOfClass: blur];
            if is_blur {
                let _: () = msg_send![view, setAlphaValue: BLUR_STRENGTH];
            }
        }
    }
}

/// A full-screen window gets a desktop of its own, where macOS shows another app's window only if it
/// is a panel that doesn't make its app the active one, as menu bar apps' popovers are. Otherwise
/// clicking the icon there would open the panel on another desktop, out of sight. So the panel's
/// window becomes such a panel, which joins every desktop, full-screen ones included, open or pinned.
#[cfg(target_os = "macos")]
fn make_panel(window: &WebviewWindow) {
    use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel};
    use objc2::{msg_send, sel};
    use std::sync::OnceLock;
    const NONACTIVATING_PANEL: usize = 1 << 7;
    const CAN_JOIN_ALL_SPACES: usize = 1 << 0;
    const FULL_SCREEN_AUXILIARY: usize = 1 << 8;
    static PANEL: OnceLock<Option<&'static AnyClass>> = OnceLock::new();
    extern "C-unwind" fn yes(_: &AnyObject, _: Sel) -> Bool {
        Bool::YES
    }
    let panel = PANEL.get_or_init(|| {
        let mut class = ClassBuilder::new(c"VerifiquePanel", AnyClass::get(c"NSPanel")?)?;
        // SAFETY: the method has the signature AppKit expects of canBecomeKeyWindow.
        unsafe { class.add_method(sel!(canBecomeKeyWindow), yes as extern "C-unwind" fn(_, _) -> _) };
        Some(class.register())
    });
    let (Some(panel), Ok(ns_window)) = (panel, window.ns_window()) else { return };
    // SAFETY: called on the main thread with the live window Tauri handed over. An NSPanel is an
    // NSWindow of the same size that adds no state of its own, so the window can take its class;
    // the only overrides of Tauri's that are lost are for windows dragged by their background.
    unsafe {
        let ns_window = &*(ns_window as *mut AnyObject);
        AnyObject::set_class(ns_window, panel);
        let style: usize = msg_send![ns_window, styleMask];
        let _: () = msg_send![ns_window, setStyleMask: style | NONACTIVATING_PANEL];
        let _: () = msg_send![ns_window, setHidesOnDeactivate: false];
        let behaviour: usize = msg_send![ns_window, collectionBehavior];
        let _: () = msg_send![ns_window, setCollectionBehavior: behaviour | CAN_JOIN_ALL_SPACES | FULL_SCREEN_AUXILIARY];
    }
}

/// Clicking another window takes the keyboard from the panel, which closes it. A panel doesn't make
/// its app the front one, though, so switching to another app without a click, as Command-Tab does,
/// would leave it open, and one that joins every desktop would follow along. So it also closes
/// whenever another app comes to the front.
#[cfg(target_os = "macos")]
fn close_when_another_app_comes_forward(app: &AppHandle) {
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, ClassBuilder, Sel};
    use objc2::{msg_send, sel};
    use std::sync::OnceLock;
    static APP: OnceLock<AppHandle> = OnceLock::new();
    fn string(text: &std::ffi::CStr) -> *mut AnyObject {
        let Some(class) = AnyClass::get(c"NSString") else { return std::ptr::null_mut() };
        // SAFETY: stringWithUTF8String: takes a NUL-terminated UTF-8 string, which a CStr is.
        unsafe { msg_send![class, stringWithUTF8String: text.as_ptr()] }
    }
    extern "C-unwind" fn came_forward(_: &AnyObject, _: Sel, notification: *mut AnyObject) {
        // SAFETY: NSWorkspace passes a live notification whose user info names the app that came
        // forward, and messages to nil return 0.
        let pid: i32 = unsafe {
            let info: *mut AnyObject = msg_send![notification, userInfo];
            let app: *mut AnyObject = msg_send![info, objectForKey: string(c"NSWorkspaceApplicationKey")];
            msg_send![app, processIdentifier]
        };
        if pid == 0 || pid as u32 == std::process::id() {
            return;
        }
        let Some(window) = APP.get().and_then(|app| app.get_webview_window("panel")) else { return };
        if !pinned() && window.is_visible().unwrap_or(false) {
            *HIDDEN_AT.lock().unwrap() = Some(Instant::now());
            let _ = window.hide();
        }
    }
    if APP.set(app.clone()).is_err() {
        return;
    }
    let (Some(object), Some(workspace)) = (AnyClass::get(c"NSObject"), AnyClass::get(c"NSWorkspace")) else { return };
    let Some(mut class) = ClassBuilder::new(c"VerifiquePanelWatcher", object) else { return };
    // SAFETY: the method has the signature NSNotificationCenter expects of an observer's selector.
    unsafe { class.add_method(sel!(appCameForward:), came_forward as extern "C-unwind" fn(_, _, _)) };
    let class = class.register();
    // SAFETY: called on the main thread at startup, with messages AppKit has on every supported
    // macOS. The watcher is never released, so the centre never calls one that is gone.
    unsafe {
        let watcher: Retained<AnyObject> = msg_send![class, new];
        let workspace: *mut AnyObject = msg_send![workspace, sharedWorkspace];
        let centre: *mut AnyObject = msg_send![workspace, notificationCenter];
        let name = string(c"NSWorkspaceDidActivateApplicationNotification");
        let _: () = msg_send![centre, addObserver: &*watcher, selector: sel!(appCameForward:), name: name, object: std::ptr::null_mut::<AnyObject>()];
        let _ = Retained::into_raw(watcher);
    }
}

fn change_pin(app: &AppHandle, on: bool) {
    PINNED.store(on, Ordering::Relaxed);
    let _ = if on {
        pin_file().parent().map(fs::create_dir_all);
        fs::write(pin_file(), "")
    } else {
        fs::remove_file(pin_file())
    };
    let _ = app.state::<PinItem>().0.set_checked(on);
    let Some(window) = app.get_webview_window("panel") else { return };
    if on {
        show_pinned(&window);
    } else {
        #[cfg(not(target_os = "macos"))]
        let _ = window.set_visible_on_all_workspaces(false);
        #[cfg(target_os = "macos")]
        let _ = window.set_effects(None::<tauri::utils::config::WindowEffectsConfig>);
        let _ = window.hide();
    }
}

#[tauri::command]
fn is_pinned() -> bool {
    pinned()
}

#[tauri::command]
fn set_pinned(app: AppHandle, on: bool) {
    change_pin(&app, on);
}

/// Whether the OS blurs what's behind a pinned panel. Where it doesn't, the page is less see-through,
/// so that it stays readable over whatever is behind it.
#[tauri::command]
fn blurs_behind() -> bool {
    cfg!(target_os = "macos")
}

/// The user's own folder: HOME on macOS and Linux, the profile folder on Windows.
fn user_home() -> PathBuf {
    env::home_dir().unwrap_or_default()
}

/// The queue's folder, wherever the CLI keeps it.
fn home() -> PathBuf {
    env::var_os("VERIFIQUE_HOME").map_or_else(|| user_home().join(".verifique"), PathBuf::from)
}

/// An app started from the desktop may have only a short PATH, so look where the install script puts
/// the command first, then on the PATH, then where Homebrew puts it.
fn verifique() -> PathBuf {
    let name = if cfg!(windows) { "verifique.exe" } else { "verifique" };
    let installed = if cfg!(windows) {
        env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_default().join("Programs").join("verifique")
    } else {
        user_home().join(".local").join("bin")
    };
    let path = env::var_os("PATH").map(|path| env::split_paths(&path).collect::<Vec<_>>()).unwrap_or_default();
    let homebrew: Vec<PathBuf> = if cfg!(target_os = "macos") {
        vec!["/opt/homebrew/bin".into(), "/usr/local/bin".into()]
    } else if cfg!(target_os = "linux") {
        vec!["/home/linuxbrew/.linuxbrew/bin".into(), user_home().join(".linuxbrew").join("bin")]
    } else {
        Vec::new()
    };
    std::iter::once(installed)
        .chain(path)
        .chain(homebrew)
        .map(|dir| dir.join(name))
        .find(|file| file.is_file())
        .unwrap_or_else(|| PathBuf::from(name))
}

/// A program the panel starts, which never opens a window of its own for a console.
fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[cfg_attr(target_os = "macos", allow(unused_mut))]
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(target_os = "linux")]
    if CHOSE_X11.load(Ordering::Relaxed) {
        command.env_remove("GDK_BACKEND");
    }
    command
}

fn cli(args: &[&str], cwd: Option<&str>) -> Result<String, String> {
    let mut command = command(verifique());
    command.args(args);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let out = command.output().map_err(|error| error.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

#[tauri::command]
fn status() -> Result<String, String> {
    cli(&["status", "--json"], None)
}

/// Whether a newer verifique is out, or an update still needs finishing, as the CLI tells it.
#[tauri::command]
fn updates() -> Result<String, String> {
    cli(&["updates", "--json"], None)
}

/// Updates verifique, the queue, its add-ons and this panel, once the person has agreed in the panel.
/// It takes a while, so it runs off the thread that draws the panel.
#[tauri::command]
async fn update() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(|| cli(&["update", "--yes"], None)).await.map_err(|error| error.to_string())?
}

/// Opens a release's page in the browser. Only verifique's own releases.
#[tauri::command]
fn open_release(url: String) -> Result<(), String> {
    if !url.starts_with("https://github.com/smnkgv/verifique/releases/") {
        return Err("that isn't a verifique release".to_string());
    }
    #[cfg(target_os = "macos")]
    let mut opener = command("open");
    #[cfg(target_os = "linux")]
    let mut opener = command("xdg-open");
    #[cfg(windows)]
    let mut opener = command("explorer");
    opener.arg(url).status().map(|_| ()).map_err(|error| error.to_string())
}

#[tauri::command]
fn proof(job: i64) -> Result<String, String> {
    cli(&["show", &job.to_string(), "--json"], None)
}

#[tauri::command]
fn run_next(job: i64) -> Result<String, String> {
    cli(&["next", &job.to_string()], None)
}

#[tauri::command]
fn set_slots(count: usize) -> Result<String, String> {
    cli(&["slots", &count.to_string()], None)
}

#[tauri::command]
fn hand_back(repo: String, branch: String) -> Result<String, String> {
    cli(&["clear", "--branch", &branch], Some(&repo))
}

/// Opens a file or folder the way the desktop would.
#[tauri::command]
fn open_path(path: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut opener = command("open");
    #[cfg(target_os = "linux")]
    let mut opener = command("xdg-open");
    // Explorer reads a forward slash as the start of an option.
    #[cfg(windows)]
    let (mut opener, path) = (command("explorer"), path.replace('/', "\\"));
    opener.arg(path).status().map(|_| ()).map_err(|error| error.to_string())
}

/// A screenshot from a check's evidence, as a data URL. Only files under the job folders can be read.
#[tauri::command]
fn evidence_image(path: String) -> Result<String, String> {
    let file = PathBuf::from(&path).canonicalize().map_err(|error| error.to_string())?;
    let jobs = home().join("jobs").canonicalize().map_err(|error| error.to_string())?;
    if !file.starts_with(jobs) {
        return Err("that file isn't part of a check's evidence".to_string());
    }
    let bytes = std::fs::read(&file).map_err(|error| error.to_string())?;
    let kind = if path.to_lowercase().ends_with(".png") { "png" } else { "jpeg" };
    Ok(format!("data:image/{kind};base64,{}", STANDARD.encode(bytes)))
}

#[tauri::command]
fn open_logs() -> Result<(), String> {
    open_path(home().join("server.log").display().to_string())
}

/// macOS recolours the black idle tick to suit the menu bar. Trays elsewhere show an icon as it is,
/// on light and dark bars alike, so there the tick is a grey that reads on both.
fn idle_icon() -> tauri::Result<Image<'static>> {
    let icon = Image::from_bytes(IDLE_ICON)?;
    #[cfg(not(target_os = "macos"))]
    let icon = grey(icon);
    Ok(icon)
}

/// The idle tick in a grey that reads on light and dark bars, for where it can't follow the bar.
fn grey(icon: Image<'_>) -> Image<'static> {
    let mut rgba = icon.rgba().to_vec();
    for pixel in rgba.as_chunks_mut::<4>().0 {
        pixel[..3].copy_from_slice(&IDLE_GREY);
    }
    Image::new_owned(rgba, icon.width(), icon.height())
}

/// A blue dot at the icon's bottom right, which the tick leaves empty, while an update is waiting.
fn with_dot(icon: Image<'_>) -> Image<'static> {
    const SAMPLES: u32 = 4;
    let (width, height) = (icon.width(), icon.height());
    let mut rgba = icon.rgba().to_vec();
    let size = f64::from(width);
    let (centre, radius) = (size * 0.82, size * 0.15);
    for (index, pixel) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (x, y) = ((index as u32 % width) as f64, (index as u32 / width) as f64);
        // How much of the pixel the dot covers, so its edge is smooth.
        let covered = (0..SAMPLES * SAMPLES)
            .filter(|sample| {
                let at_x = x + (f64::from(sample % SAMPLES) + 0.5) / f64::from(SAMPLES);
                let at_y = y + (f64::from(sample / SAMPLES) + 0.5) / f64::from(SAMPLES);
                (at_x - centre).hypot(at_y - centre) <= radius
            })
            .count() as f64
            / f64::from(SAMPLES * SAMPLES);
        let kept = f64::from(pixel[3]) / 255.0 * (1.0 - covered);
        let alpha = kept + covered;
        if alpha > 0.0 {
            for channel in 0..3 {
                pixel[channel] = ((f64::from(pixel[channel]) * kept + f64::from(UPDATE_BLUE[channel]) * covered) / alpha).round() as u8;
            }
        }
        pixel[3] = (alpha * 255.0).round() as u8;
    }
    Image::new_owned(rgba, width, height)
}

/// Windows shows no text beside a tray icon, so there the count is in the icon's tooltip, with what
/// an update waiting needs.
#[cfg(windows)]
fn tooltip(look: &str, count: usize, update: &str) -> String {
    let state = tooltip_state(look, count);
    // The tray cuts a tooltip off at 127 characters.
    if update.is_empty() { state } else { format!("{state}\n{update}").chars().take(127).collect() }
}

#[cfg(windows)]
fn tooltip_state(look: &str, count: usize) -> String {
    match (look, count) {
        ("needs", 0) => "verifique: can't read the queue".to_string(),
        ("needs", 1) => "verifique: 1 needs you".to_string(),
        ("needs", count) => format!("verifique: {count} need you"),
        ("working", 1) => "verifique: 1 check left".to_string(),
        ("working", count) => format!("verifique: {count} checks left"),
        ("passed", _) => "verifique: passed".to_string(),
        _ => "verifique".to_string(),
    }
}

/// Amber while checks run, green when they have just passed, and red when something needs you. The
/// number is how many checks are left, or how many need you. `update` is the panel's line about a
/// newer verifique, or empty, and puts a blue dot on the tick while there is one. macOS colours a
/// template icon all one colour, so there the idle tick turns grey while the dot shows.
#[tauri::command]
fn show_progress(app: AppHandle, look: String, count: usize, update: String) -> Result<(), String> {
    let tray = app.tray_by_id("main").ok_or("no tray icon")?;
    tray.set_title((count > 0).then(|| count.to_string())).map_err(|error| error.to_string())?;
    #[cfg(windows)]
    tray.set_tooltip(Some(tooltip(&look, count, &update))).map_err(|error| error.to_string())?;
    let wanted = format!("{look}{}", if update.is_empty() { "" } else { " with a dot" });
    let mut shown = LOOK.lock().map_err(|error| error.to_string())?;
    if *shown == wanted {
        return Ok(());
    }
    let (icon, template) = match look.as_str() {
        "needs" => (Image::from_bytes(NEEDS_ICON), false),
        "working" => (Image::from_bytes(WORKING_ICON), false),
        "passed" => (Image::from_bytes(PASSED_ICON), false),
        _ if !update.is_empty() => (Image::from_bytes(IDLE_ICON).map(grey), false),
        _ => (idle_icon(), true),
    };
    let icon = icon.map_err(|error| error.to_string())?;
    let icon = if update.is_empty() { icon } else { with_dot(icon) };
    tray.set_icon_with_as_template(Some(icon), template).map_err(|error| error.to_string())?;
    *shown = wanted;
    Ok(())
}

#[tauri::command]
fn set_height(window: WebviewWindow, height: f64) -> Result<(), String> {
    // GTK won't resize a window the user can't resize, even for the program itself. The panel has no
    // edges to drag, so this only lets it fit its content.
    #[cfg(target_os = "linux")]
    let _ = window.set_resizable(true);
    window.set_size(LogicalSize::new(360.0, height)).map_err(|error| error.to_string())?;
    #[cfg(not(target_os = "macos"))]
    {
        *HEIGHT.lock().unwrap() = height;
    }
    #[cfg(not(target_os = "macos"))]
    if let Some(bottom) = *BOTTOM.lock().unwrap() {
        let scale = window.scale_factor().unwrap_or(1.0);
        let x = window.outer_position().map(|at| at.x as f64).unwrap_or_default();
        window.set_position(PhysicalPosition::new(x, bottom - height * scale)).map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn quit(app: AppHandle) {
    app.exit(0);
}

fn toggle(app: &AppHandle, icon_x: f64, icon_y: f64, icon_w: f64, icon_h: f64) {
    let Some(window) = app.get_webview_window("panel") else { return };
    if pinned() {
        return;
    }
    let closed_just_now = HIDDEN_AT.lock().unwrap().is_some_and(|at| at.elapsed() < REOPEN_GAP);
    if closed_just_now || window.is_visible().unwrap_or(false) {
        let _ = window.hide();
        return;
    }
    open_by(&window, icon_x, icon_y, icon_w, icon_h);
}

/// Opens the panel below the icon, centred on it.
fn open_by(window: &WebviewWindow, icon_x: f64, icon_y: f64, icon_w: f64, icon_h: f64) {
    let scale = window.scale_factor().unwrap_or(1.0);
    let y = icon_y + icon_h + GAP_BELOW_ICON * scale;
    #[cfg(target_os = "macos")]
    let x = icon_x + icon_w / 2.0 - width_on_screen(window, scale) / 2.0;
    #[cfg(not(target_os = "macos"))]
    let (x, y) = keep_on_screen(window, icon_x + icon_w / 2.0, y, icon_y, scale);
    let _ = window.set_position(PhysicalPosition::new(x, y));
    // Showing a panel makes it the window that takes the keyboard. Focusing it as well would make
    // the app the active one, which on macOS takes the panel off a full-screen window's desktop.
    let _ = window.show();
    #[cfg(not(target_os = "macos"))]
    let _ = window.set_focus();
}

/// The menu bar is always at the top on macOS, but a taskbar or panel can be at any edge. Where the
/// panel would run off the bottom of the screen it opens above the icon instead, and it never runs
/// off either side.
#[cfg(not(target_os = "macos"))]
fn keep_on_screen(window: &WebviewWindow, centre: f64, y: f64, icon_y: f64, scale: f64) -> (f64, f64) {
    let mut bottom = BOTTOM.lock().unwrap();
    *bottom = None;
    // The app's screens rather than the window's, which may not know its screen before it first shows.
    let (width, height) = (width_on_screen(window, scale), *HEIGHT.lock().unwrap() * scale);
    let x = centre - width / 2.0;
    let Ok(Some(monitor)) = window.app_handle().monitor_from_point(centre, icon_y) else { return (x, y) };
    let free = monitor.work_area();
    let (left, top) = (free.position.x as f64, free.position.y as f64);
    let (right, floor) = (left + free.size.width as f64, top + free.size.height as f64);
    let margin = SCREEN_MARGIN * scale;
    let x = x.min(right - width - margin).max(left + margin);
    if y + height <= floor {
        return (x, y);
    }
    let above = icon_y - GAP_BELOW_ICON * scale;
    *bottom = Some(above);
    (x, (above - height).max(top))
}

/// Linux trays only show the app's menu, so the panel opens from its menu, next to the pointer that
/// chose it, or in the corner where the pointer can't be read.
#[cfg(target_os = "linux")]
fn open_from_menu(app: &AppHandle) {
    let Some(window) = app.get_webview_window("panel") else { return };
    if pinned() {
        let _ = window.set_focus();
        return;
    }
    if let Ok(at) = app.cursor_position() {
        return open_by(&window, at.x, at.y, 0.0, 0.0);
    }
    *BOTTOM.lock().unwrap() = None;
    if let Ok(Some(monitor)) = window.current_monitor() {
        let scale = monitor.scale_factor();
        let (x, y) = pinned_corner(&monitor, width_on_screen(&window, scale), scale);
        let _ = window.set_position(PhysicalPosition::new(x, y));
    }
    let _ = window.show();
    let _ = window.set_focus();
}

/// Whether the pointer is over the panel. Some X11 window managers take the keyboard for a moment
/// when a window is clicked, which reads as the panel losing focus, but a click on the panel itself is
/// never a reason to hide it.
#[cfg(target_os = "linux")]
fn pointer_over(window: &tauri::Window) -> bool {
    let (Ok(at), Ok(origin), Ok(size)) = (window.cursor_position(), window.outer_position(), window.outer_size()) else { return false };
    let (x, y) = (at.x - origin.x as f64, at.y - origin.y as f64);
    (0.0..size.width as f64).contains(&x) && (0.0..size.height as f64).contains(&y)
}

/// One panel per queue: a second one started by mistake would add a second icon. A lock that can't be
/// taken for any other reason doesn't stop the panel.
fn another_panel_runs(lock: &std::io::Result<File>) -> bool {
    matches!(lock.as_ref().map(File::try_lock), Ok(Err(TryLockError::WouldBlock)))
}

fn main() {
    let _ = fs::create_dir_all(home());
    let lock = File::options().create(true).append(true).open(home().join("panel.lock"));
    if another_panel_runs(&lock) {
        return;
    }
    // Read only while this panel holds the lock, so that installing a new panel can stop this one.
    let _ = fs::write(home().join("panel.pid"), std::process::id().to_string());
    // Wayland lets no window place itself or stay on top, and the panel does both, so it runs
    // through XWayland where there is one, unless told otherwise.
    #[cfg(target_os = "linux")]
    if env::var_os("GDK_BACKEND").is_none() && env::var_os("DISPLAY").is_some() {
        env::set_var("GDK_BACKEND", "x11");
        CHOSE_X11.store(true, Ordering::Relaxed);
    }
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![is_pinned, set_pinned, blurs_behind, status, updates, update, open_release, proof, run_next, set_slots, hand_back, open_path, evidence_image, open_logs, show_progress, set_height, quit])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            {
                app.set_activation_policy(tauri::ActivationPolicy::Accessory);
                if let Some(window) = app.get_webview_window("panel") {
                    make_panel(&window);
                }
                close_when_another_app_comes_forward(app.handle());
            }
            // An undecorated window's shadow on Windows comes with a white border on Windows 10.
            #[cfg(windows)]
            if let Some(window) = app.get_webview_window("panel") {
                let _ = window.set_shadow(false);
            }
            let icon = idle_icon()?;
            PINNED.store(pin_file().exists(), Ordering::Relaxed);
            let pin_item = CheckMenuItem::with_id(app, "pin", "Pin panel", true, pinned(), None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit panel", true, None::<&str>)?;
            #[cfg(target_os = "linux")]
            let menu = Menu::with_items(app, &[&MenuItem::with_id(app, "open", "Show panel", true, None::<&str>)?, &pin_item, &quit_item])?;
            #[cfg(not(target_os = "linux"))]
            let menu = Menu::with_items(app, &[&pin_item, &quit_item])?;
            app.manage(PinItem(pin_item));
            TrayIconBuilder::with_id("main")
                .icon(icon)
                .icon_as_template(true)
                .tooltip("verifique")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    #[cfg(target_os = "linux")]
                    "open" => open_from_menu(app),
                    "pin" => change_pin(app, !pinned()),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, rect, .. } = event {
                        let scale = tray.app_handle().get_webview_window("panel").and_then(|w| w.scale_factor().ok()).unwrap_or(1.0);
                        let position = rect.position.to_physical::<f64>(scale);
                        let size = rect.size.to_physical::<f64>(scale);
                        toggle(tray.app_handle(), position.x, position.y, size.width, size.height);
                    }
                })
                .build(app)?;
            if pinned() {
                if let Some(window) = app.get_webview_window("panel") {
                    show_pinned(&window);
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let (WindowEvent::Focused(false), false) = (event, pinned()) {
                #[cfg(target_os = "linux")]
                if pointer_over(window) {
                    return;
                }
                *HIDDEN_AT.lock().unwrap() = Some(Instant::now());
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("the panel could not start");
    drop(lock);
}
