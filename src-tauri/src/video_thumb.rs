//! Lists recorded videos and generates/caches their thumbnails — kept separate
//! from `library.rs`'s screenshot pipeline so video browsing can't regress it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::config::ConfigStore;
use crate::deletion_log::{DeleteReason, DeletionLogEntry, DeletionLogStore};
use crate::drive::DriveClient;
use crate::meta::MetaStore;
use crate::sync::SyncState;
use crate::tag::{Tag, TagStore};

fn is_video(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()),
        Some(ref e) if e == "mp4" || e == "mkv" || e == "mov"
    )
}

/// Lists every video file under `dir`, recursing up to two levels of subfolders. Returns
/// `(relative_name, absolute_path)` pairs; `relative_name` is the key other video-file lookups use.
pub(crate) fn list_video_files(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    collect_video_files(dir, "", 0, &mut out);
    out
}

fn collect_video_files(dir: &Path, prefix: &str, depth: u8, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        let fname = entry.file_name().to_string_lossy().into_owned();
        if ft.is_file() {
            if is_video(&path) {
                let name = if prefix.is_empty() { fname } else { format!("{prefix}/{fname}") };
                out.push((name, path));
            }
        } else if ft.is_dir() && depth < 2 {
            let next_prefix = if prefix.is_empty() { fname } else { format!("{prefix}/{fname}") };
            collect_video_files(&path, &next_prefix, depth + 1, out);
        }
    }
}

/// The recording-folder segment of a relative name (e.g. `"Highlights"` from
/// `"Highlights/foo.mp4"`), or `None` if there isn't one. `game_hint` (from `VideoMeta::app`)
/// disambiguates the single-segment case by stripping the known game prefix first.
pub(crate) fn folder_name_of<'a>(relative_name: &'a str, game_hint: Option<&str>) -> Option<&'a str> {
    let rest = match game_hint {
        Some(g) => {
            let prefix = crate::drive::sanitize_filename(g);
            relative_name.strip_prefix(prefix.as_str()).and_then(|r| r.strip_prefix('/')).unwrap_or(relative_name)
        }
        None => relative_name,
    };
    rest.rsplit_once('/').map(|(folder, _)| folder)
}

/// The game segment of a relative name, when unambiguous: three segments
/// (`<Game>/<Folder>/<file>`) always means the first is the game. Used as a fallback when
/// `VideoMeta::app` is missing or stale.
pub(crate) fn game_name_of(relative_name: &str) -> Option<&str> {
    let parts: Vec<&str> = relative_name.split('/').collect();
    (parts.len() >= 3).then_some(parts[0])
}

/// The other half of `game_name_of`'s ambiguous two-segment case: checks whether the segment
/// matches a known game's sanitized name instead of assuming it's a `RecordingFolder`.
fn known_game_segment_of<'a>(relative_name: &'a str, games: &crate::games_db::GamesDb) -> Option<&'a str> {
    let first = relative_name.split_once('/').map(|(seg, _)| seg)?;
    games.is_known_game_folder(first).then_some(first)
}

/// Recording/clip filenames are `YYYY-MM-DD_HH-MM-SS-mmm[_N].ext`, stamped
/// with local wall-clock time once at creation (see `make_video_save_path`)
/// and never touched again — unlike filesystem mtime, which reflects when a
/// long recording *finishes* (can land on the next calendar day for a
/// session that runs past midnight, even though it clearly "happened" the
/// day it started), or a Drive-only card's `createdTime` (whenever it
/// happened to be uploaded). Parsing it back out gives the one date that's
/// actually authoritative, when the name follows this pattern at all.
fn timestamp_from_filename(relative_name: &str) -> Option<i64> {
    use chrono::TimeZone;
    let base = Path::new(relative_name).file_stem()?.to_str()?;
    // Replay-buffer clips get a "Clip_" prefix ahead of the timestamp (see
    // `make_video_save_path`) — strip it back off before the fixed-width
    // parse below.
    let base = base.strip_prefix("Clip_").unwrap_or(base);
    // Fixed-width "YYYY-MM-DD_HH-MM-SS-mmm" prefix (23 bytes) — ignores any
    // trailing `_N` collision-avoidance suffix `make_video_save_path` adds.
    let stamp = base.get(0..23)?;
    let dt = chrono::NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d_%H-%M-%S-%3f").ok()?;
    chrono::Local.from_local_datetime(&dt).single().map(|d| d.timestamp_millis())
}

/// Where a recording is actually filed, for the Folders view — purely from
/// its real relative path (local disk layout, or Drive's own reported
/// parent-folder chain for a `drive_only` card), never from `VideoMeta::app`.
/// That per-video tag is written once at record time and can go stale (a
/// folder gets renamed/reorganized, or — the common case for Drive — a file
/// was uploaded before folder-mirroring existed and still sits flat in
/// Drive's root): trusting it over the real location would show a card
/// "in" a game folder it doesn't actually live in.
fn location_hint<'a>(relative_name: &'a str, games: &crate::games_db::GamesDb) -> Option<&'a str> {
    game_name_of(relative_name).or_else(|| known_game_segment_of(relative_name, games))
}

/// Every local-file deletion (manual or auto-cleanup) funnels through here so
/// the "Use Recycle Bin" setting has exactly one place to matter.
///
/// Retries briefly on failure: deleting the file backing a video that was
/// just closed in the player can race a still-closing read handle on it
/// (Windows briefly reports a sharing violation until it's released), which
/// otherwise silently failed the very first delete attempt.
async fn delete_local_file(path: &Path, use_recycle_bin: bool) -> Result<(), String> {
    let mut last_err = String::new();
    for attempt in 0..5 {
        let result = if use_recycle_bin { trash::delete(path).map_err(|e| e.to_string()) } else { std::fs::remove_file(path).map_err(|e| e.to_string()) };
        match result {
            Ok(()) => return Ok(()),
            Err(e) => {
                last_err = e;
                if attempt < 4 {
                    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                }
            }
        }
    }
    Err(last_err)
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct VideoItem {
    pub name: String,
    pub local_path: String,
    pub modified: Option<u64>,
    pub size: Option<u64>,
    pub app: Option<String>,
    pub title: Option<String>,
    pub tags: Vec<String>,
    /// "clip" | "youtube_live" | None (= full recording) — see `VideoMeta::kind`.
    pub kind: Option<String>,
    /// Capture stats line for YouTube live entries ("1080p · 60 FPS · 12 Mbps").
    pub stream_info: Option<String>,
    /// Session length for YouTube live entries (stamped at stop).
    pub duration_secs: Option<u64>,
    /// Frame dimensions, currently only populated for `drive_only` cards
    /// (from Drive's `videoMediaMetadata`) — local files probe this
    /// on-demand via `get_video_metadata` instead.
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    /// Set once this file has been uploaded via "Upload to YouTube".
    pub youtube_video_id: Option<String>,
    /// Whether this file has been backed up to Google Drive. Drives the card
    /// menu's Drive-upload item.
    pub drive_synced: bool,
    /// Set when this card exists only on Drive (local copy deleted or never
    /// existed here). See `list_videos`'s local/Drive merge.
    #[serde(default)]
    pub drive_only: bool,
    /// The backing Drive file id — needed to download, thumbnail, or delete a `drive_only` card.
    #[serde(default)]
    pub drive_id: Option<String>,
    /// Recording folder name, derived from `name`'s path. Display only —
    /// folder names aren't unique across scopes, so navigation filters on
    /// `folder_id` instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    /// The specific `RecordingFolder::id` this recording resolves to, or
    /// `None` if not in any configured folder. Unambiguous even when a
    /// per-game and a global folder share the same name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder_id: Option<String>,
    #[serde(default)]
    pub favorite: bool,
}

/// Last full `list_videos` result, kept in the (persistent) Rust process so a
/// webview reload (Ctrl+R only reloads the frontend, not the backend) can
/// paint the gallery instantly from `get_cached_videos` while a fresh
/// `list_videos` refreshes in the background — no full-screen loading state,
/// no cold Drive scan on the critical path.
#[derive(Default)]
pub struct VideoListCache(std::sync::Mutex<Option<Vec<VideoItem>>>);

impl VideoListCache {
    fn get(&self) -> Option<Vec<VideoItem>> {
        self.0.lock().unwrap().clone()
    }
    fn set(&self, items: &[VideoItem]) {
        *self.0.lock().unwrap() = Some(items.to_vec());
    }
}

/// Instant, non-blocking read of the last-computed video list (see
/// `VideoListCache`). `None` only before the first `list_videos` of the
/// process's life — after that, always the most recent snapshot.
#[tauri::command]
pub fn get_cached_videos(cache: State<'_, Arc<VideoListCache>>) -> Option<Vec<VideoItem>> {
    cache.get()
}

/// `force`: bypasses `DriveClient::list_files`'s 30s cache — the breadcrumb's
/// refresh button passes `true` so an explicit "get me current data" click
/// doesn't silently no-op against a still-warm cache; every other reload
/// trigger (mount, `video-saved`, polling) leaves it `false`.
#[tauri::command]
pub async fn list_videos(
    app: AppHandle,
    config: State<'_, Arc<ConfigStore>>,
    meta: State<'_, Arc<MetaStore>>,
    sync_state: State<'_, Arc<SyncState>>,
    drive: State<'_, Arc<DriveClient>>,
    games: State<'_, Arc<crate::games_db::GamesDb>>,
    cache: State<'_, Arc<VideoListCache>>,
    force: Option<bool>,
) -> Result<Vec<VideoItem>, String> {
    if force.unwrap_or(false) {
        drive.clear_cache();
    }
    let settings = config.get();
    let dir = settings.resolved_recordings_dir();
    let mut out = Vec::new();
    let mut local_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let files = list_video_files(&dir);

    // Reattaches metadata orphaned by a path change (e.g. a folder rename) by matching
    // basenames, since recording filenames are timestamp-based and effectively unique.
    let file_name_set: std::collections::HashSet<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
    let all_meta = meta.get_all();
    let mut orphans_by_basename: std::collections::HashMap<String, (String, crate::meta::VideoMeta)> = std::collections::HashMap::new();
    for (key, m) in &all_meta {
        if m.is_virtual() || file_name_set.contains(key.as_str()) {
            continue;
        }
        if let Some(base) = Path::new(key).file_name().and_then(|f| f.to_str()) {
            orphans_by_basename.insert(base.to_string(), (key.clone(), m.clone()));
        }
    }
    let mut healed_meta: Vec<(String, String, crate::meta::VideoMeta)> = Vec::new();
    let mut healed_sync: Vec<(String, String)> = Vec::new();

    for (name, path) in files {
        let Ok(fs_meta) = std::fs::metadata(&path) else { continue };
        let modified = timestamp_from_filename(&name).map(|ms| ms as u64)
            .or_else(|| fs_meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64));
        let mut video_meta = meta.get(&name);
        // Same orphan recovery for the upload-tracking store, sharing the old key.
        let mut old_key_for_sync: Option<&str> = None;
        if video_meta.is_none() {
            if let Some(base) = Path::new(&name).file_name().and_then(|f| f.to_str()) {
                if let Some((old_key, m)) = orphans_by_basename.get(base) {
                    video_meta = Some(m.clone());
                    healed_meta.push((old_key.clone(), name.clone(), m.clone()));
                    old_key_for_sync = Some(old_key.as_str());
                }
            }
        }
        let drive_id = sync_state.get_by_relative_name(&name).or_else(|| old_key_for_sync.and_then(|k| sync_state.get_by_relative_name(k)));
        if let (Some(_), Some(old_key)) = (&drive_id, old_key_for_sync) {
            if sync_state.get_by_relative_name(&name).is_none() {
                healed_sync.push((old_key.to_string(), name.clone()));
            }
        }
        let game_hint = location_hint(&name, &games);
        let folder_name = folder_name_of(&name, game_hint);
        let folder_id = folder_name.and_then(|f| settings.folder_by_name_scoped(f, game_hint)).map(|f| f.id.clone());
        // Only show the folder chip when the segment resolves to a configured
        // `RecordingFolder` — an unrecognized segment falls back to no chip.
        let folder_display = if folder_id.is_some() { folder_name } else { None };
        local_names.insert(crate::drive::sanitize_filename(&name).to_ascii_lowercase());
        out.push(VideoItem {
            drive_synced: drive_id.is_some(),
            drive_only: false,
            drive_id,
            local_path: path.to_string_lossy().into_owned(),
            modified,
            size: Some(fs_meta.len()),
            app: game_hint.map(str::to_string),
            title: video_meta.as_ref().and_then(|m| m.title.clone()),
            kind: video_meta.as_ref().and_then(|m| m.kind.clone()),
            stream_info: video_meta.as_ref().and_then(|m| m.stream_info.clone()),
            duration_secs: video_meta.as_ref().and_then(|m| m.duration_secs),
            width: None,
            height: None,
            youtube_video_id: video_meta.as_ref().and_then(|m| m.youtube_video_id.clone()),
            tags: video_meta.as_ref().map(|m| m.tags.clone()).unwrap_or_default(),
            favorite: video_meta.as_ref().map(|m| m.favorite).unwrap_or(false),
            folder: folder_display.map(str::to_string),
            folder_id,
            name,
        });
    }
    // Persist the recovery under each file's correct, current key and drop
    // the stale one — makes this a one-time fix per file instead of
    // re-discovering the same orphan via basename on every listing.
    for (old_key, new_key, m) in healed_meta {
        meta.set(new_key, m);
        meta.remove(&old_key);
    }
    for (old_key, new_key) in healed_sync {
        if let Some(id) = sync_state.get_by_relative_name(&old_key) {
            sync_state.record(crate::drive::sanitize_filename(&new_key), id);
            sync_state.remove_by_relative_name(&old_key);
        }
    }
    // Virtual entries have no local file (a YouTube live session, or a
    // recording whose local copy was deleted after upload) — both link
    // straight to the video on YouTube.
    for (name, m) in meta.get_all() {
        if !m.is_virtual() {
            continue;
        }
        let Some(id) = m.youtube_video_id.clone().or_else(|| name.strip_prefix("yt_").map(str::to_string)) else {
            continue; // no id to link to — shouldn't happen, but nothing to show
        };
        local_names.insert(crate::drive::sanitize_filename(&name).to_ascii_lowercase());
        out.push(VideoItem {
            local_path: format!("https://youtube.com/watch?v={id}"),
            modified: m.created.map(|c| c as u64 * 1000),
            size: None,
            app: m.app.clone(),
            title: m.title.clone(),
            kind: m.kind.clone(),
            stream_info: m.stream_info.clone(),
            duration_secs: m.duration_secs,
            width: None,
            height: None,
            youtube_video_id: Some(id),
            drive_synced: false,
            drive_only: false,
            drive_id: None,
            tags: m.tags.clone(),
            folder: None,
            folder_id: None,
            favorite: m.favorite,
            name,
        });
    }

    // Drive-only cards: anything Drive has that isn't shown above (local copy
    // deleted, or never existed here). Best-effort — offline or a listing
    // error just means this pass contributes nothing.
    if drive.is_connected() {
        let cid = settings.effective_google_client_id().to_string();
        let csec = settings.effective_google_client_secret().to_string();
        let folder = settings.drive_folder_name.clone();
        // Only meaningfully fires more than once when `list_files`'s 30s
        // cache is cold and there are enough files to span multiple pages
        // (1000/page) — the gallery listens for this to show scan progress
        // instead of a plain "Loading" while a large Drive library is paged.
        let progress_app = app.clone();
        let result = drive
            .list_files(&cid, &csec, &folder, move |files_so_far, page| {
                let _ = progress_app.emit("drive-scan-progress", serde_json::json!({ "files": files_so_far, "page": page }));
            })
            .await;
        if let Ok((files, _)) = result {
            // `rel` mirrors the local relative-path format, folder segments included.
            for (f, rel) in files {
                let key = crate::drive::sanitize_filename(&rel).to_ascii_lowercase();
                if local_names.contains(&key) {
                    continue; // already represented locally (or as a YouTube virtual card)
                }
                local_names.insert(key); // guard against duplicate Drive listings
                let video_meta = meta.get(&rel);
                let modified = timestamp_from_filename(&rel).map(|ms| ms as u64)
                    .or_else(|| f.created_time.as_deref()
                        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                        .map(|d| d.timestamp_millis() as u64));
                let size = f.size.as_deref().and_then(|s| s.parse::<u64>().ok());
                let game_hint = location_hint(&rel, &games);
                let folder_name = folder_name_of(&rel, game_hint);
                let folder_id = folder_name.and_then(|fname| settings.folder_by_name_scoped(fname, game_hint)).map(|f| f.id.clone());
                let folder_display = if folder_id.is_some() { folder_name } else { None };
                let drive_duration_secs = f.video_media_metadata.as_ref()
                    .and_then(|vm| vm.duration_millis.as_deref())
                    .and_then(|ms| ms.parse::<u64>().ok())
                    .map(|ms| ms / 1000);
                out.push(VideoItem {
                    drive_synced: true,
                    drive_only: true,
                    drive_id: Some(f.id.clone()),
                    local_path: f.web_view_link.clone().unwrap_or_default(),
                    modified,
                    size,
                    app: game_hint.map(str::to_string),
                    title: video_meta.as_ref().and_then(|m| m.title.clone()),
                    kind: video_meta.as_ref().and_then(|m| m.kind.clone()),
                    stream_info: video_meta.as_ref().and_then(|m| m.stream_info.clone()),
                    duration_secs: video_meta.as_ref().and_then(|m| m.duration_secs).or(drive_duration_secs),
                    width: f.video_media_metadata.as_ref().and_then(|vm| vm.width),
                    height: f.video_media_metadata.as_ref().and_then(|vm| vm.height),
                    youtube_video_id: video_meta.as_ref().and_then(|m| m.youtube_video_id.clone()),
                    tags: video_meta.as_ref().map(|m| m.tags.clone()).unwrap_or_default(),
                    favorite: video_meta.map(|m| m.favorite).unwrap_or(false),
                    folder: folder_display.map(str::to_string),
                    folder_id,
                    name: rel,
                });
            }
        }
    }

    out.sort_by_key(|v| std::cmp::Reverse(v.modified.unwrap_or(0)));
    cache.set(&out);
    Ok(out)
}

#[tauri::command]
pub fn list_tags(store: State<'_, Arc<TagStore>>) -> Vec<Tag> {
    store.get_all()
}

#[tauri::command]
pub fn save_tags(store: State<'_, Arc<TagStore>>, tags: Vec<Tag>) {
    store.save(tags);
}

#[tauri::command]
pub fn set_video_tags(meta: State<'_, Arc<MetaStore>>, name: String, tags: Vec<String>) {
    let mut entry = meta.get(&name).unwrap_or_default();
    entry.tags = tags;
    meta.set(name, entry);
}

#[tauri::command]
pub fn set_video_favorite(meta: State<'_, Arc<MetaStore>>, name: String, favorite: bool) {
    let mut entry = meta.get(&name).unwrap_or_default();
    entry.favorite = favorite;
    meta.set(name, entry);
}

/// Windows' reserved device names — disallowed as a file/folder name outright
/// (case-insensitive, and regardless of any extension after a `.`), even
/// though `sanitize_filename` lets them through since none of their
/// characters are individually illegal.
const WINDOWS_RESERVED_NAMES: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL",
    "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
    "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Rejects folder names Windows can't actually create, and — for a root-level
/// folder (`game` is `None`) — one that collides with a known game's own
/// auto-created subfolder there, since a same-named custom folder would sit
/// on top of (or be indistinguishable from, being case-insensitive) it and
/// break the path-based lookups that tell games and custom folders apart.
fn validate_folder_name(name: &str, game: Option<&str>, games: &crate::games_db::GamesDb) -> Result<(), String> {
    let stem = name.split('.').next().unwrap_or(name);
    if WINDOWS_RESERVED_NAMES.iter().any(|r| stem.eq_ignore_ascii_case(r)) {
        return Err(format!("\"{name}\" is a reserved Windows name and can't be used for a folder"));
    }
    if game.is_none() && games.is_known_game_folder(name) {
        return Err(format!("\"{name}\" is already used as a game's own recordings folder"));
    }
    Ok(())
}

/// Creates a new recording folder: a real subdirectory plus a rules entry in
/// `Settings::recording_folders`. Errors on an empty or duplicate name, since folder names
/// double as the path segment other lookups match against.
#[tauri::command]
pub fn create_recording_folder(app: AppHandle, config: State<'_, Arc<ConfigStore>>, games: State<'_, Arc<crate::games_db::GamesDb>>, name: String, game: Option<String>) -> Result<crate::config::RecordingFolder, String> {
    let name = crate::drive::sanitize_filename(name.trim());
    if name.is_empty() {
        return Err("Folder name can't be empty".into());
    }
    validate_folder_name(&name, game.as_deref(), &games)?;
    let game = game.filter(|g| !g.trim().is_empty());
    let mut settings = config.get();
    // Case-insensitive: Windows folder names are, so "GTA" and "gta" would
    // silently collide into the same physical directory despite looking like
    // two distinct entries in the settings list.
    if settings.recording_folders.iter().any(|f| f.name.eq_ignore_ascii_case(&name) && f.game == game) {
        return Err("A folder with that name already exists".into());
    }
    let mut dir = settings.resolved_recordings_dir();
    if let Some(g) = &game {
        dir = dir.join(crate::drive::sanitize_filename(g));
        // Same best-effort Explorer icon `recording::prepare` sets at session start.
        let icons = app.state::<Arc<crate::icon_cache::IconCache>>();
        if let Some(bytes) = crate::games_db::best_icon_bytes(&app, &icons, g) {
            let _ = std::fs::create_dir_all(&dir);
            crate::folder_icon::ensure_folder_icon(&dir, &bytes);
        }
    }
    dir = dir.join(&name);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let folder = crate::config::RecordingFolder {
        id: crate::recording::uuid_v4(),
        name,
        game,
        auto_delete_days: None,
        never_upload_to_drive: false,
        always_keep: false,
    };
    settings.recording_folders.push(folder.clone());
    config.save(settings).map_err(|e| e.to_string())?;
    let _ = app.emit("settings-changed", ());
    Ok(folder)
}

/// Renames a folder: moves its physical subdirectory (so path-based rule
/// lookups keep matching) and updates its settings entry. Anything
/// referencing it by `id` keeps working automatically.
#[tauri::command]
pub fn rename_recording_folder(
    app: AppHandle,
    config: State<'_, Arc<ConfigStore>>,
    games: State<'_, Arc<crate::games_db::GamesDb>>,
    meta: State<'_, Arc<MetaStore>>,
    sync_state: State<'_, Arc<SyncState>>,
    id: String,
    new_name: String,
) -> Result<(), String> {
    let new_name = crate::drive::sanitize_filename(new_name.trim());
    if new_name.is_empty() {
        return Err("Folder name can't be empty".into());
    }
    let mut settings = config.get();
    let Some(game) = settings.recording_folders.iter().find(|f| f.id == id).map(|f| f.game.clone()) else {
        return Err("Folder not found".into());
    };
    validate_folder_name(&new_name, game.as_deref(), &games)?;
    if settings.recording_folders.iter().any(|f| f.name.eq_ignore_ascii_case(&new_name) && f.game == game && f.id != id) {
        return Err("A folder with that name already exists".into());
    }
    let Some(folder) = settings.recording_folders.iter_mut().find(|f| f.id == id) else {
        return Err("Folder not found".into());
    };
    let old_name = std::mem::replace(&mut folder.name, new_name.clone());
    let mut base = settings.resolved_recordings_dir();
    // Same relative-path prefix `list_video_files` builds affected videos' metadata keys from.
    let mut prefix_base = String::new();
    if let Some(g) = &game {
        let sanitized = crate::drive::sanitize_filename(g);
        base = base.join(&sanitized);
        prefix_base = format!("{sanitized}/");
    }
    let (old_path, new_path) = (base.join(&old_name), base.join(&new_name));
    if old_path.exists() {
        std::fs::rename(&old_path, &new_path).map_err(|e| e.to_string())?;
        let old_prefix = format!("{prefix_base}{old_name}");
        let new_prefix = format!("{prefix_base}{new_name}");
        meta.rekey_prefix(&old_prefix, &new_prefix);
        sync_state.rekey_prefix(&old_prefix, &new_prefix);

        // Mirror the rename on Drive too, otherwise the next upload creates a
        // second folder under the new name instead of renaming the existing
        // one. Runs in the background; no-ops if disconnected or never uploaded.
        let app2 = app.clone();
        let old_name2 = old_name.clone();
        let new_name2 = new_name.clone();
        let game2 = game.clone();
        tauri::async_runtime::spawn(async move {
            let drive = app2.state::<Arc<DriveClient>>();
            if !drive.is_connected() { return; }
            let config = app2.state::<Arc<ConfigStore>>();
            let settings = config.get();
            let cid = settings.effective_google_client_id().to_string();
            let csec = settings.effective_google_client_secret().to_string();
            let root_id = match drive.ensure_folder(&cid, &csec, &settings.drive_folder_name).await {
                Ok(id) => id,
                Err(e) => { log::warn!("Drive folder rename: could not resolve root folder: {e}"); return; }
            };
            let parent_segments: Vec<String> = game2.as_deref()
                .map(|g| vec![crate::drive::sanitize_filename(g)])
                .unwrap_or_default();
            if let Err(e) = drive.rename_nested_folder(&cid, &csec, &root_id, &parent_segments, &old_name2, &new_name2).await {
                log::warn!("Drive folder rename failed ({old_name2} -> {new_name2}): {e}");
            }
        });
    } else {
        std::fs::create_dir_all(&new_path).map_err(|e| e.to_string())?;
    }
    config.save(settings).map_err(|e| e.to_string())?;
    let _ = app.emit("settings-changed", ());
    Ok(())
}

/// Patches a folder's auto-cleanup rules (auto-delete-days/never-upload/
/// always-keep) in one shot, used by Settings -> Games' per-game folder section.
#[tauri::command]
pub fn update_recording_folder_rules(
    app: AppHandle,
    config: State<'_, Arc<ConfigStore>>,
    id: String,
    auto_delete_days: Option<u32>,
    never_upload_to_drive: bool,
    always_keep: bool,
) -> Result<(), String> {
    let mut settings = config.get();
    let Some(folder) = settings.recording_folders.iter_mut().find(|f| f.id == id) else {
        return Err("Folder not found".into());
    };
    folder.auto_delete_days = auto_delete_days;
    folder.never_upload_to_drive = never_upload_to_drive;
    folder.always_keep = always_keep;
    config.save(settings).map_err(|e| e.to_string())?;
    let _ = app.emit("settings-changed", ());
    Ok(())
}

/// Removes a folder's *rules* only — its recordings and physical subdirectory
/// are left untouched; new recordings go straight to the recordings root instead.
#[tauri::command]
pub fn delete_recording_folder(app: AppHandle, config: State<'_, Arc<ConfigStore>>, id: String) -> Result<(), String> {
    let mut settings = config.get();
    settings.recording_folders.retain(|f| f.id != id);
    config.save(settings).map_err(|e| e.to_string())?;
    let _ = app.emit("settings-changed", ());
    Ok(())
}

/// Auto-registers any real on-disk directory that isn't yet a known `RecordingFolder`, so a
/// folder made by hand still shows up as a tile. A game's own directory is skipped; only a
/// subfolder nested inside one becomes a game-scoped `RecordingFolder`.
pub fn discover_recording_folders(app: &AppHandle) {
    let config = app.state::<Arc<ConfigStore>>();
    let games = app.state::<Arc<crate::games_db::GamesDb>>();
    let meta = app.state::<Arc<MetaStore>>();
    let mut settings = config.get();
    let root = settings.resolved_recordings_dir();
    let Ok(root_entries) = std::fs::read_dir(&root) else { return };

    // A game folder isn't always in the catalog/custom list yet (or ever) —
    // any file already tagged with that app name is just as good a signal.
    let tagged_app_names: std::collections::HashSet<String> = meta.get_all()
        .values()
        .filter_map(|m| m.app.clone())
        .collect();
    let is_game_dir = |name: &str| games.is_known_game_folder(name) || tagged_app_names.contains(name);

    let mut new_folders = Vec::new();
    for entry in root_entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_dir() { continue; }
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_game_dir(&name) {
            let Ok(sub_entries) = std::fs::read_dir(entry.path()) else { continue };
            for sub in sub_entries.flatten() {
                let Ok(sub_ft) = sub.file_type() else { continue };
                if !sub_ft.is_dir() { continue; }
                let sub_name = sub.file_name().to_string_lossy().into_owned();
                if settings.folder_by_name_scoped(&sub_name, Some(&name)).is_none() {
                    new_folders.push(crate::config::RecordingFolder {
                        id: crate::recording::uuid_v4(),
                        name: sub_name,
                        game: Some(name.clone()),
                        auto_delete_days: None,
                        never_upload_to_drive: false,
                        always_keep: false,
                    });
                }
            }
        } else if settings.folder_by_name_scoped(&name, None).is_none() {
            new_folders.push(crate::config::RecordingFolder {
                id: crate::recording::uuid_v4(),
                name,
                game: None,
                auto_delete_days: None,
                never_upload_to_drive: false,
                always_keep: false,
            });
        }
    }

    if new_folders.is_empty() { return; }
    settings.recording_folders.extend(new_folders);
    if config.save(settings).is_ok() {
        let _ = app.emit("settings-changed", ());
    }
}

fn thumb_cache_dir(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_cache_dir().ok().map(|d| d.join("video_thumbs"))
}

/// Generates (if not already cached) a 320px-wide JPEG thumbnail for `name`, returning its
/// cache file path. Seeks to 1s in to skip the black opening frame (falls back to 0s for very
/// short clips); the cache path mirrors `name`'s folder structure and is created explicitly.
pub(crate) async fn ensure_thumbnail_cached(app: &AppHandle, name: &str) -> Result<PathBuf, String> {
    let source = app.state::<Arc<ConfigStore>>().get().resolved_recordings_dir().join(name);
    let Some(cache_dir) = thumb_cache_dir(app) else { return Err("no cache dir".into()) };
    let cache_path = cache_dir.join(format!("{name}.jpg"));
    if cache_path.exists() {
        return Ok(cache_path);
    }
    if let Some(parent) = cache_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let cmd = crate::integrity::ffmpeg_sidecar(app).map_err(|e| e.to_string())?;
    let output = cmd
        .args([
            "-y", "-ss", "1",
            "-i", &source.to_string_lossy(),
            "-frames:v", "1", "-vf", "scale=320:-1",
            &cache_path.to_string_lossy(),
        ])
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if !output.status.success() || !cache_path.exists() {
        // Likely a clip shorter than 1s — retry from the very first frame.
        let cmd = crate::integrity::ffmpeg_sidecar(app).map_err(|e| e.to_string())?;
        let output = cmd
            .args([
                "-y", "-i", &source.to_string_lossy(),
                "-frames:v", "1", "-vf", "scale=320:-1",
                &cache_path.to_string_lossy(),
            ])
            .output()
            .await
            .map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(format!("ffmpeg thumbnail failed: {}", String::from_utf8_lossy(&output.stderr)));
        }
    }
    Ok(cache_path)
}

/// Pre-warms thumbnails for every local recording without one cached yet, so opening the
/// gallery doesn't wait on an ffmpeg extraction. Sequential, since there can be thousands.
pub async fn generate_missing_thumbnails(app: &AppHandle) {
    let dir = app.state::<Arc<ConfigStore>>().get().resolved_recordings_dir();
    for (name, _) in list_video_files(&dir) {
        if let Err(e) = ensure_thumbnail_cached(app, &name).await {
            log::warn!("thumbnail pre-warm failed for '{name}': {e}");
        }
    }
}

/// Generates (and caches) a JPEG thumbnail for `name`, returning it as base64. Seeks to 1s in
/// to skip the black opening frame; falls back to 0s for very short clips.
#[tauri::command]
pub async fn read_video_thumbnail(app: AppHandle, name: String) -> Result<String, String> {
    let cache_path = ensure_thumbnail_cached(&app, &name).await?;
    use base64::Engine;
    let bytes = std::fs::read(&cache_path).map_err(|e| e.to_string())?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

fn waveform_cache_dir(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_cache_dir().ok().map(|d| d.join("video_waveforms"))
}

/// Generates (if not already cached) a static waveform PNG for `name`'s entire audio track via
/// ffmpeg's `showwavespic` filter. White on black by design — the frontend composites it with
/// `mix-blend-mode: screen` to fake an alpha channel.
pub(crate) async fn ensure_waveform_cached(app: &AppHandle, name: &str) -> Result<PathBuf, String> {
    let source = app.state::<Arc<ConfigStore>>().get().resolved_recordings_dir().join(name);
    let Some(cache_dir) = waveform_cache_dir(app) else { return Err("no cache dir".into()) };
    let cache_path = cache_dir.join(format!("{name}.png"));
    if cache_path.exists() {
        return Ok(cache_path);
    }
    if let Some(parent) = cache_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let cmd = crate::integrity::ffmpeg_sidecar(app).map_err(|e| e.to_string())?;
    let output = cmd
        .args([
            "-y",
            "-i", &source.to_string_lossy(),
            "-filter_complex", "showwavespic=s=1000x64:colors=white:scale=cbrt",
            "-frames:v", "1",
            &cache_path.to_string_lossy(),
        ])
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!("ffmpeg waveform failed: {}", String::from_utf8_lossy(&output.stderr)));
    }
    Ok(cache_path)
}

/// Generates (and caches) a waveform PNG for `name` (a recording filename,
/// resolved against `resolved_recordings_dir()`). Returns a base64 PNG.
#[tauri::command]
pub async fn get_video_waveform(app: AppHandle, name: String) -> Result<String, String> {
    let cache_path = ensure_waveform_cached(&app, &name).await?;
    use base64::Engine;
    let bytes = std::fs::read(&cache_path).map_err(|e| e.to_string())?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// Same idea, but rendered fresh for just `[start_ms, end_ms)` — the full
/// waveform PNG is a fixed 1000px wide regardless of duration, so stretching
/// a small crop of it to fill the trim tool's zoomed-in view just blows up a
/// handful of source pixels into a blank smear. Re-running `showwavespic`
/// against only the zoomed window gives it real detail to show instead.
///
/// Written to a temp file rather than piped through stdout — tauri-plugin-shell's
/// `Command::output()` reassembles stdout line by line (re-inserting a `\n`
/// after each chunk), which is fine for text but corrupts arbitrary binary
/// bytes like a PNG. The temp filename is unique per call (not a fixed
/// shared path) so two overlapping requests — e.g. React StrictMode's
/// double effect invocation in dev — can't race and hand back each other's
/// range; removed again once read.
#[tauri::command]
pub async fn get_video_waveform_range(app: AppHandle, name: String, start_ms: u64, end_ms: u64) -> Result<String, String> {
    if end_ms <= start_ms {
        return Err("End must be after start".into());
    }
    let source = app.state::<Arc<ConfigStore>>().get().resolved_recordings_dir().join(&name);
    let start_s = start_ms as f64 / 1000.0;
    let duration_s = (end_ms - start_ms) as f64 / 1000.0;
    let nonce: u64 = rand::random();
    let out_path = std::env::temp_dir().join(format!("capcove-waveform-zoom-{nonce:x}.png"));

    // `-t` has to sit before `-i` (an input option, seeking the source
    // together with `-ss`) rather than after — placed as an output option
    // it's silently ignored for a single-frame `image2`/`-frames:v 1`
    // output, and showwavespic ends up rendering everything from the seek
    // point to EOF squeezed into the same 1000px, not just this window.
    let cmd = crate::integrity::ffmpeg_sidecar(&app).map_err(|e| e.to_string())?;
    let output = cmd
        .args([
            "-y",
            "-ss", &format!("{start_s:.3}"),
            "-t", &format!("{duration_s:.3}"),
            "-i", &source.to_string_lossy(),
            "-filter_complex", "showwavespic=s=1000x64:colors=white:scale=cbrt",
            "-frames:v", "1",
            &out_path.to_string_lossy(),
        ])
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        let _ = std::fs::remove_file(&out_path);
        return Err(format!("ffmpeg waveform range failed: {}", String::from_utf8_lossy(&output.stderr)));
    }
    use base64::Engine;
    let bytes = std::fs::read(&out_path).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&out_path);
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

#[derive(Debug, Clone, Serialize)]
pub struct VideoMetadata {
    pub duration_secs: f64,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// `duration_secs,width,height` — width/height fields empty when ffprobe
/// couldn't find a video stream.
fn parse_metadata_cache(s: &str) -> Option<VideoMetadata> {
    let mut parts = s.trim().splitn(3, ',');
    let duration_secs = parts.next()?.parse::<f64>().ok()?;
    let width = parts.next().and_then(|p| p.parse::<u32>().ok());
    let height = parts.next().and_then(|p| p.parse::<u32>().ok());
    Some(VideoMetadata { duration_secs, width, height })
}

/// Duration/resolution lookups spawn an ffprobe process, which dominated
/// gallery load time when run per card on every open — so the value is
/// cached next to the thumbnails and only re-probed when the source file is
/// newer.
#[tauri::command]
pub async fn get_video_metadata(app: AppHandle, config: State<'_, Arc<ConfigStore>>, name: String) -> Result<VideoMetadata, String> {
    let source = config.get().resolved_recordings_dir().join(&name);
    let cache_path = thumb_cache_dir(&app).map(|d| d.join(format!("{name}.dur")));

    if let Some(c) = &cache_path {
        let fresh = match (std::fs::metadata(c).and_then(|m| m.modified()), std::fs::metadata(&source).and_then(|m| m.modified())) {
            (Ok(cm), Ok(sm)) => cm >= sm,
            _ => false,
        };
        if fresh {
            if let Some(m) = std::fs::read_to_string(c).ok().and_then(|s| parse_metadata_cache(&s)) {
                return Ok(m);
            }
        }
    }

    let cmd = crate::integrity::ffprobe_sidecar(&app).map_err(|e| e.to_string())?;
    let output = cmd
        .args([
            "-v", "error", "-select_streams", "v:0",
            "-show_entries", "format=duration:stream=width,height",
            "-of", "json",
            &source.to_string_lossy(),
        ])
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!("ffprobe failed: {}", String::from_utf8_lossy(&output.stderr)));
    }
    #[derive(serde::Deserialize, Default)]
    struct Stream { width: Option<u32>, height: Option<u32> }
    #[derive(serde::Deserialize, Default)]
    struct Format { duration: Option<String> }
    #[derive(serde::Deserialize, Default)]
    struct Probe { #[serde(default)] streams: Vec<Stream>, #[serde(default)] format: Format }
    let probe: Probe = serde_json::from_slice(&output.stdout).unwrap_or_default();
    let duration_secs = probe.format.duration.as_deref().and_then(|d| d.parse::<f64>().ok()).unwrap_or(0.0);
    let width = probe.streams.first().and_then(|s| s.width);
    let height = probe.streams.first().and_then(|s| s.height);

    if let Some(c) = &cache_path {
        if let Some(parent) = c.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let w = width.map(|w| w.to_string()).unwrap_or_default();
        let h = height.map(|h| h.to_string()).unwrap_or_default();
        let _ = std::fs::write(c, format!("{duration_secs},{w},{h}"));
    }
    Ok(VideoMetadata { duration_secs, width, height })
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct VideoProbeDetails {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    pub video_codec: Option<String>,
    pub video_bitrate_kbps: Option<u32>,
    pub audio_codec: Option<String>,
    pub audio_channels: Option<u32>,
    pub audio_sample_rate: Option<u32>,
    pub overall_bitrate_kbps: Option<u32>,
    pub container: Option<String>,
}

/// Parses ffprobe's `r_frame_rate`/similar "num/den" fraction fields.
fn parse_fraction(s: &str) -> Option<f64> {
    let (num, den) = s.split_once('/')?;
    let num: f64 = num.parse().ok()?;
    let den: f64 = den.parse().ok()?;
    if den == 0.0 { return None; }
    Some(num / den)
}

/// Heavier than `get_video_metadata` (full codec/bitrate/fps probe instead of
/// just duration+dimensions) — only invoked on demand when the player header
/// or details panel is actually opened, never per-card in the gallery grid.
#[tauri::command]
pub async fn get_video_details(app: AppHandle, config: State<'_, Arc<ConfigStore>>, name: String) -> Result<VideoProbeDetails, String> {
    let source = config.get().resolved_recordings_dir().join(&name);
    let cmd = crate::integrity::ffprobe_sidecar(&app).map_err(|e| e.to_string())?;
    let output = cmd
        .args([
            "-v", "error", "-of", "json",
            "-show_entries", "format=bit_rate,format_name:stream=codec_type,codec_name,width,height,r_frame_rate,bit_rate,channels,sample_rate",
            &source.to_string_lossy(),
        ])
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!("ffprobe failed: {}", String::from_utf8_lossy(&output.stderr)));
    }
    #[derive(serde::Deserialize, Default)]
    struct Stream {
        codec_type: Option<String>,
        codec_name: Option<String>,
        width: Option<u32>,
        height: Option<u32>,
        r_frame_rate: Option<String>,
        bit_rate: Option<String>,
        channels: Option<u32>,
        sample_rate: Option<String>,
    }
    #[derive(serde::Deserialize, Default)]
    struct Format { bit_rate: Option<String>, format_name: Option<String> }
    #[derive(serde::Deserialize, Default)]
    struct Probe { #[serde(default)] streams: Vec<Stream>, #[serde(default)] format: Format }
    let probe: Probe = serde_json::from_slice(&output.stdout).unwrap_or_default();
    let vstream = probe.streams.iter().find(|s| s.codec_type.as_deref() == Some("video"));
    let astream = probe.streams.iter().find(|s| s.codec_type.as_deref() == Some("audio"));

    Ok(VideoProbeDetails {
        width: vstream.and_then(|s| s.width),
        height: vstream.and_then(|s| s.height),
        fps: vstream.and_then(|s| s.r_frame_rate.as_deref()).and_then(parse_fraction),
        video_codec: vstream.and_then(|s| s.codec_name.clone()),
        video_bitrate_kbps: vstream.and_then(|s| s.bit_rate.as_deref()).and_then(|b| b.parse::<u64>().ok()).map(|b| (b / 1000) as u32),
        audio_codec: astream.and_then(|s| s.codec_name.clone()),
        audio_channels: astream.and_then(|s| s.channels),
        audio_sample_rate: astream.and_then(|s| s.sample_rate.as_deref()).and_then(|s| s.parse::<u32>().ok()),
        overall_bitrate_kbps: probe.format.bit_rate.as_deref().and_then(|b| b.parse::<u64>().ok()).map(|b| (b / 1000) as u32),
        container: probe.format.format_name,
    })
}

fn playable_cache_dir(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_cache_dir().ok().map(|d| d.join("playable_fix"))
}

/// Some files aren't actually valid MP4/MOV/MKV despite their extension, which the browser's
/// `<video>` demuxer refuses outright. Probes the real container and, if needed, remuxes a
/// fixed copy (stream copy, no re-encode) into the cache. Doesn't fix an unsupported codec.
#[tauri::command]
pub async fn ensure_playable_video(app: AppHandle, config: State<'_, Arc<ConfigStore>>, name: String) -> Result<String, String> {
    let source = config.get().resolved_recordings_dir().join(&name);
    if !source.exists() {
        return Err("file not found".into());
    }
    let original = source.to_string_lossy().into_owned();

    let Some(cache_dir) = playable_cache_dir(&app) else { return Ok(original) };
    let cache_path = cache_dir.join(format!("{name}.fixed.mp4"));
    if cache_path.exists() {
        let fresh = match (std::fs::metadata(&cache_path).and_then(|m| m.modified()), std::fs::metadata(&source).and_then(|m| m.modified())) {
            (Ok(cm), Ok(sm)) => cm >= sm,
            _ => false,
        };
        if fresh {
            return Ok(cache_path.to_string_lossy().into_owned());
        }
    }

    let Ok(cmd) = crate::integrity::ffprobe_sidecar(&app) else { return Ok(original) };
    let Ok(output) = cmd
        .args([
            "-v", "error", "-of", "json",
            "-show_entries", "format=format_name:stream=codec_type,codec_name,is_avc",
            &original,
        ])
        .output()
        .await
    else { return Ok(original) };
    if !output.status.success() {
        // Can't even probe it — hand the original to the player as before;
        // no worse off than if this check didn't exist.
        return Ok(original);
    }
    let probe: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_default();
    let format_name = probe["format"]["format_name"].as_str().unwrap_or("");
    let browser_safe_container = format_name.split(',').any(|f| matches!(f, "mov" | "mp4" | "m4a" | "3gp" | "3g2" | "mj2" | "matroska" | "webm"));
    let streams = probe["streams"].as_array().cloned().unwrap_or_default();
    let video_is_annexb = streams.iter().any(|s| s["codec_type"] == "video" && s["is_avc"] == "false");
    if browser_safe_container && !video_is_annexb {
        return Ok(original);
    }

    if let Some(parent) = cache_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let audio_is_aac = streams.iter().any(|s| s["codec_type"] == "audio" && s["codec_name"] == "aac");
    let mut args: Vec<String> = vec![
        "-y".into(), "-i".into(), original.clone(),
        "-map".into(), "0:v:0".into(), "-map".into(), "0:a:0?".into(),
        "-c".into(), "copy".into(),
    ];
    // Only when the audio is actually AAC — this bitstream filter errors out
    // applied to anything else.
    if audio_is_aac {
        args.push("-bsf:a".into());
        args.push("aac_adtstoasc".into());
    }
    args.push(cache_path.to_string_lossy().into_owned());

    let Ok(cmd) = crate::integrity::ffmpeg_sidecar(&app) else { return Ok(original) };
    let Ok(output) = cmd.args(args).output().await else { return Ok(original) };
    if !output.status.success() || !cache_path.exists() {
        log::warn!("ensure_playable_video: remux failed for '{name}': {}", String::from_utf8_lossy(&output.stderr));
        return Ok(original);
    }
    Ok(cache_path.to_string_lossy().into_owned())
}

#[tauri::command]
pub fn open_videos_folder(app: AppHandle, config: State<'_, Arc<ConfigStore>>) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let dir = config.get().resolved_recordings_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    app.opener().open_path(dir.to_string_lossy().to_string(), None::<&str>).map_err(|e| e.to_string())
}

/// The recordings root, formatted for display in the folder-explorer's
/// breadcrumb: forward slashes and the user's profile directory shortened to
/// `~`, regardless of platform — this is a label, never passed back to a
/// shell, so the styling doesn't need to match the OS's native path syntax.
#[tauri::command]
pub fn recordings_root_display(config: State<'_, Arc<ConfigStore>>) -> String {
    let dir = config.get().resolved_recordings_dir();
    let mut s = dir.to_string_lossy().replace('\\', "/");
    if let Some(home) = std::env::var_os("USERPROFILE").map(PathBuf::from) {
        let home_s = home.to_string_lossy().replace('\\', "/");
        if let Some(rest) = s.strip_prefix(&home_s) {
            s = format!("~{rest}");
        }
    }
    s.trim_end_matches('/').to_string()
}

/// Opens a game's own recordings subdirectory (see `recording::prepare`'s
/// `<root>/<sanitized game name>/...` layout) — the breadcrumb's game
/// segment, once inside a game, opens this instead of just navigating.
#[tauri::command]
pub fn open_game_folder(app: AppHandle, config: State<'_, Arc<ConfigStore>>, name: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let dir = config.get().resolved_recordings_dir().join(crate::drive::sanitize_filename(&name));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    app.opener().open_path(dir.to_string_lossy().to_string(), None::<&str>).map_err(|e| e.to_string())
}

/// Opens a specific `RecordingFolder`'s own subdirectory — nested under its
/// game's folder when scoped to one, directly under the root otherwise (see
/// `RecordingFolder::game` and `recording::prepare`'s layout).
#[tauri::command]
pub fn open_recording_folder(app: AppHandle, config: State<'_, Arc<ConfigStore>>, folder_id: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let settings = config.get();
    let folder = settings.folder_by_id(&folder_id).ok_or("folder not found")?;
    let mut dir = settings.resolved_recordings_dir();
    if let Some(game) = folder.game.as_deref() {
        dir = dir.join(crate::drive::sanitize_filename(game));
    }
    dir = dir.join(&folder.name);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    app.opener().open_path(dir.to_string_lossy().to_string(), None::<&str>).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_video(
    config: State<'_, Arc<ConfigStore>>,
    meta: State<'_, Arc<MetaStore>>,
    sync_state: State<'_, Arc<SyncState>>,
    drive: State<'_, Arc<DriveClient>>,
    name: String,
    // Shift+right-click's "delete permanently" — bypasses the Recycle Bin
    // regardless of the `use_recycle_bin` setting, for this one call only.
    permanent: Option<bool>,
) -> Result<(), String> {
    let path = config.get().resolved_recordings_dir().join(&name);
    if path.exists() {
        let use_recycle_bin = config.get().use_recycle_bin && !permanent.unwrap_or(false);
        delete_local_file(&path, use_recycle_bin).await?;
        // Uploaded to YouTube: keep a link-only card instead of forgetting
        // it — the recording is still watchable there. A second delete (on
        // that now-fileless card) falls through to full removal below.
        if let Some(mut m) = meta.get(&name) {
            if m.youtube_video_id.is_some() {
                m.kind = Some("youtube_only".to_string());
                meta.set(name, m);
                return Ok(());
            }
        }
        // Still backed up to Drive: keep the metadata/sync record — the
        // next `list_videos` re-surfaces it as a `drive_only` card instead
        // of it vanishing (this is "delete local copy", not "forget it").
        if sync_state.get_by_relative_name(&name).is_some() {
            return Ok(());
        }
    } else if let Some(drive_id) = sync_state.get_by_relative_name(&name) {
        // No local file: this IS the Drive-only card, so "delete" here
        // means actually deleting it from Drive, not just forgetting a link
        // (unlike the YouTube-only case, there's no second place it lives).
        let settings = config.get();
        let _ = drive
            .delete_file(settings.effective_google_client_id(), settings.effective_google_client_secret(), &drive_id)
            .await;
        sync_state.remove_by_relative_name(&name);
        // Evict it from the Drive list cache too, so a reload within the 30s
        // cache window doesn't re-surface it as a `drive_only` card.
        drive.forget_cached_file(&drive_id);
    }
    // Neither a local file nor a Drive record — already gone, or this is a
    // virtual entry (YouTube live/upload record) being removed from the list.
    meta.remove(&name);
    Ok(())
}

/// The inverse of "delete local copy": removes only the Drive backup, keeping the local file
/// untouched. No-op if there's no Drive record.
#[tauri::command]
pub async fn delete_drive_copy(
    config: State<'_, Arc<ConfigStore>>,
    sync_state: State<'_, Arc<SyncState>>,
    drive: State<'_, Arc<DriveClient>>,
    name: String,
) -> Result<(), String> {
    let Some(drive_id) = sync_state.get_by_relative_name(&name) else { return Ok(()) };
    let settings = config.get();
    drive
        .delete_file(settings.effective_google_client_id(), settings.effective_google_client_secret(), &drive_id)
        .await
        .map_err(|e| e.to_string())?;
    sync_state.remove_by_relative_name(&name);
    // Evict it from the Drive list cache too, so a reload within the 30s cache
    // window doesn't re-surface it as a `drive_only` card (Drive's own
    // `files.list` lags a few seconds behind this delete).
    drive.forget_cached_file(&drive_id);
    Ok(())
}

/// True if `name` (a `list_video_files`-style relative name) should never be
/// touched by any auto-delete mechanism: its folder is `always_keep`, or
/// it's a favorite and `Settings::keep_favorites` is on.
fn is_auto_delete_exempt(settings: &crate::config::Settings, meta: &MetaStore, name: &str) -> bool {
    let video_meta = meta.get(name);
    let game_hint = video_meta.as_ref().and_then(|m| m.app.as_deref()).or_else(|| game_name_of(name));
    if let Some(folder) = folder_name_of(name, game_hint) {
        if settings.folder_by_name_scoped(folder, game_hint).is_some_and(|f| f.always_keep) {
            return true;
        }
    }
    settings.keep_favorites && video_meta.is_some_and(|m| m.favorite)
}

/// `Some((used_bytes, limit_bytes))` when over the configured limit but
/// `auto_delete_oldest` is off, so nothing was actually cleaned up — surfaced
/// by `check_storage_startup_summary` as a startup warning.
pub struct OverLimitNoAutoDelete {
    pub used_bytes: u64,
    pub limit_bytes: u64,
}

/// Storage-limit auto-cleanup: when over `storage_limit_mb` and `auto_delete_oldest` is on,
/// deletes the oldest recordings (by mtime, skipping exempt ones) until back under the limit.
/// Also independently runs each folder's own `auto_delete_days` cutoff.
pub async fn enforce_storage_limit(app: &AppHandle) -> Option<OverLimitNoAutoDelete> {
    let config = app.state::<Arc<ConfigStore>>();
    let settings = config.get();
    let dir = settings.resolved_recordings_dir();
    let meta = app.state::<Arc<MetaStore>>();
    let sync_state = app.state::<Arc<SyncState>>();
    let drive = app.state::<Arc<DriveClient>>();
    let deletion_log = app.state::<Arc<DeletionLogStore>>();

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    // Per-folder age cutoffs — independent of the global limit below.
    for folder in settings.recording_folders.iter().filter(|f| !f.always_keep) {
        let Some(days) = folder.auto_delete_days else { continue };
        let cutoff_ms = now_ms.saturating_sub(days as u64 * 24 * 60 * 60 * 1000);
        for (name, path) in list_video_files(&dir) {
            let video_meta = meta.get(&name);
            let game_hint = video_meta.as_ref().and_then(|m| m.app.as_deref()).or_else(|| game_name_of(&name));
            if folder_name_of(&name, game_hint) != Some(folder.name.as_str()) {
                continue;
            }
            // Confirm this recording's actual folder match is this entry, not a same-named
            // folder scoped to a different game (see `folder_by_name_scoped`).
            if settings.folder_by_name_scoped(&folder.name, game_hint).map(|f| f.id.as_str()) != Some(folder.id.as_str()) {
                continue;
            }
            if settings.keep_favorites && video_meta.is_some_and(|m| m.favorite) {
                continue;
            }
            let Ok(fs_meta) = std::fs::metadata(&path) else { continue };
            let modified = fs_meta.modified().ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64)
                .unwrap_or(now_ms);
            if modified >= cutoff_ms {
                continue;
            }
            log::info!("folder '{}': auto-deleting '{name}' (older than {days}d)", folder.name);
            let size = fs_meta.len();
            let folder_name = folder.name.clone();
            match delete_video(config.clone(), meta.clone(), sync_state.clone(), drive.clone(), name.clone(), None).await {
                Ok(()) => deletion_log.record(name, size, DeleteReason::FolderAge { folder: folder_name, days }),
                Err(e) => log::warn!("folder auto-delete failed: {e}"),
            }
        }
    }

    let Some(limit_mb) = settings.storage_limit_mb else { return None };
    let limit_bytes = limit_mb.saturating_mul(1024 * 1024);

    // Computed regardless of `auto_delete_oldest` — a limit that's exceeded
    // while auto-delete is off still needs to be visible somewhere (the
    // startup summary), not just silently ignored here.
    let mut candidates: Vec<(String, u64, u64)> = Vec::new(); // (name, size, modified_ms)
    let mut total: u64 = 0;
    for (name, path) in list_video_files(&dir) {
        let Ok(fs_meta) = std::fs::metadata(&path) else { continue };
        total += fs_meta.len();
        if is_auto_delete_exempt(&settings, &meta, &name) {
            continue; // not a deletion candidate, but its bytes still count toward `total`
        }
        if settings.only_delete_long_recordings && meta.get(&name).and_then(|m| m.kind).as_deref() == Some("clip") {
            continue;
        }
        let modified = fs_meta.modified().ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        candidates.push((name, fs_meta.len(), modified));
    }
    if total <= limit_bytes {
        return None;
    }
    if !settings.auto_delete_oldest {
        return Some(OverLimitNoAutoDelete { used_bytes: total, limit_bytes });
    }
    candidates.sort_by_key(|c| c.2);

    for (name, size, _) in candidates {
        if total <= limit_bytes {
            break;
        }
        log::info!("storage limit: auto-deleting oldest recording '{name}' ({size} bytes) to get back under {limit_mb} MB");
        match delete_video(config.clone(), meta.clone(), sync_state.clone(), drive.clone(), name.clone(), None).await {
            Ok(()) => {
                total = total.saturating_sub(size);
                deletion_log.record(name, size, DeleteReason::StorageLimit);
            }
            Err(e) => log::warn!("storage limit: auto-delete failed: {e}"),
        }
    }
    None
}

/// Holds the most recent `check_storage_startup_summary` outcome so the
/// gallery can still pull it if it wasn't mounted (listening) when the event
/// fired — same startup-mount race as `recording::CrashRecoveryState`.
#[derive(Default)]
pub struct StorageSummaryState(std::sync::Mutex<Option<serde_json::Value>>);

impl StorageSummaryState {
    pub fn take(&self) -> Option<serde_json::Value> {
        self.0.lock().unwrap().take()
    }
}

/// Checked once at startup, right after `enforce_storage_limit`: surfaces
/// whatever it auto-deleted that the user hasn't seen a summary for yet
/// (`Settings::deletion_summary_acked_at`), plus a standing warning if the
/// local limit is currently exceeded with `auto_delete_oldest` off (nothing
/// was or will be cleaned up automatically in that case).
pub async fn check_storage_startup_summary(app: &AppHandle, over_limit: Option<OverLimitNoAutoDelete>) {
    let config = app.state::<Arc<ConfigStore>>();
    let settings = config.get();
    let log = app.state::<Arc<DeletionLogStore>>();

    let new_deletions: Vec<DeletionLogEntry> = log.get_all().into_iter()
        .filter(|e| e.deleted_at > settings.deletion_summary_acked_at)
        .collect();

    if new_deletions.is_empty() && over_limit.is_none() {
        return;
    }

    let payload = serde_json::json!({
        "new_deletions": if new_deletions.is_empty() { None } else { Some(new_deletions) },
        "over_limit": over_limit.map(|o| serde_json::json!({ "used_bytes": o.used_bytes, "limit_bytes": o.limit_bytes })),
        "use_recycle_bin": settings.use_recycle_bin,
    });

    if let Some(state) = app.try_state::<Arc<StorageSummaryState>>() {
        *state.0.lock().unwrap() = Some(payload.clone());
    }
    let _ = app.emit("storage-summary", payload);
}

/// Pull side of `check_storage_startup_summary` — see `StorageSummaryState`.
#[tauri::command]
pub fn get_storage_summary_result(app: AppHandle) -> Option<serde_json::Value> {
    app.state::<Arc<StorageSummaryState>>().take()
}

/// Marks every auto-deletion up to now as seen, so the startup summary only
/// ever shows genuinely new ones from here on.
#[tauri::command]
pub fn ack_deletion_summary(config: State<'_, Arc<ConfigStore>>) -> Result<(), String> {
    let mut settings = config.get();
    settings.deletion_summary_acked_at = chrono::Utc::now().timestamp();
    config.save(settings).map_err(|e| e.to_string())
}

/// Opens the Windows Recycle Bin — offered by the startup deletion summary
/// when `use_recycle_bin` is on, so a shown-in-error auto-delete is one click
/// from being restored.
#[tauri::command]
pub fn open_recycle_bin(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener().open_path("shell:RecycleBinFolder", None::<&str>).map_err(|e| e.to_string())
}

/// The log Settings -> Storage shows so the storage-limit/folder-age
/// auto-delete behavior is actually verifiable, not just a background log line.
#[tauri::command]
pub fn get_deletion_log(log: State<'_, Arc<DeletionLogStore>>) -> Vec<DeletionLogEntry> {
    log.get_all()
}

#[tauri::command]
pub fn clear_deletion_log(log: State<'_, Arc<DeletionLogStore>>) {
    log.clear();
}

/// Downloads a Drive-only card's file to the local recordings folder — the
/// gallery's play/edit/reveal actions call this first when a card has no
/// local file yet (see `useVideoOpenOrPlay`'s `openOrPlay`).
///
/// `modified_ms` is the same Drive `createdTime` already shown for the
/// `drive_only` card (see `VideoItem::modified` above) — stamped onto the
/// downloaded file's mtime so it keeps its place in the date-sorted list
/// instead of jumping to the top with today's date, the moment it becomes a
/// normal local file.
#[tauri::command]
pub async fn download_video_from_drive(
    app: AppHandle,
    config: State<'_, Arc<ConfigStore>>,
    drive: State<'_, Arc<DriveClient>>,
    engine: State<'_, Arc<crate::sync::SyncEngine>>,
    drive_id: String,
    name: String,
    modified_ms: Option<u64>,
) -> Result<String, String> {
    let settings = config.get();
    let dir = settings.resolved_recordings_dir();
    let dest = dir.join(&name);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    // Show downloads in the same transfer queue/panel as uploads (they went
    // straight to the network before, invisibly) — keyed by the file name so
    // a per-file progress row shows and can later flip to done/error.
    let transfers = engine.transfers_manager.clone();
    transfers.update_transfer(&app, name.clone(), "downloading".into(), None, Some((0, 0, 0)));
    let cid = settings.effective_google_client_id().to_string();
    let csec = settings.effective_google_client_secret().to_string();
    let (progress_app, progress_name, progress_transfers) = (app.clone(), name.clone(), transfers.clone());
    // Streamed straight to `dest` instead of buffering the whole recording in
    // memory first — this can be a multi-GB file.
    drive
        .download_file_to_path_with_progress(&cid, &csec, &drive_id, &dest, move |received, total, bps| {
            progress_transfers.update_transfer(&progress_app, progress_name.clone(), "downloading".into(), None, Some((received, total, bps)));
        })
        .await
        .map_err(|e| {
            transfers.update_transfer(&app, name.clone(), "error".into(), Some(e.to_string()), None);
            e.to_string()
        })?;
    if let Some(ms) = modified_ms {
        let ft = filetime::FileTime::from_unix_time((ms / 1000) as i64, ((ms % 1000) * 1_000_000) as u32);
        let _ = filetime::set_file_mtime(&dest, ft);
    }
    transfers.update_transfer(&app, name.clone(), "done".into(), None, None);
    let _ = app.emit("library-changed", ());
    Ok(dest.to_string_lossy().into_owned())
}

fn drive_video_thumb_cache_dir(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_cache_dir().ok().map(|d| d.join("drive_video_thumbs"))
}

/// Thumbnail for a `drive_only` card, fetched from Drive's `thumbnailLink` and disk-cached
/// like `read_video_thumbnail` since it's a network fetch, not a local ffmpeg probe.
#[tauri::command]
pub async fn read_drive_video_thumbnail(
    app: AppHandle,
    config: State<'_, Arc<ConfigStore>>,
    drive: State<'_, Arc<DriveClient>>,
    drive_id: String,
) -> Result<String, String> {
    let Some(cache_dir) = drive_video_thumb_cache_dir(&app) else { return Err("no cache dir".into()) };
    std::fs::create_dir_all(&cache_dir).map_err(|e| e.to_string())?;
    let cache_path = cache_dir.join(format!("{drive_id}.jpg"));

    if !cache_path.exists() {
        let settings = config.get();
        let bytes = drive
            .thumbnail(settings.effective_google_client_id(), settings.effective_google_client_secret(), &drive_id, 320)
            .await
            .map_err(|e| e.to_string())?;
        std::fs::write(&cache_path, &bytes).map_err(|e| e.to_string())?;
    }

    use base64::Engine;
    let bytes = std::fs::read(&cache_path).map_err(|e| e.to_string())?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// Opens a file with the OS default handler (e.g. the system video player).
#[tauri::command]
pub fn open_item(app: AppHandle, path: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener().open_path(path, None::<&str>).map_err(|e| e.to_string())
}

/// Reveals a file in the OS file manager (Explorer/Finder/whatever Linux DE is running).
#[tauri::command]
pub fn reveal_item(app: AppHandle, path: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let _ = app;
        std::process::Command::new("explorer")
            .args(["/select,", &path])
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        use tauri_plugin_opener::OpenerExt;
        let dir = std::path::Path::new(&path).parent().map(|p| p.to_string_lossy().to_string()).unwrap_or(path);
        app.opener().open_path(dir, None::<&str>).map_err(|e| e.to_string())
    }
}
