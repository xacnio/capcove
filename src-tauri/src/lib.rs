mod capture;
mod commands;
mod config;
#[cfg(windows)]
mod deletion_log;
mod drive;
mod folder_icon;
mod games_db;
mod icon_cache;
#[cfg(windows)]
mod hud_native;
mod integrity;
mod library;
mod logging;
mod meta;
mod overlay;
mod recorder;
#[cfg(windows)]
mod recording;
#[cfg(windows)]
mod video_thumb;
// Dev-only screenshot automation (store listing captures) — gated the same
// as its only call site below, so a release build doesn't compile a module
// that's genuinely unreachable there (that mismatch is what caused a wall of
// "never used" warnings under `cargo check --release`/`build`).
#[cfg(all(windows, debug_assertions))]
mod store_screenshots;
mod sound;
mod sync;
mod tag;
mod toast;
#[cfg(windows)]
mod toast_native;
mod tray;
mod translate;
mod wheel;
mod win_util;

use config::ConfigStore;
use drive::DriveClient;
use std::sync::{Arc, Mutex};
use sync::SyncState;
use tauri::{AppHandle, Emitter, Manager, WindowEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt as AutostartExt};

// ---------------------------------------------------------------------------
// Shared types
// ---------------------------------------------------------------------------

/// The window/monitor picker overlay (reused from the screenshot-tool days)
/// now exists purely to pick a *recording* target.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum OverlayMode {
    RecordWindow,
    RecordArea,
}

#[derive(Clone)]
pub(crate) struct MonitorInfo {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub scale: f32,
}

pub(crate) struct PendingCapture {
    pub image_jpeg: Option<String>,
    pub scale: f32,
    pub mode: OverlayMode,
    pub windows: Vec<capture::WinInfo>,
    pub mon_x: i32,
    pub mon_y: i32,
    pub mon_w: u32,
    pub mon_h: u32,
    pub monitors: Vec<MonitorInfo>,
    pub mon_jpegs: Vec<String>,
    pub live_mode: bool,
}

#[derive(Default)]
pub(crate) struct Pending(pub Mutex<Option<PendingCapture>>);

// ---------------------------------------------------------------------------
// Notification helpers
// ---------------------------------------------------------------------------

// Generic/uncategorized notifications land in `ToastCategory::General`,
// which always shows. Uses only the in-app toast (`toast.rs`), not a native
// OS toast, to stay corner-positioned and out of full-monitor recordings.
pub(crate) fn notify(app: &AppHandle, title: &str, body: &str) {
    toast::show(app, "info", toast::ToastCategory::General, title, body);
}

pub(crate) fn notify_error(app: &AppHandle, body: &str) {
    toast::show(app, "error", toast::ToastCategory::General, "Capcove — Error", body);
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "linux")]
    std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");

    // Logs to stderr (dev) and a file (readable in a packaged build) — see
    // `logging::init`. Tray "Open logs" opens that file.
    logging::init();
    win_util::set_app_user_model_id();
    if !win_util::acquire_single_instance() {
        return;
    }
    tauri::Builder::default()
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_filter(|label| {
                    label != "overlay" && !label.starts_with("overlay-")
                        && label != "rec-hud" && label != "wheel" && label != "toast"
                        && label != "recorder" && label != "recorder-frame"
                })
                .build(),
        )
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--autostart"]),
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(Pending::default())
        .manage(Arc::new(commands::update::PendingUpdate::default()))
        .invoke_handler(tauri::generate_handler![
            toast::toast_ready,
            recording::hud::get_hud_badges,
            commands::app::window_ready,
            commands::app::has_builtin_credentials,
            commands::app::is_store_screenshot_mode,
            commands::app::platform_capabilities,
            commands::app::get_settings,
            commands::app::save_settings,
            commands::app::get_drive_status,
            commands::app::connect_drive,
            commands::app::disconnect_drive,
            commands::app::connect_youtube,
            commands::app::disconnect_youtube,
            commands::app::cancel_drive_connect,
            commands::app::get_youtube_channel,
            commands::app::get_youtube_live_info,
            commands::app::list_drive_folders,
            commands::app::sync_now,
            commands::app::pick_folder,
            commands::app::pick_exe_file,
            commands::app::get_app_icon,
            commands::app::open_settings,
            commands::app::open_logs,
            commands::app::get_is_elevated,
            commands::app::is_packaged_install,
            commands::app::request_admin,
            commands::app::capability_statuses,
            commands::app::request_capability,
            commands::overlay_cmd::get_overlay_image,
            commands::overlay_cmd::get_overlay_setup,
            commands::overlay_cmd::area_selected,
            commands::overlay_cmd::set_area_live_mode,
            commands::overlay_cmd::reopen_overlay_live,
            commands::overlay_cmd::window_selected,
            commands::overlay_cmd::overlay_cancel,
            commands::overlay_cmd::overlay_ready,
            commands::overlay_cmd::set_native_highlight,
            commands::overlay_cmd::main_ready,
            library::get_offline_ops_count,
            library::upload_items,
            library::get_storage_info,
            library::clear_app_cache,
            library::get_cache_breakdown,
            library::clear_cache_categories,
            library::get_reclaimable_files,
            library::delete_local_copies,
            library::copy_text,
            library::open_url,
            translate::translate_text,
            wheel::wheel_action,
            wheel::open_wheel,
            wheel::wheel_closed,
            sound::preview_sound_effect,
            sound::pick_sound_file,
            sound::list_windows_sounds,
            recorder::open_recorder,
            recorder::recorder_window_ready,
            recorder::recorder_set_mode,
            recorder::recorder_resume_area,
            recorder::recorder_open_mode,
            recorder::recorder_current_mode,
            recorder::recorder_current_window_target,
            recorder::recorder_area_drag_begin,
            recorder::recorder_area_drag_end,
            recorder::recorder_pick_window,
            recorder::recorder_picker_ready,
            recorder::recorder_cancel_picker,
            recorder::recorder_pick_window_select,
            recorder::recorder_list_window_thumbs,
            recorder::recorder_track_window,
            recorder::recorder_start,
            recorder::recorder_set_opacity,
            recorder::recorder_minimize,
            recorder::recorder_close,
            commands::games::list_games,
            commands::games::set_game_enabled,
            commands::games::set_game_overrides,
            commands::games::get_game_overrides,
            commands::games::get_current_game,
            commands::games::add_custom_game,
            commands::games::inspect_exe_file,
            commands::games::remove_custom_game,
            commands::games::remove_custom_game_group,
            commands::games::sync_games,
            commands::games::fetch_game_icon,
            commands::games::fetch_game_cover,
            sync::get_transfers,
            sync::toggle_sync_pause,
            sync::clear_sync_queue,
            commands::update::get_app_version,
            commands::update::check_for_update,
            commands::update::get_pending_update,
            commands::update::download_and_install_update,
            commands::update::get_release_history,
            #[cfg(windows)]
            commands::recording::start_window_recording,
            #[cfg(windows)]
            commands::recording::stop_recording,
            #[cfg(windows)]
            commands::recording::cancel_recording,
            #[cfg(windows)]
            commands::recording::get_recording_status,
            #[cfg(windows)]
            commands::recording::get_crash_recovery_result,
            #[cfg(windows)]
            commands::recording::pause_recording,
            #[cfg(windows)]
            commands::recording::get_recording_paused,
            #[cfg(windows)]
            commands::recording::toggle_local_recording,
            #[cfg(windows)]
            commands::recording::toggle_live_streaming,
            #[cfg(windows)]
            commands::recording::list_available_encoders,
            #[cfg(windows)]
            commands::recording::resolve_auto_encoder,
            #[cfg(windows)]
            commands::recording::list_audio_devices,
            #[cfg(windows)]
            commands::recording::list_audio_apps,
            #[cfg(windows)]
            commands::recording::start_replay_buffer,
            #[cfg(windows)]
            commands::recording::stop_replay_buffer,
            #[cfg(windows)]
            commands::recording::save_replay,
            #[cfg(windows)]
            commands::recording::get_replay_buffer_status,
            #[cfg(windows)]
            commands::recording::get_replay_crash_recovery_result,
            #[cfg(windows)]
            commands::recording::recover_replay_buffer_crash,
            #[cfg(windows)]
            commands::recording::discard_replay_buffer_crash,
            commands::recording::get_pending_clip_result,
            commands::recording::confirm_pending_clip,
            commands::recording::discard_pending_clip,
            #[cfg(windows)]
            commands::video_editor::probe_video,
            #[cfg(windows)]
            commands::video_editor::export_edit,
            #[cfg(windows)]
            commands::video_editor::export_trim_clip,
            #[cfg(windows)]
            commands::video_editor::prepare_edit_audio,
            #[cfg(windows)]
            commands::video_editor::pick_video_file,
            #[cfg(windows)]
            commands::video_editor::render_waveform,
            #[cfg(windows)]
            commands::video_editor::upload_video_to_youtube,
            #[cfg(windows)]
            video_thumb::list_videos,
            video_thumb::get_cached_videos,
            #[cfg(windows)]
            video_thumb::read_video_thumbnail,
            #[cfg(windows)]
            video_thumb::get_video_waveform,
            #[cfg(windows)]
            video_thumb::get_video_waveform_range,
            #[cfg(windows)]
            video_thumb::get_video_metadata,
            #[cfg(windows)]
            video_thumb::get_video_details,
            #[cfg(windows)]
            video_thumb::ensure_playable_video,
            #[cfg(windows)]
            video_thumb::open_videos_folder,
            #[cfg(windows)]
            video_thumb::recordings_root_display,
            #[cfg(windows)]
            video_thumb::open_game_folder,
            #[cfg(windows)]
            video_thumb::open_recording_folder,
            #[cfg(windows)]
            video_thumb::delete_video,
            video_thumb::delete_drive_copy,
            video_thumb::download_video_from_drive,
            #[cfg(windows)]
            video_thumb::get_deletion_log,
            #[cfg(windows)]
            video_thumb::clear_deletion_log,
            #[cfg(windows)]
            video_thumb::get_storage_summary_result,
            #[cfg(windows)]
            video_thumb::ack_deletion_summary,
            #[cfg(windows)]
            video_thumb::open_recycle_bin,
            video_thumb::read_drive_video_thumbnail,
            #[cfg(windows)]
            video_thumb::open_item,
            #[cfg(windows)]
            video_thumb::reveal_item,
            #[cfg(windows)]
            video_thumb::list_tags,
            #[cfg(windows)]
            video_thumb::save_tags,
            #[cfg(windows)]
            video_thumb::set_video_tags,
            #[cfg(windows)]
            video_thumb::set_video_favorite,
            #[cfg(windows)]
            video_thumb::create_recording_folder,
            #[cfg(windows)]
            video_thumb::rename_recording_folder,
            #[cfg(windows)]
            video_thumb::delete_recording_folder,
            #[cfg(windows)]
            video_thumb::update_recording_folder_rules,
        ])
        .setup(|app| {
            // Tray-only on macOS: avoids a Dock/Cmd+Tab entry, and sidesteps a
            // quirk where focusing any window brings every window of the app
            // (including a backgrounded gallery) to the front.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let config_dir = app.path().app_config_dir()?;
            #[cfg(all(windows, debug_assertions))]
            let (config_dir, store_screenshots_mode) = if store_screenshots::requested() {
                let dir = store_screenshots::temp_config_dir();
                store_screenshots::prepare_temp_config(&dir);
                (dir, true)
            } else {
                (config_dir, false)
            };
            #[cfg(not(all(windows, debug_assertions)))]
            let _store_screenshots_mode = false;

            let store = Arc::new(ConfigStore::load(config_dir.clone()));
            app.manage(store.clone());
            #[cfg(all(windows, debug_assertions))]
            let drive = Arc::new(if store_screenshots_mode {
                DriveClient::new_isolated(config_dir.clone())
            } else {
                DriveClient::new(config_dir.clone())
            });
            #[cfg(not(all(windows, debug_assertions)))]
            let drive = Arc::new(DriveClient::new(config_dir.clone()));
            app.manage(drive.clone());
            app.manage(Arc::new(SyncState::load(config_dir.clone())));
            app.manage(Arc::new(meta::MetaStore::load(config_dir.clone())));
            app.manage(Arc::new(tag::TagStore::load(config_dir.clone())));
            #[cfg(windows)]
            app.manage(Arc::new(deletion_log::DeletionLogStore::load(config_dir.clone())));
            #[cfg(windows)]
            app.manage(Arc::new(video_thumb::StorageSummaryState::default()));
            app.manage(Arc::new(video_thumb::VideoListCache::default()));
            app.manage(Arc::new(games_db::GamesDb::load(config_dir.clone())));
            app.manage(Arc::new(icon_cache::IconCache::new(&config_dir)));
            app.manage(library::LibraryCache::default());
            #[cfg(target_os = "macos")]
            app.manage(ScreenPermission(std::sync::atomic::AtomicU8::new(0)));
            #[cfg(windows)]
            app.manage(Arc::new(recording::RecordingManager::default()));
            #[cfg(windows)]
            app.manage(Arc::new(recording::replay_buffer::ReplayBufferManager::default()));
            #[cfg(windows)]
            app.manage(recording::encoder::AutoEncoderCache::default());
            #[cfg(windows)]
            app.manage(recording::encoder::EncoderListCache::default());
            #[cfg(windows)]
            app.manage(Arc::new(recording::CrashRecoveryState::default()));
            #[cfg(windows)]
            app.manage(Arc::new(recording::replay_buffer::ReplayCrashRecoveryState::default()));
            app.manage(Arc::new(recording::replay_buffer::PendingClipState::default()));
            win_util::start_single_instance_listener(app.handle().clone());
            if let Some(icon) = app.default_window_icon() {
                let rgba = icon.rgba().to_vec();
                let (w, h) = (icon.width(), icon.height());
                let icon_path = config_dir.join("notification-icon.ico");
                std::thread::spawn(move || {
                    if let Some(img) = image::RgbaImage::from_raw(w, h, rgba) {
                        let resized = image::imageops::resize(&img, 256, 256, image::imageops::FilterType::Lanczos3);
                        let _ = resized.save(&icon_path);
                    }
                    win_util::register_notification_aumid(&icon_path);
                });
            }
            // Validate stored Drive token in background before sync starts.
            // Network error → keep tokens (offline). Auth error → clear tokens (logged out).
            {
                let drive_ref = drive.clone();
                let settings  = store.get();
                let cid  = settings.effective_google_client_id().to_string();
                let csec = settings.effective_google_client_secret().to_string();
                tauri::async_runtime::spawn(async move {
                    drive_ref.validate_on_startup(&cid, &csec).await;
                });
            }
            sync::start(app.handle());
            tray::build_tray(app.handle())?;
            tray::register_hotkeys(app.handle());
            // Toast overlay up front — built lazily it loses the very first
            // notification to its own page-load race (see toast::preload).
            toast::preload(app.handle());
            // ffmpeg/ffprobe ship as loose files next to the exe, in a
            // normally user-writable install dir — verify they haven't been
            // swapped for something else before anything gets a chance to
            // run them (see `integrity.rs`'s doc comment for the threat this
            // guards against). A toast here, not a blocking dialog: the rest
            // of the app (gallery, settings, editor) still works fine
            // without a trusted ffmpeg, only recording/export do not.
            #[cfg(windows)]
            if integrity::ffmpeg_sidecar(app.handle()).is_err() {
                notify_error(app.handle(), "ffmpeg/ffprobe failed an integrity check — recording and export are disabled until you reinstall Capcove.");
            }
            // Resolve borderless-capture access up front so the first recording
            // doesn't pay the consent round-trip — see `borderless_capture_granted`.
            // Only checks the cached OS decision (never prompts): the actual
            // consent prompt is only ever shown from the frontend's explainer
            // modal or its titlebar warning icon, not silently at startup.
            #[cfg(windows)]
            {
                tauri::async_runtime::spawn_blocking(|| win_util::capability_status(win_util::CapabilityKind::BorderlessCapture));
            }
            // Probe available encoders up front (each is a real ffmpeg dry-run)
            // so the settings/onboarding UIs read them from cache instantly
            // instead of waiting on every open.
            #[cfg(windows)]
            {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let _ = recording::encoder::cached_available_encoders(&app_handle).await;
                    let _ = recording::resolve_encoder(&app_handle, &config::EncoderChoice::Auto).await;
                });
            }
            #[cfg(windows)]
            {
                // A marker left over from an active recording means the app
                // itself was killed/crashed mid-recording last time, not a
                // normal stop/quit — see `recording::check_crash_recovery`.
                recording::check_crash_recovery(app.handle());
                // Must run before the auto-start below: that call
                // unconditionally wipes the segment directory this is
                // looking for — see its own doc comment.
                recording::replay_buffer::stage_replay_buffer_crash_recovery(app.handle());
                let rb_settings = store.get().video.replay_buffer.clone();
                if rb_settings.enabled {
                    let app_handle = app.handle().clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) = recording::replay_buffer::start_replay_buffer(&app_handle).await {
                            log::warn!("failed to auto-start replay buffer: {e}");
                        }
                    });
                }
                // Game watcher: reacts to a detected game per
                // `replay_buffer.game_detect_mode` (clip buffer / full
                // recording / off). Known-game catalog refreshes alongside.
                games_db::spawn_refresh(app.handle());
                recording::game_detect::spawn_detection_loop(app.handle());
                recorder::spawn_window_thumb_cache_loop(app.handle().clone());
                // Accumulates every app seen playing audio since launch —
                // feeds the settings page's per-app track list.
                recording::audio_capture::spawn_session_watcher();
            }
            if !cfg!(debug_assertions) {
                let s = store.get();
                win_util::update_start_menu_shortcut(s.run_as_admin);
                if s.run_as_admin && win_util::is_elevated() {
                    // Ensure registry-based autostart is off; use Task Scheduler instead
                    let _ = app.autolaunch().disable();
                    if s.autostart {
                        if let Err(e) = win_util::create_admin_autostart() {
                            log::warn!("failed to create admin autostart task on startup: {e}");
                        }
                    }
                } else if s.autostart && !s.run_as_admin {
                    let _ = app.autolaunch().enable();
                }
            }
            let launched_at_boot = std::env::args().any(|a| a == "--autostart");
            let gallery_will_open = !launched_at_boot && store.get().start_with_gallery;
            if gallery_will_open {
                tray::show_main(app.app_handle());
            }
            if store.get().auto_update && !win_util::is_packaged() {
                let update_app = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    use tauri_plugin_updater::UpdaterExt;
                    let Ok(updater) = update_app.updater() else { return };
                    if let Ok(Some(update)) = updater.check().await {
                        let version = update.version.clone();
                        let pending = update_app.state::<Arc<commands::update::PendingUpdate>>();
                        *pending.0.lock().await = Some(update);
                        let _ = update_app.emit("update-available", version.clone());
                        // No gallery window to surface the modal in — at least
                        // let the user know; it'll be waiting next time they
                        // open the gallery from the tray (see get_pending_update).
                        if !gallery_will_open {
                            notify(
                                &update_app,
                                "Capcove",
                                &format!("Update available: v{version}. Open Capcove from the tray to install."),
                            );
                        }
                    }
                });
            }
            #[cfg(all(windows, debug_assertions))]
            if store_screenshots_mode {
                store_screenshots::run(app.handle().clone());
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // macOS: stay tray-only (Accessory, no Dock icon) while no real
            // window is open, but switch to Regular so open windows show up
            // in Cmd+Tab.
            #[cfg(target_os = "macos")]
            if matches!(event, WindowEvent::Focused(_) | WindowEvent::Destroyed | WindowEvent::CloseRequested { .. }) {
                let app = window.app_handle();
                let closing_label = matches!(event, WindowEvent::Destroyed).then(|| window.label());
                // Existence, not visibility: a minimized window still counts as
                // "present". Excludes the window that just got Destroyed, which
                // is still in the map for this event but on its way out.
                let has_real_window = app.webview_windows().keys().any(|label| {
                    !label.starts_with("overlay") && label != "loading" && Some(label.as_str()) != closing_label
                });
                let _ = app.set_activation_policy(if has_real_window {
                    tauri::ActivationPolicy::Regular
                } else {
                    tauri::ActivationPolicy::Accessory
                });
            }
            if window.label() == "recorder-frame" {
                match event {
                    WindowEvent::Moved(pos) => recorder::on_frame_moved(&window.app_handle(), pos.x, pos.y),
                    WindowEvent::Resized(_) => recorder::reposition_bar(&window.app_handle()),
                    _ => {}
                }
            }
            if window.label() == "recorder" {
                match event {
                    WindowEvent::Moved(pos) => recorder::on_bar_moved(&window.app_handle(), pos.x, pos.y),
                    // Clicking its taskbar button to restore from minimized
                    // doesn't reliably un-minimize an always-on-top, borderless
                    // window on Windows — the click still activates/focuses it,
                    // so force the un-minimize explicitly rather than trusting
                    // the OS to have already done it.
                    WindowEvent::Focused(true) => {
                        let _ = window.unminimize();
                    }
                    _ => {}
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to start tauri application")
        .run(|_app, event| {
            if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}
