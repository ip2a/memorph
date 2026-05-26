use anyhow::Result;
use memorph_lib::{api, server};
use std::collections::HashSet;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use tauri::{PhysicalSize, WebviewUrl, WebviewWindowBuilder, WindowEvent};

const MIN_INNER_WIDTH: f64 = 620.0;
const MIN_INNER_HEIGHT: f64 = 423.0;
const DEFAULT_BASELINE_SCREEN_WIDTH: f64 = 2560.0;
const DEFAULT_BASELINE_SCREEN_HEIGHT: f64 = 1440.0;
const DEFAULT_BASELINE_WINDOW_WIDTH: f64 = 970.0;
const DEFAULT_BASELINE_WINDOW_HEIGHT: f64 = 670.0;
const DEFAULT_SCREEN_WIDTH_RATIO: f64 =
    DEFAULT_BASELINE_WINDOW_WIDTH / DEFAULT_BASELINE_SCREEN_WIDTH;
const DEFAULT_SCREEN_HEIGHT_RATIO: f64 =
    DEFAULT_BASELINE_WINDOW_HEIGHT / DEFAULT_BASELINE_SCREEN_HEIGHT;

fn start_local_server() -> Result<String> {
    let std_listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let addr = std_listener.local_addr()?;
    std_listener.set_nonblocking(true)?;
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("desktop server runtime should build");
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(std_listener)
                .expect("desktop listener should convert");
            let app = server::build_router();
            if let Err(error) = axum::serve(listener, app).await {
                eprintln!("memorph desktop server stopped: {error}");
            }
        });
    });

    Ok(format!("http://{}", addr))
}

fn clamp_dimension(target: f64, min: f64, max: f64) -> f64 {
    if max.is_finite() && max > 0.0 {
        target.clamp(min.min(max), max)
    } else {
        target.max(min)
    }
}

fn logical_work_area(monitor: &tauri::Monitor) -> (f64, f64) {
    let scale_factor = monitor.scale_factor();
    let work_area = monitor.work_area().size;
    (
        work_area.width as f64 / scale_factor,
        work_area.height as f64 / scale_factor,
    )
}

fn logical_monitor_size(monitor: &tauri::Monitor) -> (f64, f64) {
    let scale_factor = monitor.scale_factor();
    let size = monitor.size();
    (
        size.width as f64 / scale_factor,
        size.height as f64 / scale_factor,
    )
}

fn default_window_state(monitor: Option<&tauri::Monitor>) -> memorph_lib::config::DesktopWindowState {
    let (screen_width, screen_height) = monitor.map(logical_monitor_size).unwrap_or((
        DEFAULT_BASELINE_SCREEN_WIDTH,
        DEFAULT_BASELINE_SCREEN_HEIGHT,
    ));
    let (max_width, max_height) = monitor
        .map(logical_work_area)
        .unwrap_or((screen_width, screen_height));

    let width = screen_width * DEFAULT_SCREEN_WIDTH_RATIO;
    let height = screen_height * DEFAULT_SCREEN_HEIGHT_RATIO;

    memorph_lib::config::DesktopWindowState {
        width: clamp_dimension(width, MIN_INNER_WIDTH, max_width).round() as u32,
        height: clamp_dimension(height, MIN_INNER_HEIGHT, max_height).round() as u32,
    }
}

fn clamp_window_state(
    state: memorph_lib::config::DesktopWindowState,
    monitor: Option<&tauri::Monitor>,
) -> memorph_lib::config::DesktopWindowState {
    if let Some(monitor) = monitor {
        let (max_width, max_height) = logical_work_area(monitor);
        memorph_lib::config::DesktopWindowState {
            width: clamp_dimension(state.width as f64, MIN_INNER_WIDTH, max_width).round() as u32,
            height: clamp_dimension(state.height as f64, MIN_INNER_HEIGHT, max_height).round()
                as u32,
        }
    } else {
        memorph_lib::config::DesktopWindowState {
            width: state.width.max(MIN_INNER_WIDTH as u32),
            height: state.height.max(MIN_INNER_HEIGHT as u32),
        }
    }
}

fn initial_window_state(monitor: Option<&tauri::Monitor>) -> memorph_lib::config::DesktopWindowState {
    let stored = memorph_lib::config::desktop_window_state().ok().flatten();
    match stored {
        Some(state) => clamp_window_state(state, monitor),
        None => default_window_state(monitor),
    }
}

fn logical_state_from_physical(
    size: PhysicalSize<u32>,
    scale_factor: f64,
) -> memorph_lib::config::DesktopWindowState {
    memorph_lib::config::DesktopWindowState {
        width: ((size.width as f64) / scale_factor).round().max(1.0) as u32,
        height: ((size.height as f64) / scale_factor).round().max(1.0) as u32,
    }
}

fn persist_window_state(window_state: &Arc<Mutex<memorph_lib::config::DesktopWindowState>>) {
    let state = match window_state.lock() {
        Ok(state) => *state,
        Err(_) => return,
    };
    if let Err(error) = memorph_lib::config::set_desktop_window_state(state) {
        eprintln!("Failed to persist memorph desktop window size: {error}");
    }
}

fn prime_desktop_environment() {
    #[cfg(not(target_os = "windows"))]
    {
        if let Some(shell_path) = read_login_shell_path() {
            let current = std::env::var_os("PATH");
            if let Some(merged) = merge_path_values(&shell_path, current) {
                std::env::set_var("PATH", merged);
            }
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn read_login_shell_path() -> Option<String> {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "/bin/sh".to_string());
    let mut candidates = vec![shell, "/bin/zsh".to_string(), "/bin/bash".to_string(), "/bin/sh".to_string()];
    candidates.dedup();

    for program in candidates {
        let Ok(output) = Command::new(&program)
            .args(["-lc", "printf %s \"$PATH\""])
            .output()
        else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !value.is_empty() {
            return Some(value);
        }
    }

    None
}

fn merge_path_values(preferred: &str, current: Option<OsString>) -> Option<OsString> {
    let mut merged = Vec::new();
    let mut seen = HashSet::new();

    for value in [Some(OsString::from(preferred)), current].into_iter().flatten() {
        for path in std::env::split_paths(&value) {
            if path.as_os_str().is_empty() {
                continue;
            }
            if seen.insert(path.clone()) {
                merged.push(path);
            }
        }
    }

    std::env::join_paths(merged).ok()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = api::register_folder_picker(|start_path| {
        let mut dialog = rfd::FileDialog::new();
        if let Some(path) = start_path
            .map(PathBuf::from)
            .filter(|path| path.exists() && path.is_dir())
        {
            dialog = dialog.set_directory(path);
        }
        Ok(dialog
            .pick_folder()
            .map(|path| path.to_string_lossy().to_string()))
    });

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            prime_desktop_environment();
            if let Some(home) = dirs::home_dir() {
                let _ = std::env::set_current_dir(home);
            }

            let primary_monitor = app.primary_monitor().ok().flatten();
            let initial_state = initial_window_state(primary_monitor.as_ref());
            let url = start_local_server()?;
            let window = WebviewWindowBuilder::new(
                app,
                "main",
                WebviewUrl::External(url.parse().expect("desktop URL is valid")),
            )
            .title("memorph")
            .inner_size(initial_state.width as f64, initial_state.height as f64)
            .min_inner_size(MIN_INNER_WIDTH, MIN_INNER_HEIGHT)
            .center()
            .build()?;

            let latest_window_state = Arc::new(Mutex::new(initial_state));
            let latest_window_state_handle = Arc::clone(&latest_window_state);
            let event_window = window.clone();
            window.on_window_event(move |event| match event {
                WindowEvent::Resized(size) => {
                    if let Ok(scale_factor) = event_window.scale_factor() {
                        if let Ok(mut state) = latest_window_state_handle.lock() {
                            *state = logical_state_from_physical(*size, scale_factor);
                        }
                    }
                }
                WindowEvent::ScaleFactorChanged {
                    scale_factor,
                    new_inner_size,
                    ..
                } => {
                    if let Ok(mut state) = latest_window_state_handle.lock() {
                        *state = logical_state_from_physical(*new_inner_size, *scale_factor);
                    }
                }
                WindowEvent::CloseRequested { .. } | WindowEvent::Destroyed => {
                    persist_window_state(&latest_window_state_handle);
                }
                _ => {}
            });

            let _ = window.set_focus();
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running memorph desktop application");
}

#[cfg(test)]
mod tests {
    use super::merge_path_values;

    #[test]
    fn merge_path_values_prefers_shell_entries_without_duplicates() {
        let current = Some(std::env::join_paths(["/usr/bin", "/bin"]).expect("current path"));
        let merged = merge_path_values("/opt/homebrew/bin:/usr/bin", current).expect("merged path");
        let values = std::env::split_paths(&merged)
            .map(|path| path.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        assert_eq!(values, vec!["/opt/homebrew/bin", "/usr/bin", "/bin"]);
    }
}
