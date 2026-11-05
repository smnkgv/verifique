#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use tauri::image::Image;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{ActivationPolicy, AppHandle, LogicalSize, Manager, PhysicalPosition, WebviewWindow, WindowEvent};

/// Clicking the icon while the panel is open first takes focus from it, which hides it. Without this
/// gap the same click would open it again straight away.
const REOPEN_GAP: Duration = Duration::from_millis(250);
const GAP_BELOW_ICON: f64 = 6.0;

static HIDDEN_AT: Mutex<Option<Instant>> = Mutex::new(None);

/// `--pinned` keeps the panel open in the corner of the screen, so people watching a demo can follow it.
fn pinned() -> bool {
    std::env::args().any(|arg| arg == "--pinned")
}

fn pin(window: &WebviewWindow) {
    if let Ok(Some(monitor)) = window.current_monitor() {
        let scale = monitor.scale_factor();
        let width = window.outer_size().map(|size| size.width as f64).unwrap_or(360.0 * scale);
        let x = monitor.position().x as f64 + monitor.size().width as f64 - width - 16.0 * scale;
        let y = monitor.position().y as f64 + 36.0 * scale;
        let _ = window.set_position(PhysicalPosition::new(x, y));
    }
    let _ = window.show();
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

/// An app opened from the Finder has almost no PATH, so look where the install puts the command first.
fn verifique() -> PathBuf {
    let installed = home().join(".local/bin/verifique");
    if installed.exists() {
        installed
    } else {
        PathBuf::from("verifique")
    }
}

fn cli(args: &[&str], cwd: Option<&str>) -> Result<String, String> {
    let mut command = Command::new(verifique());
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

#[tauri::command]
fn open_path(path: String) -> Result<(), String> {
    Command::new("open").arg(path).status().map(|_| ()).map_err(|error| error.to_string())
}

/// A screenshot from a check's evidence, as a data URL. Only files under the job folders can be read.
#[tauri::command]
fn evidence_image(path: String) -> Result<String, String> {
    let file = PathBuf::from(&path).canonicalize().map_err(|error| error.to_string())?;
    if !file.starts_with(home().join(".verifique/jobs")) {
        return Err("that file isn't part of a check's evidence".to_string());
    }
    let bytes = std::fs::read(&file).map_err(|error| error.to_string())?;
    let kind = if path.to_lowercase().ends_with(".png") { "png" } else { "jpeg" };
    Ok(format!("data:image/{kind};base64,{}", STANDARD.encode(bytes)))
}

#[tauri::command]
fn open_logs() -> Result<(), String> {
    open_path(home().join(".verifique/server.log").display().to_string())
}

/// The icon shows a number only when something needs you.
#[tauri::command]
fn set_badge(app: AppHandle, count: usize) -> Result<(), String> {
    let tray = app.tray_by_id("main").ok_or("no tray icon")?;
    let title = (count > 0).then(|| count.to_string());
    tray.set_title(title).map_err(|error| error.to_string())
}

#[tauri::command]
fn set_height(window: WebviewWindow, height: f64) -> Result<(), String> {
    window.set_size(LogicalSize::new(360.0, height)).map_err(|error| error.to_string())
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
    let scale = window.scale_factor().unwrap_or(1.0);
    let width = window.outer_size().map(|size| size.width as f64).unwrap_or(360.0 * scale);
    let x = icon_x + icon_w / 2.0 - width / 2.0;
    let y = icon_y + icon_h + GAP_BELOW_ICON * scale;
    let _ = window.set_position(PhysicalPosition::new(x, y));
    let _ = window.show();
    let _ = window.set_focus();
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![status, proof, run_next, set_slots, hand_back, open_path, evidence_image, open_logs, set_badge, set_height, quit])
        .setup(|app| {
            app.set_activation_policy(ActivationPolicy::Accessory);
            let icon = Image::from_bytes(include_bytes!("../icons/tray.png"))?;
            TrayIconBuilder::with_id("main")
                .icon(icon)
                .icon_as_template(true)
                .tooltip("verifique")
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
                    pin(&window);
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let (WindowEvent::Focused(false), false) = (event, pinned()) {
                *HIDDEN_AT.lock().unwrap() = Some(Instant::now());
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("the panel could not start");
}
