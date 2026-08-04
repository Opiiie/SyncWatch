mod discovery;
mod player;
mod protocol;
mod ws_server;

#[cfg(desktop)]
use tauri::Manager;

#[tauri::command]
fn get_ws_server_info(
    server: tauri::State<'_, ws_server::WsServerInfo>,
) -> ws_server::WsServerInfo {
    server.inner().clone()
}

#[tauri::command]
async fn set_room_media_sources(
    server: tauri::State<'_, ws_server::ServerState>,
    room_code: String,
    access_token: String,
    sources: Vec<ws_server::MediaSourceInput>,
) -> Result<(), String> {
    server
        .set_media_sources(&room_code, &access_token, sources)
        .await
}

#[tauri::command]
async fn start_room_discovery(
    discovery: tauri::State<'_, discovery::DiscoveryController>,
    server_state: tauri::State<'_, ws_server::ServerState>,
    server_info: tauri::State<'_, ws_server::WsServerInfo>,
) -> Result<(), String> {
    discovery
        .start(server_state.inner().clone(), server_info.inner().clone())
        .await
}

#[tauri::command]
fn stop_room_discovery(discovery: tauri::State<'_, discovery::DiscoveryController>) {
    discovery.stop();
}

#[tauri::command]
async fn discover_local_rooms(
    room_code: Option<String>,
) -> Result<Vec<discovery::DiscoveredRoom>, String> {
    discovery::discover_rooms(room_code).await
}

#[tauri::command]
async fn find_external_subtitles(video_paths: Vec<String>) -> Vec<ws_server::DetectedSubtitles> {
    ws_server::find_external_subtitles(video_paths).await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let (server_listener, server_info) = ws_server::bind_available_port()
        .expect("failed to reserve an available port for the SyncWatch server");
    let server_state = ws_server::ServerState::new();
    let background_state = server_state.clone();

    let mut builder = tauri::Builder::default();

    #[cfg(desktop)]
    {
        let allow_multiple_instances = cfg!(debug_assertions)
            && std::env::args().any(|argument| argument == "--allow-multiple-instances");
        if !allow_multiple_instances {
            builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            }));
        }
    }

    builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(move |app| {
            #[cfg(windows)]
            if let Some(window) = app.get_webview_window("main") {
                window.with_webview(|webview| unsafe {
                    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings3;
                    use windows::core::Interface;

                    if let Ok(core_webview) = webview.controller().CoreWebView2() {
                        if let Ok(settings) = core_webview.Settings() {
                            let _ = settings.SetAreDefaultContextMenusEnabled(false);
                            if let Ok(settings3) = settings.cast::<ICoreWebView2Settings3>() {
                                let _ = settings3.SetAreBrowserAcceleratorKeysEnabled(false);
                            }
                        }
                    }
                })?;
            }
            tauri::async_runtime::spawn(async move {
                if let Err(error) = ws_server::run(background_state, server_listener).await {
                    eprintln!("SyncWatch WebSocket server failed: {error}");
                }
            });
            Ok(())
        })
        .manage(server_state)
        .manage(server_info)
        .manage(discovery::DiscoveryController::default())
        .manage(player::PlayerController::default())
        .invoke_handler(tauri::generate_handler![
            get_ws_server_info,
            set_room_media_sources,
            start_room_discovery,
            stop_room_discovery,
            discover_local_rooms,
            find_external_subtitles,
            player::player_create_surface,
            player::player_set_surface_bounds,
            player::player_load,
            player::player_set_paused,
            player::player_set_volume,
            player::player_seek,
            player::player_set_speed,
            player::player_get_state,
            player::player_select_track,
            player::player_add_subtitle,
            player::player_destroy,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
