// Kept desktop shell.
//
// One small native process:
//   * the background agent loop (sync → extract → plan → notify) runs on a Tokio task,
//   * the dashboard is a WebView2 window that is *destroyed* when you close it
//     (so the resident agent uses only a few tens of MB), and re-created from the tray,
//   * a tray icon offers Open / Sync now / Pause / Quit.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod keychain;

use kept_core::secrets::{MemoryStore, SecretStore};
use kept_core::{notify, Agent};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, RunEvent, WindowEvent};
use tauri_plugin_notification::NotificationExt;

pub struct AppState {
    pub agent: Arc<Agent>,
    pub wake: Arc<tokio::sync::Notify>,
    pub keychain_ok: bool,
}

struct ShellState {
    quitting: AtomicBool,
    told_about_tray: AtomicBool,
    pause_item: Mutex<Option<CheckMenuItem<tauri::Wry>>>,
    tray_id: Mutex<Option<tauri::tray::TrayIconId>>,
    _log_guard: Option<tracing_appender::non_blocking::WorkerGuard>,
}

/// Show the dashboard, creating the webview if it was closed.
pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let cfg = app.config().app.windows.iter().find(|w| w.label == "main").cloned();
    if let Some(cfg) = cfg {
        match tauri::WebviewWindowBuilder::from_config(app, &cfg).and_then(|b| b.build()) {
            Ok(w) => {
                let _ = w.show();
                let _ = w.set_focus();
            }
            Err(e) => tracing::error!("could not open window: {e}"),
        }
    }
}

pub fn refresh_tray(app: &AppHandle) {
    let st = app.state::<AppState>();
    let paused = st.agent.prefs().map(|p| p.monitoring_paused).unwrap_or(false);
    let unread = st.agent.db(notify::unread_count).unwrap_or(0);
    let shell = app.state::<ShellState>();
    let item = shell.pause_item.lock().unwrap().clone();
    if let Some(item) = item {
        let _ = item.set_checked(paused);
    }
    let id = shell.tray_id.lock().unwrap().clone();
    if let Some(id) = id {
        if let Some(tray) = app.tray_by_id(&id) {
            let tip = match (paused, unread) {
                (true, _) => "Kept — monitoring paused".to_string(),
                (false, 0) => "Kept — all caught up".to_string(),
                (false, n) => format!("Kept — {n} unread reminder{}", if n == 1 { "" } else { "s" }),
            };
            let _ = tray.set_tooltip(Some(tip));
        }
    }
}

fn init_logging(dir: &std::path::Path) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};
    let logs = dir.join("logs");
    std::fs::create_dir_all(&logs).ok()?;
    let appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("kept")
        .filename_suffix("log")
        .max_log_files(7)
        .build(&logs)
        .ok()?;
    let (nb, guard) = tracing_appender::non_blocking(appender);
    // Logs contain ids, counts and error kinds only — never message bodies or secrets.
    let filter = EnvFilter::try_from_env("KEPT_LOG").unwrap_or_else(|_| EnvFilter::new("info,kept=info,kept_core=info,tao=warn,wry=warn"));
    let _ = tracing_subscriber::registry().with(filter).with(fmt::layer().with_writer(nb).with_ansi(false)).try_init();
    Some(guard)
}

fn start_agent_loop(app: AppHandle) {
    let st = app.state::<AppState>();
    let agent = st.agent.clone();
    let wake = st.wake.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        loop {
            let rep = agent.tick().await;
            if let Some(gap) = rep.resumed_after_gap_minutes {
                tracing::info!("agent resumed after {gap} min (sleep/offline); reminders recalculated");
            }
            for e in &rep.errors {
                tracing::warn!("tick: {e}");
            }
            for t in &rep.toasts {
                let r = app.notification().builder().title(&t.title).body(&t.body).show();
                if let Err(err) = r {
                    tracing::warn!("toast failed: {err}");
                    let _ = agent.db(|c| notify::mark_failed(c, &t.notification_id, &err.to_string()));
                }
            }
            if rep.synced_accounts > 0 || rep.processed_items > 0 || !rep.toasts.is_empty() || rep.notifications_created > 0 {
                commands::changed(&app);
            }
            refresh_tray(&app);
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(60)) => {}
                _ = wake.notified() => {}
            }
        }
    });
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Kept", true, None::<&str>)?;
    let sync = MenuItem::with_id(app, "sync", "Sync now", true, None::<&str>)?;
    let paused = app.state::<AppState>().agent.prefs().map(|p| p.monitoring_paused).unwrap_or(false);
    let pause = CheckMenuItem::with_id(app, "pause", "Pause monitoring", true, paused, None::<&str>)?;
    let quiet = MenuItem::with_id(app, "quiet", "Silence notifications for 1 hour", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Kept", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &PredefinedMenuItem::separator(app)?, &sync, &pause, &quiet, &PredefinedMenuItem::separator(app)?, &quit])?;
    let mut b = TrayIconBuilder::with_id("kept-tray").menu(&menu).show_menu_on_left_click(false).tooltip("Kept");
    if let Some(icon) = app.default_window_icon() {
        b = b.icon(icon.clone());
    }
    let tray = b
        .on_menu_event(|app, ev| match ev.id().as_ref() {
            "open" => show_main(app),
            "sync" => app.state::<AppState>().wake.notify_one(),
            "pause" => {
                let st = app.state::<AppState>();
                if let Ok(mut p) = st.agent.prefs() {
                    p.monitoring_paused = !p.monitoring_paused;
                    let _ = st.agent.save_prefs(&p);
                }
                refresh_tray(app);
                commands::changed(app);
            }
            "quiet" => {
                let st = app.state::<AppState>();
                if let Ok(mut p) = st.agent.prefs() {
                    p.notifications_paused_until = Some(chrono::Utc::now() + chrono::Duration::hours(1));
                    let _ = st.agent.save_prefs(&p);
                }
                commands::changed(app);
            }
            "quit" => {
                app.state::<ShellState>().quitting.store(true, Ordering::SeqCst);
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, ev| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
                show_main(tray.app_handle());
            }
        })
        .build(app)?;
    *app.state::<ShellState>().pause_item.lock().unwrap() = Some(pause);
    *app.state::<ShellState>().tray_id.lock().unwrap() = Some(tray.id().clone());
    Ok(())
}

fn main() {
    let start_minimized_flag = std::env::args().any(|a| a == "--minimized");
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| show_main(app)))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec!["--minimized"])))
        .setup(move |app| {
            let dir = app.path().app_local_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let guard = init_logging(&dir);
            std::panic::set_hook(Box::new(|info| tracing::error!("panic: {info}")));
            let keychain_ok = keychain::probe();
            let secrets: Arc<dyn SecretStore> = if keychain_ok {
                Arc::new(keychain::KeychainStore)
            } else {
                tracing::error!("OS credential store unavailable; secrets will only be kept in memory for this session");
                Arc::new(MemoryStore::default())
            };
            let agent = Agent::open(&dir, secrets).map_err(|e| Box::<dyn std::error::Error>::from(format!("{e:#}")))?;
            let prefs = agent.prefs().unwrap_or_default();
            if let Err(e) = agent.reanalyse_after_upgrade() {
                tracing::warn!("re-analysis after upgrade failed: {e:#}");
            }
            tracing::info!("Kept {} starting; data dir {}", app.package_info().version, dir.display());
            app.manage(AppState { agent, wake: Arc::new(tokio::sync::Notify::new()), keychain_ok });
            app.manage(ShellState {
                quitting: AtomicBool::new(false),
                told_about_tray: AtomicBool::new(false),
                pause_item: Mutex::new(None),
                tray_id: Mutex::new(None),
                _log_guard: guard,
            });
            if let Err(e) = build_tray(app.handle()) {
                tracing::warn!("tray unavailable: {e}");
            }
            if !(start_minimized_flag || (prefs.start_minimized && prefs.onboarded)) {
                show_main(app.handle());
            }
            start_agent_loop(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { .. } = event {
                let app = window.app_handle();
                let close_to_tray = app.state::<AppState>().agent.prefs().map(|p| p.close_to_tray).unwrap_or(true);
                if !close_to_tray {
                    app.state::<ShellState>().quitting.store(true, Ordering::SeqCst);
                    app.exit(0);
                    return;
                }
                let shell = app.state::<ShellState>();
                if !shell.told_about_tray.swap(true, Ordering::SeqCst) {
                    let _ = app
                        .notification()
                        .builder()
                        .title("Kept is still running")
                        .body("Reminders keep working in the background. Open Kept from the tray icon, or quit it from the tray menu.")
                        .show();
                }
                // Let the window close: the webview is destroyed to free memory; the agent keeps running.
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_dashboard,
            commands::get_day_plan,
            commands::list_tasks,
            commands::get_task,
            commands::create_task,
            commands::update_task,
            commands::set_task_status,
            commands::review_tasks,
            commands::undo_change,
            commands::delete_task,
            commands::add_dependency,
            commands::get_source,
            commands::list_meetings,
            commands::get_briefing,
            commands::ai_briefing,
            commands::list_followups,
            commands::set_commitment,
            commands::list_notifications,
            commands::notification_action,
            commands::get_rules,
            commands::save_rule,
            commands::test_notification,
            commands::get_prefs,
            commands::save_prefs,
            commands::get_integrations,
            commands::connect_account,
            commands::sync_now,
            commands::disconnect_account,
            commands::set_account_streams,
            commands::set_secret,
            commands::has_secret,
            commands::test_ai,
            commands::import_files,
            commands::load_sample,
            commands::export_data,
            commands::backup_data,
            commands::delete_all_data,
            commands::storage_info,
            commands::audit_log,
            commands::ai_usage,
            commands::open_external,
            commands::open_data_folder,
            commands::app_info,
            commands::set_paused,
            commands::hide_window,
            commands::reanalyse,
            commands::connect_token,
            commands::related_documents,
            commands::push_to_todoist,
            commands::check_connection_value,
            commands::save_connection,
            commands::export_connection_settings,
            commands::import_connection_settings,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build Kept");

    app.run(|app, event| {
        if let RunEvent::ExitRequested { api, code, .. } = event {
            // Closing the last window must not stop the background agent.
            if code.is_none() && !app.state::<ShellState>().quitting.load(Ordering::SeqCst) {
                api.prevent_exit();
            }
        }
    });
}
