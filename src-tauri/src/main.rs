#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, WindowEvent,
};
use serde::Serialize;
use tokedb_runtime::config::RuntimeConfig;
use tokedb_runtime::image::ImageSummary;
use tokedb_runtime::runtime::{Container, ContainerLogs, ResourceLimits};
use tokedb_runtime::service::{CreateRequest, RuntimeService};
use tokedb_runtime::storage::Volume;

// ── Tauri commands ─────────────────────────────────────────────────

#[tauri::command]
fn list_containers(state: tauri::State<AppData>) -> Result<Vec<Container>, String> {
    let svc = state.service.lock().map_err(|e| e.to_string())?;
    svc.list().map_err(|e| e.to_string())
}

#[tauri::command]
fn inspect_container(state: tauri::State<AppData>, name: String) -> Result<Container, String> {
    let svc = state.service.lock().map_err(|e| e.to_string())?;
    svc.inspect(&name).map_err(|e| e.to_string())
}

#[tauri::command]
fn create_container(
    state: tauri::State<AppData>,
    name: String,
    image: String,
    memory_mb: Option<u64>,
    cpu_quota: Option<f64>,
    pids_max: Option<u64>,
    ports: Vec<String>,
) -> Result<Container, String> {
    let svc = state.service.lock().map_err(|e| e.to_string())?;
    svc.create(&CreateRequest {
        name,
        image,
        resources: ResourceLimits {
            memory_bytes: memory_mb.map(|mb| mb.saturating_mul(1024 * 1024)),
            cpu_quota,
            pids_max,
        },
        ports,
        env: vec![],
        args: vec![],
    })
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn start_container(state: tauri::State<AppData>, name: String) -> Result<(), String> {
    let svc = state.service.lock().map_err(|e| e.to_string())?;
    svc.start(&name).map_err(|e| e.to_string())
}

#[tauri::command]
fn stop_container(state: tauri::State<AppData>, name: String) -> Result<(), String> {
    let svc = state.service.lock().map_err(|e| e.to_string())?;
    svc.stop(&name).map_err(|e| e.to_string())
}

#[tauri::command]
fn destroy_container(state: tauri::State<AppData>, name: String) -> Result<(), String> {
    let svc = state.service.lock().map_err(|e| e.to_string())?;
    svc.destroy(&name).map_err(|e| e.to_string())
}

#[tauri::command]
fn read_logs(state: tauri::State<AppData>, name: String) -> Result<ContainerLogs, String> {
    let svc = state.service.lock().map_err(|e| e.to_string())?;
    svc.read_logs(&name).map_err(|e| e.to_string())
}

// ── Image commands ─────────────────────────────────────────────────

#[tauri::command]
fn list_images(state: tauri::State<AppData>) -> Result<Vec<ImageSummary>, String> {
    let svc = state.service.lock().map_err(|e| e.to_string())?;
    svc.images().map_err(|e| e.to_string())
}

#[tauri::command]
fn import_image(state: tauri::State<AppData>, path: String) -> Result<String, String> {
    let svc = state.service.lock().map_err(|e| e.to_string())?;
    let image = svc
        .import(&PathBuf::from(&path))
        .map_err(|e| e.to_string())?;
    Ok(format!(
        "imported {} ({} layer(s))",
        image.reference,
        image.manifest.layers.len()
    ))
}

#[tauri::command]
fn export_image(
    state: tauri::State<AppData>,
    reference: String,
    output: String,
) -> Result<(), String> {
    let svc = state.service.lock().map_err(|e| e.to_string())?;
    svc.export(&reference, &PathBuf::from(&output))
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn pull_image(
    state: tauri::State<AppData>,
    reference: String,
    registry: Option<String>,
) -> Result<String, String> {
    let svc = state.service.lock().map_err(|e| e.to_string())?;
    let image = svc
        .pull(&reference, registry.as_deref())
        .map_err(|e| e.to_string())?;
    Ok(format!(
        "pulled {} ({} layer(s))",
        image.reference,
        image.manifest.layers.len()
    ))
}

#[tauri::command]
fn remove_image(state: tauri::State<AppData>, reference: String) -> Result<(), String> {
    let svc = state.service.lock().map_err(|e| e.to_string())?;
    svc.remove_image(&reference).map_err(|e| e.to_string())
}

// ── Volume commands ────────────────────────────────────────────────

#[tauri::command]
fn list_volumes(state: tauri::State<AppData>) -> Result<Vec<Volume>, String> {
    let svc = state.service.lock().map_err(|e| e.to_string())?;
    svc.volume_list().map_err(|e| e.to_string())
}

#[tauri::command]
fn create_volume(state: tauri::State<AppData>, name: String) -> Result<Volume, String> {
    let svc = state.service.lock().map_err(|e| e.to_string())?;
    svc.volume_create(&name).map_err(|e| e.to_string())
}

#[tauri::command]
fn remove_volume(state: tauri::State<AppData>, name: String) -> Result<(), String> {
    let svc = state.service.lock().map_err(|e| e.to_string())?;
    svc.volume_remove(&name).map_err(|e| e.to_string())
}

#[tauri::command]
fn backup_volume(
    state: tauri::State<AppData>,
    name: String,
    dest: String,
) -> Result<String, String> {
    let svc = state.service.lock().map_err(|e| e.to_string())?;
    let path = svc
        .volume_backup(&name, &PathBuf::from(&dest))
        .map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

// ── Config commands ────────────────────────────────────────────────

#[tauri::command]
fn get_data_root(state: tauri::State<AppData>) -> Result<String, String> {
    let svc = state.service.lock().map_err(|e| e.to_string())?;
    Ok(svc.config().data_root.display().to_string())
}

#[tauri::command]
fn get_engine_info() -> Vec<EngineInfo> {
    vec![
        EngineInfo {
            name: "MariaDB".into(),
            engine: "mariadb".into(),
            default_port: 3306,
            data_directory: "/var/lib/mysql".into(),
        },
        EngineInfo {
            name: "MySQL".into(),
            engine: "mysql".into(),
            default_port: 3306,
            data_directory: "/var/lib/mysql".into(),
        },
        EngineInfo {
            name: "PostgreSQL".into(),
            engine: "postgres".into(),
            default_port: 5432,
            data_directory: "/var/lib/postgresql/data".into(),
        },
        EngineInfo {
            name: "MongoDB".into(),
            engine: "mongodb".into(),
            default_port: 27017,
            data_directory: "/data/db".into(),
        },
    ]
}

#[derive(Serialize)]
pub struct EngineInfo {
    name: String,
    engine: String,
    default_port: u16,
    data_directory: String,
}

// ── Window commands ────────────────────────────────────────────────

#[tauri::command]
fn minimize_window(app: AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.minimize();
    }
}

#[tauri::command]
fn hide_to_tray(app: AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}

// ── Main ───────────────────────────────────────────────────────────

pub struct AppData {
    service: Mutex<RuntimeService>,
}

fn main() {
    let config = RuntimeConfig::from_env().unwrap_or_default();
    let state = AppData {
        service: Mutex::new(RuntimeService::new(config)),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(state)
        .setup(|app| {
            let show_i = MenuItem::with_id(app, "show", "Mostrar ventana", true, None::<&str>)?;
            let hide_i = MenuItem::with_id(app, "hide", "Ocultar ventana", true, None::<&str>)?;
            let quit_i = MenuItem::with_id(app, "quit", "Salir", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_i, &hide_i, &quit_i])?;

            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("tokedb Manager")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "hide" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.hide();
                        }
                    }
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| match event {
                    TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } => {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    _ => {}
                })
                .build(app)?;

            Ok(())
        })
        .on_window_event(|_window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            list_containers,
            inspect_container,
            create_container,
            start_container,
            stop_container,
            destroy_container,
            read_logs,
            list_images,
            import_image,
            export_image,
            pull_image,
            remove_image,
            list_volumes,
            create_volume,
            remove_volume,
            backup_volume,
            get_data_root,
            get_engine_info,
            minimize_window,
            hide_to_tray,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tokedb manager");
}
