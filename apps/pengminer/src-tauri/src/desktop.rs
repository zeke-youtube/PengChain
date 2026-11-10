use crate::{
    engine::{public_error, Engine, Snapshot},
    storage::{Files, OsSessionStore, Settings},
};
use serde::Serialize;
use std::sync::Arc;
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Emitter, Manager,
};
use tauri_plugin_notification::NotificationExt;

fn failure(error: anyhow::Error) -> String {
    public_error(&error)
}
#[tauri::command]
fn snapshot(engine: tauri::State<'_, Arc<Engine>>) -> Snapshot {
    engine.snapshot()
}
#[tauri::command]
async fn create_wallet(
    engine: tauri::State<'_, Arc<Engine>>,
    password: String,
) -> Result<serde_json::Value, String> {
    engine
        .create_wallet(password)
        .await
        .map(|wallet| serde_json::json!({"walletId": wallet}))
        .map_err(failure)
}
#[tauri::command]
async fn login(
    engine: tauri::State<'_, Arc<Engine>>,
    wallet: String,
    password: String,
) -> Result<Snapshot, String> {
    engine.login(wallet, password).await.map_err(failure)
}
#[tauri::command]
async fn logout(engine: tauri::State<'_, Arc<Engine>>) -> Result<Snapshot, String> {
    engine.logout().await.map_err(failure)
}
#[tauri::command]
async fn start_mining(engine: tauri::State<'_, Arc<Engine>>) -> Result<Snapshot, String> {
    engine.start().await.map_err(failure)
}
#[tauri::command]
async fn stop_mining(engine: tauri::State<'_, Arc<Engine>>) -> Result<Snapshot, String> {
    engine.stop().await.map_err(failure)
}
#[tauri::command]
async fn refresh(engine: tauri::State<'_, Arc<Engine>>) -> Result<Snapshot, String> {
    engine.refresh().await.map_err(failure)
}
#[tauri::command]
async fn create_token(
    engine: tauri::State<'_, Arc<Engine>>,
    request: crate::api::CreateTokenRequest,
) -> Result<Snapshot, String> {
    engine.create_token(request).await.map_err(failure)
}
#[tauri::command]
async fn save_settings(
    engine: tauri::State<'_, Arc<Engine>>,
    settings: Settings,
) -> Result<Snapshot, String> {
    engine.save_settings(settings).await.map_err(failure)
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Devices {
    gpus: Vec<pengwallet_protocol::Device>,
    warning: Option<String>,
}
#[tauri::command]
async fn devices(engine: tauri::State<'_, Arc<Engine>>) -> Result<Devices, String> {
    engine
        .devices()
        .await
        .map(|(gpus, warning)| Devices { gpus, warning })
        .map_err(failure)
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            snapshot,
            login,
            create_wallet,
            logout,
            start_mining,
            stop_mining,
            refresh,
            create_token,
            save_settings,
            devices
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let engine = Engine::new(
                Arc::new(OsSessionStore),
                Files {
                    directory: app.path().app_data_dir()?,
                },
                Arc::new(move |event| {
                    let _ = handle.emit("pengminer:event", &event);
                    if event.snapshot.settings.notifications
                        && matches!(event.kind.as_str(), "mining_error" | "reward_accepted")
                    {
                        let message = if event.kind == "reward_accepted" {
                            "A verified mining reward was accepted."
                        } else {
                            "Mining stopped. Open PengMiner for details."
                        };
                        let _ = handle
                            .notification()
                            .builder()
                            .title("PengMiner")
                            .body(message)
                            .show();
                    }
                }),
            )?;
            app.manage(Arc::clone(&engine));
            let open = MenuItem::with_id(app, "open", "Open PengMiner", true, None::<&str>)?;
            let stop = MenuItem::with_id(app, "stop", "Stop mining", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &stop, &quit])?;
            let mut tray = TrayIconBuilder::new()
                .tooltip("PengMiner · PikaStudio")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.unminimize();
                            let _ = window.set_focus();
                        }
                    }
                    "stop" => {
                        let engine = app.state::<Arc<Engine>>().inner().clone();
                        tauri::async_runtime::spawn(async move {
                            let _ = engine.stop().await;
                        });
                    }
                    "quit" => {
                        let engine = app.state::<Arc<Engine>>().inner().clone();
                        let app = app.clone();
                        tauri::async_runtime::spawn(async move {
                            let _ = engine.stop().await;
                            app.exit(0);
                        });
                    }
                    _ => (),
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;
            tauri::async_runtime::spawn(async move {
                let _ = engine.initialize().await;
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let engine = window.state::<Arc<Engine>>().inner().clone();
                api.prevent_close();
                if engine.snapshot().settings.minimize_to_tray {
                    let _ = window.hide();
                } else {
                    let app = window.app_handle().clone();
                    tauri::async_runtime::spawn(async move {
                        let _ = engine.stop().await;
                        app.exit(0);
                    });
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("Could not initialize PengMiner")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if app.state::<Arc<Engine>>().snapshot().running {
                    api.prevent_exit();
                    let engine = app.state::<Arc<Engine>>().inner().clone();
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        let _ = engine.stop().await;
                        app.exit(0);
                    });
                }
            }
        });
}
