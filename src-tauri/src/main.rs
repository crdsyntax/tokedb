#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(clippy::needless_return)]

use serde::Serialize;
#[cfg(not(windows))]
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, WindowEvent,
};
use tokedb_runtime::config::RuntimeConfig;
use tokedb_runtime::image::ImageSummary;
#[cfg(not(windows))]
use tokedb_runtime::runtime::ResourceLimits;
use tokedb_runtime::runtime::{Container, ContainerLogs, ResourceUsage};
#[cfg(not(windows))]
use tokedb_runtime::service::CreateRequest;
use tokedb_runtime::service::RuntimeService;
use tokedb_runtime::storage::Volume;



#[tauri::command]
fn list_containers(state: tauri::State<AppData>) -> Result<Vec<Container>, String> {
    #[cfg(windows)]
    {
        let _ = &state;
        return wsl_json(&["list"]);
    }
    #[cfg(not(windows))]
    {
        let svc = state.service.lock().map_err(|e| e.to_string())?;
        svc.list().map_err(|e| e.to_string())
    }
}

#[tauri::command]
fn inspect_container(state: tauri::State<AppData>, name: String) -> Result<Container, String> {
    #[cfg(windows)]
    {
        let _ = &state;
        return wsl_json(&["inspect", name.as_str()]);
    }
    #[cfg(not(windows))]
    {
        let svc = state.service.lock().map_err(|e| e.to_string())?;
        svc.inspect(&name).map_err(|e| e.to_string())
    }
}

#[tauri::command]
fn container_stats(state: tauri::State<AppData>, name: String) -> Result<ResourceUsage, String> {
    #[cfg(windows)]
    {
        let _ = &state;
        return wsl_json(&["stats", name.as_str()]);
    }
    #[cfg(not(windows))]
    {
        let svc = state.service.lock().map_err(|e| e.to_string())?;
        svc.stats(&name).map_err(|e| e.to_string())
    }
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn create_container(
    state: tauri::State<AppData>,
    name: String,
    image: String,
    memory_mb: Option<u64>,
    cpu_quota: Option<f64>,
    pids_max: Option<u64>,
    ports: Vec<String>,
    username: String,
    password: String,
) -> Result<Container, String> {
    #[cfg(windows)]
    {
        let _ = &state;
        let mut args = vec![String::from("create"), name.clone(), image.clone()];
        if let Some(mb) = memory_mb {
            args.push(String::from("--memory-mb"));
            args.push(mb.to_string());
        }
        if let Some(cq) = cpu_quota {
            args.push(String::from("--cpu-quota"));
            args.push(cq.to_string());
        }
        if let Some(pm) = pids_max {
            args.push(String::from("--pids-max"));
            args.push(pm.to_string());
        }
        for port in &ports {
            args.push(String::from("--port"));
            args.push(port.clone());
        }
        args.push(String::from("--user"));
        args.push(username);
        args.push(String::from("--password"));
        args.push(password);
        let out = wsl::capture(&args)?;
        return serde_json::from_str(&out)
            .map_err(|err| format!("respuesta inválida del backend WSL: {err}"));
    }
    #[cfg(not(windows))]
    {
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
            db_user: tokedb_runtime::service::DbUser {
                username,
                password,
            },
        })
        .map_err(|e| e.to_string())
    }
}

#[tauri::command]
fn start_container(state: tauri::State<AppData>, name: String) -> Result<(), String> {
    #[cfg(windows)]
    {
        let _ = &state;
        wsl::spawn(&[String::from("start"), name])
    }
    #[cfg(not(windows))]
    {
        let svc = state.service.lock().map_err(|e| e.to_string())?;
        svc.start(&name).map_err(|e| e.to_string())
    }
}

#[tauri::command]
fn stop_container(state: tauri::State<AppData>, name: String) -> Result<(), String> {
    #[cfg(windows)]
    {
        let _ = &state;
        wsl::capture(&[String::from("stop"), name]).map(|_| ())
    }
    #[cfg(not(windows))]
    {
        let svc = state.service.lock().map_err(|e| e.to_string())?;
        svc.stop(&name).map_err(|e| e.to_string())
    }
}

#[tauri::command]
fn destroy_container(state: tauri::State<AppData>, name: String) -> Result<(), String> {
    #[cfg(windows)]
    {
        let _ = &state;
        wsl::capture(&[String::from("destroy"), name]).map(|_| ())
    }
    #[cfg(not(windows))]
    {
        let svc = state.service.lock().map_err(|e| e.to_string())?;
        svc.destroy(&name).map_err(|e| e.to_string())
    }
}

#[tauri::command]
fn read_logs(state: tauri::State<AppData>, name: String) -> Result<ContainerLogs, String> {
    #[cfg(windows)]
    {
        let _ = &state;
        return wsl_json(&["logs", name.as_str()]);
    }
    #[cfg(not(windows))]
    {
        let svc = state.service.lock().map_err(|e| e.to_string())?;
        svc.read_logs(&name).map_err(|e| e.to_string())
    }
}



#[tauri::command]
fn list_images(state: tauri::State<AppData>) -> Result<Vec<ImageSummary>, String> {
    #[cfg(windows)]
    {
        let _ = &state;
        return wsl_json(&["images"]);
    }
    #[cfg(not(windows))]
    {
        let svc = state.service.lock().map_err(|e| e.to_string())?;
        svc.images().map_err(|e| e.to_string())
    }
}

#[tauri::command]
fn import_image(state: tauri::State<AppData>, path: String) -> Result<String, String> {
    #[cfg(windows)]
    {
        let _ = &state;
        return wsl::capture(&[String::from("import"), path]);
    }
    #[cfg(not(windows))]
    {
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
}

#[tauri::command]
fn export_image(
    state: tauri::State<AppData>,
    reference: String,
    output: String,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        let _ = &state;
        return wsl::capture(&[String::from("export"), reference, output]).map(|_| ());
    }
    #[cfg(not(windows))]
    {
        let svc = state.service.lock().map_err(|e| e.to_string())?;
        svc.export(&reference, &PathBuf::from(&output))
            .map_err(|e| e.to_string())
    }
}

#[tauri::command]
fn pull_image(
    state: tauri::State<AppData>,
    reference: String,
    registry: Option<String>,
) -> Result<String, String> {
    #[cfg(windows)]
    {
        let _ = &state;
        let mut args = vec![String::from("pull"), reference];
        if let Some(reg) = registry {
            args.push(String::from("--registry"));
            args.push(reg);
        }
        return wsl::capture(&args);
    }
    #[cfg(not(windows))]
    {
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
}

#[tauri::command]
fn remove_image(state: tauri::State<AppData>, reference: String) -> Result<(), String> {
    #[cfg(windows)]
    {
        let _ = &state;
        return wsl::capture(&[String::from("rmi"), reference]).map(|_| ());
    }
    #[cfg(not(windows))]
    {
        let svc = state.service.lock().map_err(|e| e.to_string())?;
        svc.remove_image(&reference).map_err(|e| e.to_string())
    }
}



#[tauri::command]
fn list_volumes(state: tauri::State<AppData>) -> Result<Vec<Volume>, String> {
    #[cfg(windows)]
    {
        let _ = &state;
        return wsl_json(&["volumes", "list"]);
    }
    #[cfg(not(windows))]
    {
        let svc = state.service.lock().map_err(|e| e.to_string())?;
        svc.volume_list().map_err(|e| e.to_string())
    }
}

#[tauri::command]
fn create_volume(state: tauri::State<AppData>, name: String) -> Result<Volume, String> {
    #[cfg(windows)]
    {
        let _ = &state;
        return wsl_json(&["volumes", "create", name.as_str()]);
    }
    #[cfg(not(windows))]
    {
        let svc = state.service.lock().map_err(|e| e.to_string())?;
        svc.volume_create(&name).map_err(|e| e.to_string())
    }
}

#[tauri::command]
fn remove_volume(state: tauri::State<AppData>, name: String) -> Result<(), String> {
    #[cfg(windows)]
    {
        let _ = &state;
        return wsl::capture(&[String::from("volumes"), String::from("remove"), name]).map(|_| ());
    }
    #[cfg(not(windows))]
    {
        let svc = state.service.lock().map_err(|e| e.to_string())?;
        svc.volume_remove(&name).map_err(|e| e.to_string())
    }
}

#[tauri::command]
fn backup_volume(
    state: tauri::State<AppData>,
    name: String,
    dest: String,
) -> Result<String, String> {
    #[cfg(windows)]
    {
        let _ = &state;
        return wsl::capture(&[String::from("volumes"), String::from("backup"), name, dest]);
    }
    #[cfg(not(windows))]
    {
        let svc = state.service.lock().map_err(|e| e.to_string())?;
        let path = svc
            .volume_backup(&name, &PathBuf::from(&dest))
            .map_err(|e| e.to_string())?;
        Ok(path.display().to_string())
    }
}



#[tauri::command]
fn get_data_root(state: tauri::State<AppData>) -> Result<String, String> {
    #[cfg(windows)]
    {
        let _ = &state;
        return Ok(wsl::data_root_display());
    }
    #[cfg(not(windows))]
    {
        let svc = state.service.lock().map_err(|e| e.to_string())?;
        Ok(svc.config().data_root.display().to_string())
    }
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
        EngineInfo {
            name: "Redis".into(),
            engine: "redis".into(),
            default_port: 6379,
            data_directory: "/data".into(),
        },
        EngineInfo {
            name: "SQLite".into(),
            engine: "sqlite".into(),
            default_port: 54321,
            data_directory: "/var/lib/sqlite".into(),
        },
        EngineInfo {
            name: "SQL".into(),
            engine: "sql".into(),
            default_port: 1433,
            data_directory: "/var/opt/mssql/data".into(),
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



pub struct AppData {
    #[allow(dead_code)]
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
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;

            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                if let Some(w) = window.get_webview_window("main") {
                    let _ = w.hide();
                }
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
            container_stats,
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











#[cfg(windows)]
mod wsl {
    use std::process::{Child, Command};
    use std::sync::Mutex;

    const ENV_DISTRO: &str = "TOKEDB_WSL_DISTRO";
    const ENV_BINARY: &str = "TOKEDB_WSL_BINARY";
    const ENV_USER: &str = "TOKEDB_WSL_USER";

    static PERSISTENT_CHILDREN: Mutex<Vec<Child>> = Mutex::new(Vec::new());

    fn distro() -> String {
        std::env::var(ENV_DISTRO).unwrap_or_else(|_| "Ubuntu-24.04".to_string())
    }

    fn binary() -> String {
        std::env::var(ENV_BINARY).unwrap_or_else(|_| "/usr/local/bin/tokedb".to_string())
    }

    fn user() -> String {
        std::env::var(ENV_USER).unwrap_or_else(|_| "root".to_string())
    }

    fn quote(value: &str) -> String {
        format!("'{}'", value.replace('\'', "'\\''"))
    }

    fn data_root_export() -> String {
        match std::env::var("TOKEDB_DATA_ROOT") {
            Ok(raw) if !raw.trim().is_empty() => {
                format!("export TOKEDB_DATA_ROOT={}; ", quote(&to_wsl_path(&raw)))
            }
            _ => String::new(),
        }
    }

    fn to_wsl_path(raw: &str) -> String {
        let bytes = raw.as_bytes();
        if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
            let drive = (bytes[0] as char).to_ascii_lowercase();
            format!("/mnt/{}/{}", drive, &raw[2..].replace('\\', "/"))
        } else {
            raw.to_string()
        }
    }

    
    
    
    
    
    pub fn capture(args: &[String]) -> Result<String, String> {
        let mut script = format!("{}exec {} --json", data_root_export(), quote(&binary()));
        for arg in args {
            script.push(' ');
            script.push_str(&quote(arg));
        }
        let output = Command::new("wsl.exe")
            .arg("-d")
            .arg(distro())
            .arg("-u")
            .arg(user())
            .arg("--")
            .arg("sh")
            .arg("-c")
            .arg(&script)
            .output()
            .map_err(|err| {
                format!(
                    "no se pudo iniciar wsl.exe: {err} (instala WSL2 y el distro `{d}`)",
                    d = distro()
                )
            })?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
        }
    }

    
    
    pub fn spawn(args: &[String]) -> Result<(), String> {
        let joined = args
            .iter()
            .map(|arg| quote(arg))
            .collect::<Vec<_>>()
            .join(" ");
        let script = format!(
            "{}exec {} {}",
            data_root_export(),
            quote(&binary()),
            joined
        );
        let child = Command::new("wsl.exe")
            .arg("-d")
            .arg(distro())
            .arg("-u")
            .arg(user())
            .arg("--")
            .arg("sh")
            .arg("-c")
            .arg(&script)
            .spawn()
            .map_err(|err| format!("no se pudo iniciar wsl.exe: {err}"))?;
        PERSISTENT_CHILDREN
            .lock()
            .map_err(|_| "estado de procesos envenenado".to_string())?
            .push(child);
        Ok(())
    }

    
    pub fn data_root_display() -> String {
        match std::env::var("TOKEDB_DATA_ROOT") {
            Ok(raw) if !raw.trim().is_empty() => {
                let bytes = raw.as_bytes();
                if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
                    let drive = (bytes[0] as char).to_ascii_lowercase();
                    format!("/mnt/{}/{}", drive, &raw[2..].replace('\\', "/"))
                } else {
                    raw
                }
            }
            _ => "/var/lib/db-runtime".to_string(),
        }
    }
}

#[cfg(windows)]
fn wsl_json<T>(args: &[&str]) -> Result<T, String>
where
    T: serde::de::DeserializeOwned,
{
    let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let out = wsl::capture(&owned)?;
    serde_json::from_str(&out).map_err(|err| format!("respuesta inválida del backend WSL: {err}"))
}
