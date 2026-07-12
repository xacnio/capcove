use crate::config::ConfigStore;
use crate::drive::DriveClient;
use crate::meta::MetaStore;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_opener::OpenerExt;

/// No-op cache stub kept because `sync.rs`/`tray.rs` still call `.clear()`
/// after operations that used to invalidate a now-removed cache.
#[derive(Default)]
pub struct LibraryCache;

impl LibraryCache {
    pub fn clear(&self) {}
}

/// Extracts a Unix timestamp from a screenshot filename. Recognizes Capcove's
/// own format plus common macOS/Android/Windows screenshot naming schemes,
/// falling back to scanning for a date+time pattern anywhere in the name.
pub(crate) fn parse_timestamp_from_filename(name: &str) -> Option<i64> {
    use chrono::NaiveDateTime;

    let stem = std::path::Path::new(name).file_stem()?.to_str()?;

    let try_parse = |s: &str, fmt: &str| -> Option<i64> {
        NaiveDateTime::parse_from_str(s, fmt)
            .ok()
            .map(|dt| dt.and_utc().timestamp())
    };

    // 1. Capcove: "2026-06-11_16-36-58" (ignore milliseconds if present)
    if stem.len() >= 19 && stem.is_char_boundary(19) {
        if let Some(ts) = try_parse(&stem[..19], "%Y-%m-%d_%H-%M-%S") {
            return Some(ts);
        }
    }

    // 2. macOS " at " format:
    //    "Screenshot 2026-06-11 at 16.36.58"
    //    "Screen Shot 2026-06-11 at 01.36.58.564 AM"
    if let Some(at_idx) = stem.find(" at ") {
        if stem.is_char_boundary(at_idx) && stem.is_char_boundary(at_idx + 4) {
            let before = stem[..at_idx].trim();
            let rest   = stem[at_idx + 4..].trim();
            // Date: last 10 characters of before are "YYYY-MM-DD"
            if before.len() >= 10 && before.is_char_boundary(before.len() - 10) {
                let date_str = &before[before.len() - 10..];
                let tokens: Vec<&str> = rest.split_whitespace().collect();
                if let Some(raw_time) = tokens.first() {
                    // If there's a third dot, trim milliseconds
                    let time_str = {
                        let third_dot = raw_time.match_indices('.').nth(2).map(|(i, _)| i);
                        if let Some(pos) = third_dot { &raw_time[..pos] } else { raw_time }
                    };
                    let ampm = tokens.get(1).map(|s| s.to_ascii_uppercase());
                    // If AM/PM present, use 12-hour format
                    if matches!(ampm.as_deref(), Some("AM") | Some("PM")) {
                        let combined = format!("{} {} {}", date_str, time_str, ampm.unwrap());
                        if let Some(ts) = try_parse(&combined, "%Y-%m-%d %I.%M.%S %p") {
                            return Some(ts);
                        }
                    }
                    // 24-hour
                    let combined = format!("{} {}", date_str, time_str);
                    if let Some(ts) = try_parse(&combined, "%Y-%m-%d %H.%M.%S") {
                        return Some(ts);
                    }
                }
            }
        }
    }

    // 3. Look after underscore (Screenshot_... formats)
    if let Some(uidx) = stem.find('_') {
        if stem.is_char_boundary(uidx + 1) {
            let after = &stem[uidx + 1..];
            // Android compact: "20260611-163658"
            if after.len() >= 15 && after.is_char_boundary(15) {
                if let Some(ts) = try_parse(&after[..15], "%Y%m%d-%H%M%S") {
                    return Some(ts);
                }
            }
            // Android alt: "2026-06-11-16-36-58"
            if after.len() >= 19 && after.is_char_boundary(19) {
                if let Some(ts) = try_parse(&after[..19], "%Y-%m-%d-%H-%M-%S") {
                    return Some(ts);
                }
            }
        }
    }

    // 4. Compact "20260611_163658"
    if stem.len() >= 15 && stem.is_char_boundary(15) {
        if let Some(ts) = try_parse(&stem[..15], "%Y%m%d_%H%M%S") {
            return Some(ts);
        }
    }

    // 5. General scan: find "YYYY-MM-DD" anywhere in the filename
    //    then look for a time separated by space/dash/colon
    let bytes = stem.as_bytes();
    let mut i = 0usize;
    while i + 10 <= bytes.len() {
        if !stem.is_char_boundary(i) || !stem.is_char_boundary(i + 10) {
            i += 1;
            continue;
        }
        // "YYYY-MM-DD" pattern: digit(4) - digit(2) - digit(2)
        if bytes[i+4] == b'-' && bytes[i+7] == b'-'
            && bytes[i..i+4].iter().all(|b| b.is_ascii_digit())
            && bytes[i+5..i+7].iter().all(|b| b.is_ascii_digit())
            && bytes[i+8..i+10].iter().all(|b| b.is_ascii_digit())
        {
            let date_str = &stem[i..i + 10];
            let rest = stem[i + 10..].trim_start_matches(|c: char| c == ' ' || c == '_' || c == 'T');

            // "HH-MM-SS" or "HH:MM:SS"
            if rest.len() >= 8 && rest.is_char_boundary(8) {
                for sep in ['-', ':'] {
                    let cand = format!("{} {}", date_str,
                        &rest[..8].replace(sep, ":"));
                    if let Some(ts) = try_parse(&cand, "%Y-%m-%d %H:%M:%S") {
                        return Some(ts);
                    }
                }
            }
            // "HHMMSS" (6 consecutive digits)
            if rest.len() >= 6 && rest.is_char_boundary(6) && rest[..6].chars().all(|c| c.is_ascii_digit()) {
                let cand = format!("{} {}", date_str, &rest[..6]);
                if let Some(ts) = try_parse(&cand, "%Y-%m-%d %H%M%S") {
                    return Some(ts);
                }
            }
        }
        i += 1;
    }

    None
}

// ---------------------------------------------------------------------------
// Offline Drive operation queue — persists ops that couldn't run without network
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone)]
struct OfflineOp {
    op: String,       // "delete"
    drive_id: String,
    name: String,
}

fn offline_ops_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok()
        .map(|d| d.join("offline_ops.json"))
}

fn load_offline_ops(app: &AppHandle) -> Vec<OfflineOp> {
    let Some(path) = offline_ops_path(app) else { return Vec::new() };
    let Ok(bytes) = std::fs::read(&path) else { return Vec::new() };
    serde_json::from_slice::<Vec<OfflineOp>>(&bytes).unwrap_or_default()
}

fn save_offline_ops(app: &AppHandle, ops: &[OfflineOp]) {
    let Some(path) = offline_ops_path(app) else { return };
    if let Ok(json) = serde_json::to_vec(ops) {
        let _ = std::fs::write(path, json);
    }
}

/// Called on startup and after Drive reconnect — executes any queued offline ops.
pub async fn drain_offline_ops(app: &AppHandle) {
    let drive = app.state::<Arc<DriveClient>>();
    if !drive.is_connected() {
        return;
    }
    let ops = load_offline_ops(app);
    if ops.is_empty() {
        return;
    }
    let config = app.state::<Arc<ConfigStore>>();
    let s = config.get();
    let cid  = s.effective_google_client_id().to_string();
    let csec = s.effective_google_client_secret().to_string();
    let mut remaining = Vec::new();
    let mut drained = 0usize;
    for op in ops {
        match op.op.as_str() {
            "delete" => {
                match drive.delete_file(&cid, &csec, &op.drive_id).await {
                    Ok(()) => {
                        log::info!("Offline delete drained: {}", op.name);
                        drained += 1;
                    }
                    Err(e) => {
                        log::warn!("Offline delete drain failed for {}: {e}", op.name);
                        remaining.push(op);
                    }
                }
            }
            _ => { /* unknown op — discard */ }
        }
    }
    save_offline_ops(app, &remaining);
    if drained > 0 {
        let _ = app.emit("library-changed", ());
    }
}
#[tauri::command]
pub fn get_offline_ops_count(app: AppHandle) -> usize {
    load_offline_ops(&app).len()
}
#[tauri::command]
pub async fn upload_items(app: AppHandle, paths: Vec<String>) -> Result<(), String> {
    {
        let drive = app.state::<Arc<DriveClient>>();
        if !drive.is_connected() {
            return Err("Google Drive not connected".into());
        }
    }
    let path_bufs: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    // force=true: this is the gallery's explicit "Upload to Drive" action,
    // which must work even in Manual sync mode.
    crate::sync::inspect_and_enqueue_background(app, path_bufs, true);
    Ok(())
}

#[derive(Serialize)]
pub struct StorageInfo {
    pub local_bytes: u64,
    pub drive_limit: Option<u64>,
    pub drive_usage: u64,
    pub cache_bytes: u64,
    /// Bytes used by the recordings folder's clips (kind == "clip") and
    /// full recordings (everything else), respectively.
    pub clips_bytes: u64,
    pub recordings_bytes: u64,
    /// Total/free space of the drive the recordings folder lives on —
    /// `None` if it couldn't be determined (non-Windows, or the query failed).
    pub disk_total: Option<u64>,
    pub disk_free: Option<u64>,
    /// The actual effective recordings path, since `Settings.video.recordings_dir`
    /// is often empty (meaning "use the default").
    pub resolved_dir: String,
}

#[cfg(windows)]
fn disk_space(path: &Path) -> Option<(u64, u64)> {
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    use windows::core::HSTRING;

    // GetDiskFreeSpaceExW wants an existing path — walk up to the nearest
    // ancestor that exists (the recordings dir may not have been created
    // yet on a fresh install).
    let existing = path.ancestors().find(|p| p.exists())?;
    let wide = HSTRING::from(existing.as_os_str());
    let mut free_to_caller = 0u64;
    let mut total = 0u64;
    let mut free_total = 0u64;
    unsafe {
        GetDiskFreeSpaceExW(
            &wide,
            Some(&mut free_to_caller),
            Some(&mut total),
            Some(&mut free_total),
        )
        .ok()?;
    }
    Some((total, free_to_caller))
}

#[cfg(not(windows))]
fn disk_space(_path: &Path) -> Option<(u64, u64)> {
    None
}

/// Every disposable cache directory, tagged with a stable id the cleanup UI
/// selects by. All hold only re-derivable data — video thumbnails/waveforms
/// (re-extracted via ffmpeg), remuxed "playable" copies, downloaded Drive
/// thumbnails, and extracted app icons — so any is safe to wipe at any time.
fn cache_dirs_tagged(app: &AppHandle) -> Vec<(&'static str, PathBuf)> {
    let mut out = Vec::new();
    if let Ok(c) = app.path().app_cache_dir() {
        out.push(("video_thumbs", c.join("video_thumbs")));
        out.push(("waveforms", c.join("video_waveforms")));
        out.push(("playable", c.join("playable_fix")));
        out.push(("drive_thumbs", c.join("drive_video_thumbs")));
    }
    if let Ok(cfg) = app.path().app_config_dir() {
        out.push(("icons", cfg.join("icon_cache")));
    }
    // Editor audio-preview extractions live under the system temp dir, not
    // app_cache_dir (see `commands::video_editor`) — re-extracted on demand,
    // so just as disposable as the rest.
    out.push(("editor_previews", std::env::temp_dir().join("dev.xacnio.capcove").join("editor_preview")));
    out
}

fn cache_dirs(app: &AppHandle) -> Vec<PathBuf> {
    cache_dirs_tagged(app).into_iter().map(|(_, p)| p).collect()
}

/// Recursive — the video-thumbnail/waveform caches mirror the recordings'
/// own `<Game>/<Folder>/` subdirectory structure, so a flat read undercounts.
fn dir_size(dir: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    rd.flatten()
        .map(|e| match e.file_type() {
            Ok(ft) if ft.is_dir() => dir_size(&e.path()),
            _ => e.metadata().map(|m| m.len()).unwrap_or(0),
        })
        .sum()
}

/// For a name-keyed cache dir, the source recording's relative name a cache
/// file belongs to (strip the cache suffix) — or `None` for dirs keyed by
/// something other than the recording name (Drive thumbs by file id, icons by
/// app name), which have no per-recording orphan to detect.
fn orphan_source_name(cache_id: &str, rel: &str) -> Option<String> {
    match cache_id {
        "video_thumbs" => rel.strip_suffix(".jpg").or_else(|| rel.strip_suffix(".dur")).map(str::to_string),
        "waveforms" => rel.strip_suffix(".png").map(str::to_string),
        "playable" => rel.strip_suffix(".fixed.mp4").map(str::to_string),
        _ => None,
    }
}

/// Cache files left behind by recordings that no longer exist locally (deleted,
/// moved, or renamed). `skip` names dirs already being fully cleared, so their
/// orphans aren't counted/removed twice. Returns (bytes, file paths).
fn collect_orphans(app: &AppHandle, recordings_dir: &Path, skip: &std::collections::HashSet<&str>) -> (u64, Vec<PathBuf>) {
    let mut bytes = 0u64;
    let mut files = Vec::new();
    for (id, dir) in cache_dirs_tagged(app) {
        if skip.contains(id) || orphan_source_name(id, "").is_none() && !matches!(id, "video_thumbs" | "waveforms" | "playable") {
            continue;
        }
        let mut stack = vec![(dir.clone(), String::new())];
        while let Some((d, prefix)) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else { continue };
            for e in rd.flatten() {
                let fname = e.file_name().to_string_lossy().into_owned();
                let rel = if prefix.is_empty() { fname.clone() } else { format!("{prefix}/{fname}") };
                match e.file_type() {
                    Ok(ft) if ft.is_dir() => stack.push((e.path(), rel)),
                    _ => {
                        if let Some(src) = orphan_source_name(id, &rel) {
                            if !recordings_dir.join(&src).exists() {
                                bytes += e.metadata().map(|m| m.len()).unwrap_or(0);
                                files.push(e.path());
                            }
                        }
                    }
                }
            }
        }
    }
    (bytes, files)
}

#[tauri::command]
pub async fn get_storage_info(
    app: AppHandle,
    config: State<'_, Arc<ConfigStore>>,
    drive: State<'_, Arc<DriveClient>>,
    meta: State<'_, Arc<MetaStore>>,
) -> Result<StorageInfo, String> {
    let settings = config.get();
    let dir = settings.resolved_recordings_dir();

    let mut local_bytes = 0u64;
    let mut clips_bytes = 0u64;
    let mut recordings_bytes = 0u64;
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for entry in rd.flatten() {
            // Recording folders are handled in the pass below, so their
            // bytes aren't double-counted here.
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let Ok(fs_meta) = entry.metadata() else { continue };
            let size = fs_meta.len();
            local_bytes += size;
            let is_clip = entry.path().file_name().and_then(|n| n.to_str())
                .and_then(|name| meta.get(name))
                .and_then(|m| m.kind)
                .as_deref() == Some("clip");
            if is_clip { clips_bytes += size } else { recordings_bytes += size };
        }
    }
    // Recordings under a game and/or recording-folder subdirectory (see
    // `RecordingFolder`) only show up via `list_video_files`'s recursion.
    for (name, path) in crate::video_thumb::list_video_files(&dir) {
        if !name.contains('/') {
            continue; // already counted at the root, above
        }
        let Ok(fs_meta) = std::fs::metadata(&path) else { continue };
        let size = fs_meta.len();
        local_bytes += size;
        let is_clip = meta.get(&name).and_then(|m| m.kind).as_deref() == Some("clip");
        if is_clip { clips_bytes += size } else { recordings_bytes += size };
    }

    let (drive_limit, drive_usage) = if drive.is_connected() {
        drive
            .storage_quota(settings.effective_google_client_id(), settings.effective_google_client_secret())
            .await
            .unwrap_or((None, 0))
    } else {
        (None, 0)
    };

    let cache_bytes = cache_dirs(&app).iter().map(|d| dir_size(d)).sum();
    let (disk_total, disk_free) = match disk_space(&dir) {
        Some((total, free)) => (Some(total), Some(free)),
        None => (None, None),
    };

    Ok(StorageInfo {
        local_bytes, drive_limit, drive_usage, cache_bytes, clips_bytes, recordings_bytes,
        disk_total, disk_free, resolved_dir: dir.to_string_lossy().into_owned(),
    })
}

/// Wipes the disposable thumbnail/icon caches and returns the number of bytes freed.
/// Everything removed here is re-downloaded or re-extracted on demand.
#[tauri::command]
pub fn clear_app_cache(app: AppHandle) -> Result<u64, String> {
    let mut freed = 0u64;
    for dir in cache_dirs(&app) {
        freed += dir_size(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
    }
    Ok(freed)
}

#[derive(Serialize)]
pub struct CacheCategory {
    /// One of `cache_dirs_tagged`'s ids, plus the synthetic `"unused"`.
    pub id: String,
    pub bytes: u64,
}

/// Per-category cache sizes for the cleanup UI, so each can be cleared on its
/// own. `"unused"` is the subset of the name-keyed caches whose source
/// recording is gone — it overlaps the per-dir totals (it's a subset), and is
/// offered as its own "just reclaim the orphans" action.
#[tauri::command]
pub fn get_cache_breakdown(app: AppHandle, config: State<'_, Arc<ConfigStore>>) -> Vec<CacheCategory> {
    let recordings_dir = config.get().resolved_recordings_dir();
    let mut out: Vec<CacheCategory> = cache_dirs_tagged(&app)
        .into_iter()
        .map(|(id, dir)| CacheCategory { id: id.to_string(), bytes: dir_size(&dir) })
        .collect();
    let (orphan_bytes, _) = collect_orphans(&app, &recordings_dir, &std::collections::HashSet::new());
    out.push(CacheCategory { id: "unused".into(), bytes: orphan_bytes });
    out
}

/// Clears the selected cache categories, returning bytes freed. A full-dir
/// category wipes its whole directory; `"unused"` removes only orphaned files
/// from the dirs NOT already being fully cleared (so nothing double-counts).
#[tauri::command]
pub fn clear_cache_categories(app: AppHandle, config: State<'_, Arc<ConfigStore>>, categories: Vec<String>) -> Result<u64, String> {
    let recordings_dir = config.get().resolved_recordings_dir();
    let dirs = cache_dirs_tagged(&app);
    let mut freed = 0u64;
    let full: std::collections::HashSet<&str> = dirs
        .iter()
        .map(|(id, _)| *id)
        .filter(|id| categories.iter().any(|c| c == id))
        .collect();
    for (_, dir) in dirs.iter().filter(|(id, _)| full.contains(id)) {
        freed += dir_size(dir);
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::create_dir_all(dir);
    }
    if categories.iter().any(|c| c == "unused") {
        let (bytes, files) = collect_orphans(&app, &recordings_dir, &full);
        for f in files {
            let _ = std::fs::remove_file(f);
        }
        freed += bytes;
    }
    Ok(freed)
}

#[derive(Serialize)]
pub struct ReclaimFile {
    pub name: String,
    pub size: u64,
}

/// Local recordings that are also backed up to Drive — deleting the local copy
/// frees disk space while the file stays safe on Drive. Largest first.
#[tauri::command]
pub fn get_reclaimable_files(
    config: State<'_, Arc<ConfigStore>>,
    sync_state: State<'_, Arc<crate::sync::SyncState>>,
) -> Vec<ReclaimFile> {
    let dir = config.get().resolved_recordings_dir();
    let mut out: Vec<ReclaimFile> = crate::video_thumb::list_video_files(&dir)
        .into_iter()
        .filter(|(name, _)| sync_state.get_by_relative_name(name).is_some())
        .filter_map(|(name, path)| std::fs::metadata(&path).ok().map(|m| ReclaimFile { name, size: m.len() }))
        .collect();
    out.sort_by(|a, b| b.size.cmp(&a.size));
    out
}

/// Deletes the local copies of `names` (permanently — they're backed up on
/// Drive, the whole point) while keeping the Drive record, so each stays in
/// the gallery as a `drive_only` card. Only touches files that are actually
/// synced. Returns bytes freed.
#[tauri::command]
pub fn delete_local_copies(
    app: AppHandle,
    config: State<'_, Arc<ConfigStore>>,
    sync_state: State<'_, Arc<crate::sync::SyncState>>,
    names: Vec<String>,
) -> Result<u64, String> {
    let dir = config.get().resolved_recordings_dir();
    let mut freed = 0u64;
    for name in names {
        if sync_state.get_by_relative_name(&name).is_none() {
            continue; // never delete an unsynced local file here — that's data loss
        }
        let path = dir.join(&name);
        if let Ok(m) = std::fs::metadata(&path) {
            let size = m.len();
            if std::fs::remove_file(&path).is_ok() {
                freed += size;
            }
        }
    }
    let _ = app.emit("library-changed", ());
    Ok(freed)
}

#[tauri::command]
pub fn copy_text(app: AppHandle, text: String) -> Result<(), String> {
    app.clipboard().write_text(&text).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn open_url(app: AppHandle, url: String) -> Result<(), String> {
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())
}
