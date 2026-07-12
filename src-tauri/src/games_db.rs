//! Known-game executable catalog, sourced from a public game-detection catalog endpoint.
//! Game detection checks the foreground exe against this catalog first, falling back to the
//! fullscreen heuristic only for unknown exes. Cached on disk, refreshed at most once a week.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

const URL: &str = "https://discord.com/api/v9/applications/detectable";
const REFRESH_AFTER: Duration = Duration::from_secs(7 * 24 * 3600);

/// Build-time snapshot of the catalog, so a fresh install has the full game
/// list immediately, offline included. Superseded by the disk cache once
/// refresh/sync writes one.
const EMBEDDED_CATALOG: &[u8] = include_bytes!("../resources/games_catalog.json");

/// Shared format for `game_icons.pack`/`game_covers.pack`: `[u32 LE index
/// length][index JSON {name: [offset, len]}][blob bytes]`. Bundled as
/// resource files, not `include_bytes!`-embedded; each lookup is a
/// seek+read of one entry, not a load of the whole pack.
struct ArtPack {
    path: PathBuf,
    blob_start: u64,
    index: HashMap<String, (u32, u32)>,
}

fn open_art_pack(app: &AppHandle, resource_rel_path: &str) -> Option<ArtPack> {
    use std::io::Read;
    if !crate::integrity::resource_trusted(app, resource_rel_path) {
        log::error!("{resource_rel_path} failed its integrity check — treating it as unavailable");
        return None;
    }
    let path = app.path().resolve(resource_rel_path, tauri::path::BaseDirectory::Resource).ok()?;
    let mut f = std::fs::File::open(&path).ok()?;
    let mut hdr = [0u8; 4];
    f.read_exact(&mut hdr).ok()?;
    let ilen = u32::from_le_bytes(hdr) as usize;
    let mut ibuf = vec![0u8; ilen];
    f.read_exact(&mut ibuf).ok()?;
    let index = serde_json::from_slice(&ibuf).ok()?;
    Some(ArtPack { path, blob_start: (4 + ilen) as u64, index })
}

fn read_art_pack_entry(pack: &ArtPack, name: &str) -> Option<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let (offset, len) = *pack.index.get(name)?;
    let mut f = std::fs::File::open(&pack.path).ok()?;
    f.seek(SeekFrom::Start(pack.blob_start + offset as u64)).ok()?;
    let mut buf = vec![0u8; len as usize];
    f.read_exact(&mut buf).ok()?;
    Some(buf)
}

static ICON_PACK: std::sync::OnceLock<Option<ArtPack>> = std::sync::OnceLock::new();

fn icon_pack(app: &AppHandle) -> Option<&'static ArtPack> {
    ICON_PACK
        .get_or_init(|| {
            let pack = open_art_pack(app, "resources/game_icons.pack");
            if pack.is_none() {
                log::info!("game icon pack not available (resources/game_icons.pack)");
            }
            pack
        })
        .as_ref()
}

/// The packed WebP icon for a game display name, if the pack has it.
pub fn embedded_icon(app: &AppHandle, name: &str) -> Option<Vec<u8>> {
    read_art_pack_entry(icon_pack(app)?, name)
}

/// Best available icon for a game, as raw bytes decodable by `image::load_from_memory`. Same
/// embedded -> synced cache -> catalog cache priority as `commands::app::get_app_icon`.
pub fn best_icon_bytes(app: &AppHandle, icons: &crate::icon_cache::IconCache, name: &str) -> Option<Vec<u8>> {
    embedded_icon(app, name)
        .or_else(|| icons.get_bytes(name))
        .or_else(|| icons.get_catalog_bytes(name))
}

/// Same, as a ready-to-render data URL.
pub fn embedded_icon_data_url(app: &AppHandle, name: &str) -> Option<String> {
    use base64::Engine;
    let bytes = embedded_icon(app, name)?;
    Some(format!("data:image/webp;base64,{}", base64::engine::general_purpose::STANDARD.encode(&bytes)))
}

static COVER_PACK: std::sync::OnceLock<Option<ArtPack>> = std::sync::OnceLock::new();

fn cover_pack(app: &AppHandle) -> Option<&'static ArtPack> {
    COVER_PACK
        .get_or_init(|| {
            let pack = open_art_pack(app, "resources/game_covers.pack");
            if pack.is_none() {
                log::info!("game cover pack not available (resources/game_covers.pack)");
            }
            pack
        })
        .as_ref()
}

/// The packed square cover art for a game display name, as a data URL —
/// `None` when the pack is missing or has no entry for this game.
pub fn packed_cover_data_url(app: &AppHandle, name: &str) -> Option<String> {
    use base64::Engine;
    let bytes = read_art_pack_entry(cover_pack(app)?, name)?;
    Some(format!("data:image/webp;base64,{}", base64::engine::general_purpose::STANDARD.encode(&bytes)))
}

/// One catalog hit: display name plus (when the catalog has art for it) the
/// Discord CDN URLs of the game's icon and wide cover art.
#[derive(Clone)]
pub struct GameEntry {
    pub name: String,
    pub icon_url: Option<String>,
    pub cover_url: Option<String>,
    /// When the game was added to Discord's catalog, unix ms — decoded from
    /// the app id, which is a snowflake (`(id >> 22) + discord epoch`).
    pub created_ms: Option<i64>,
}

/// One executable entry under a stem in the lookup map. Catalog names are either a bare file
/// name (stem match) or a path-qualified one, which disambiguates generic binaries like
/// "launcher.exe" so one doesn't light up the wrong game.
#[derive(Clone)]
struct CatalogHit {
    /// Lowercased, forward-slash path suffix the process path must end
    /// with — `None` for bare names (stem match is sufficient).
    path_suffix: Option<String>,
    entry: GameEntry,
}

/// Discord snowflake → unix ms of creation (Discord epoch = 2015-01-01).
fn snowflake_ms(id: &str) -> Option<i64> {
    id.parse::<u64>().ok().map(|id| ((id >> 22) + 1_420_070_400_000) as i64)
}

/// User-managed per-game preferences, persisted to `games_prefs.json`.
#[derive(Default, Serialize, Deserialize)]
struct GamePrefs {
    /// Exe stems (lowercase) the user turned OFF — detection ignores them.
    #[serde(default)]
    disabled: HashSet<String>,
    /// Manually added games (exes the catalog doesn't know).
    #[serde(default)]
    custom: Vec<CustomGame>,
    /// exe stem (lowercase) → unix seconds of the last detection.
    #[serde(default)]
    last_played: HashMap<String, i64>,
    /// display name (lowercase) → per-game capture overrides.
    #[serde(default)]
    overrides: HashMap<String, crate::config::GameOverrides>,
}

#[derive(Clone, Serialize, Deserialize)]
struct CustomGame {
    exe: String,
    name: String,
}

/// One executable of a grouped game row, with its own enabled state so the
/// expanded row view can toggle exes individually.
#[derive(Clone, Serialize)]
pub struct ExeStatus {
    pub exe: String,
    pub enabled: bool,
}

/// One row of the settings "Games" page — one game, possibly known under
/// several executables (launcher variants, anti-cheat wrappers, …).
#[derive(Clone, Serialize)]
pub struct GameRow {
    /// Every exe stem the catalog maps to this game.
    pub exes: Vec<ExeStatus>,
    pub name: String,
    /// ON while at least one of the exes isn't disabled; toggling flips all.
    pub enabled: bool,
    pub custom: bool,
    pub last_played: Option<i64>,
    /// Catalog-addition time (unix ms, from the Discord snowflake id) — the
    /// list's newest-first sort key for games the user hasn't played.
    pub created: Option<i64>,
    pub has_icon_url: bool,
    /// Whether the catalog has wide cover art for this game — gates the
    /// settings list's lazy `fetch_game_cover` calls so the (majority of)
    /// games without one don't each fire a doomed request.
    pub has_cover_url: bool,
    /// Per-game capture overrides, when any are set.
    pub overrides: Option<crate::config::GameOverrides>,
}

/// Window handle/title of the game `GamesDb::current_game()` names, kept in
/// lockstep with it by the detection loop — lets a caller target "whatever's
/// currently playing" without re-running foreground detection itself.
#[derive(Clone, Serialize)]
pub struct CurrentGameTarget {
    pub hwnd: u32,
    pub title: String,
    pub app: String,
}

pub struct GamesDb {
    /// exe stem (lowercase, no path, no ".exe") -> every catalog entry sharing
    /// that stem (see `CatalogHit` for how several games can share one stem).
    by_exe: RwLock<HashMap<String, Vec<CatalogHit>>>,
    cache_path: PathBuf,
    prefs: RwLock<GamePrefs>,
    prefs_path: PathBuf,
    /// Display name of the game currently detected in the foreground (kept
    /// fresh by the detection loop) — powers the "Playing now" row badge.
    current: RwLock<Option<String>>,
    /// Same timing/lifecycle as `current`, but with enough detail to target
    /// the window directly. See `CurrentGameTarget`.
    current_target: RwLock<Option<CurrentGameTarget>>,
}

#[derive(Deserialize)]
struct DetectableApp {
    name: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    icon_hash: Option<String>,
    #[serde(default)]
    cover_image_hash: Option<String>,
    #[serde(default)]
    executables: Vec<Executable>,
}

#[derive(Deserialize)]
struct Executable {
    #[serde(default)]
    is_launcher: bool,
    name: String,
    #[serde(default)]
    os: Option<String>,
}

/// "grand theft auto v/gta5.exe" → "gta5"; also strips the catalog's ">" relative-path marker
/// prefix. Also used by the "add custom game" file-picker preview (`commands::games::inspect_exe_file`).
pub(crate) fn exe_stem(raw: &str) -> String {
    let last = raw.rsplit(['/', '\\']).next().unwrap_or(raw);
    last.trim_start_matches('>')
        .trim_end_matches(".exe")
        .trim_end_matches(".EXE")
        .to_ascii_lowercase()
}

impl GamesDb {
    pub fn load(config_dir: PathBuf) -> Self {
        let cache_path = config_dir.join("cache").join("games_db.json");
        let prefs_path = config_dir.join("games_prefs.json");
        let prefs: GamePrefs = std::fs::read_to_string(&prefs_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let db = Self { by_exe: RwLock::new(HashMap::new()), cache_path, prefs: RwLock::new(prefs), prefs_path, current: RwLock::new(None), current_target: RwLock::new(None) };
        let from_cache = std::fs::read(&db.cache_path).map(|bytes| db.ingest(&bytes)).unwrap_or(0);
        if from_cache > 0 {
            log::info!("games db: loaded {from_cache} known game executables from cache");
        } else {
            // No (or unparseable) disk cache — fall back to the snapshot
            // baked into the binary so the catalog works from first launch.
            let n = db.ingest(EMBEDDED_CATALOG);
            log::info!("games db: loaded {n} known game executables from the embedded snapshot");
        }
        db
    }

    fn save_prefs(&self) {
        let prefs = self.prefs.read().unwrap();
        if let Ok(json) = serde_json::to_string_pretty(&*prefs) {
            let _ = std::fs::write(&self.prefs_path, json);
        }
    }

    pub fn is_disabled(&self, exe: &str) -> bool {
        self.prefs.read().unwrap().disabled.contains(&exe.to_ascii_lowercase())
    }

    /// Flips every exe of a game row in one go (rows group a game's exes).
    pub fn set_enabled(&self, exes: &[String], enabled: bool) {
        {
            let mut prefs = self.prefs.write().unwrap();
            for exe in exes {
                let key = exe.to_ascii_lowercase();
                if enabled {
                    prefs.disabled.remove(&key);
                } else {
                    prefs.disabled.insert(key);
                }
            }
        }
        self.save_prefs();
    }

    /// Returns the effective display name actually stored (falls back to
    /// `exe` when `name` is blank) — the caller needs this to cache the
    /// icon under the same key `get_app_icon` will later look it up by.
    pub fn add_custom(&self, exe: &str, name: &str) -> String {
        let stem = exe_stem(exe);
        if stem.is_empty() {
            return String::new();
        }
        let effective_name = if name.trim().is_empty() { exe.to_string() } else { name.trim().to_string() };
        {
            let mut prefs = self.prefs.write().unwrap();
            prefs.custom.retain(|c| !c.exe.eq_ignore_ascii_case(&stem));
            prefs.custom.push(CustomGame { exe: stem, name: effective_name.clone() });
        }
        self.save_prefs();
        effective_name
    }

    /// Whether an exe stem already has a games-db entry, catalog or custom —
    /// used to decide whether a recorded window needs auto-registering as a
    /// new custom game (see `recording::finish_starting`).
    pub fn is_known_exe(&self, exe_stem: &str) -> bool {
        let key = exe_stem.to_ascii_lowercase();
        self.by_exe.read().unwrap().contains_key(&key)
            || self.prefs.read().unwrap().custom.iter().any(|c| c.exe.eq_ignore_ascii_case(exe_stem))
    }

    /// Removes one exe from a (possibly multi-exe) custom game.
    pub fn remove_custom(&self, exe: &str) {
        {
            let mut prefs = self.prefs.write().unwrap();
            prefs.custom.retain(|c| !c.exe.eq_ignore_ascii_case(exe));
        }
        self.save_prefs();
    }

    /// Removes every exe registered under a custom game's display name (case-insensitive),
    /// deleting the whole game, unlike `remove_custom` which drops just one exe.
    pub fn remove_custom_group(&self, name: &str) {
        {
            let mut prefs = self.prefs.write().unwrap();
            prefs.custom.retain(|c| !c.name.eq_ignore_ascii_case(name));
        }
        self.save_prefs();
    }

    pub fn set_current_game(&self, name: Option<String>) {
        *self.current.write().unwrap() = name;
    }

    pub fn current_game(&self) -> Option<String> {
        self.current.read().unwrap().clone()
    }

    /// Set alongside `set_current_game` (same caller, same tick) whenever a
    /// window handle is available — cleared in lockstep too.
    pub fn set_current_target(&self, target: Option<CurrentGameTarget>) {
        *self.current_target.write().unwrap() = target;
    }

    pub fn current_target(&self) -> Option<CurrentGameTarget> {
        self.current_target.read().unwrap().clone()
    }

    /// Per-game capture overrides for a game's display name, if any are set.
    pub fn overrides_for(&self, display_name: &str) -> Option<crate::config::GameOverrides> {
        self.prefs.read().unwrap().overrides.get(&display_name.to_lowercase()).cloned()
    }

    /// Stores (or clears, when everything is "default") a game's overrides.
    pub fn set_overrides(&self, display_name: &str, ov: crate::config::GameOverrides) {
        {
            let mut prefs = self.prefs.write().unwrap();
            let key = display_name.to_lowercase();
            if ov.is_empty() {
                prefs.overrides.remove(&key);
            } else {
                prefs.overrides.insert(key, ov);
            }
        }
        self.save_prefs();
    }

    /// Stamps "played now" — feeds the Games page's recently-played sort.
    pub fn touch_played(&self, exe: &str) {
        {
            let mut prefs = self.prefs.write().unwrap();
            prefs.last_played.insert(exe.to_ascii_lowercase(), chrono::Utc::now().timestamp());
        }
        self.save_prefs();
    }

    /// Catalog + custom games merged into settings-page rows — grouped by
    /// display name (one game often maps several exes) — recently played
    /// first, then alphabetical.
    pub fn list_games(&self) -> Vec<GameRow> {
        let prefs = self.prefs.read().unwrap();
        let catalog = self.by_exe.read().unwrap();

        let mut rows: Vec<GameRow> = Vec::with_capacity(prefs.custom.len() + 64);
        // Grouped by display name (case-insensitive) so one custom game with several exes
        // shows as a single row instead of one per exe.
        let mut custom_groups: HashMap<String, GameRow> = HashMap::new();
        for c in &prefs.custom {
            let key = c.exe.to_ascii_lowercase();
            let enabled = !prefs.disabled.contains(&key);
            let name_key = c.name.to_lowercase();
            let row = custom_groups.entry(name_key.clone()).or_insert_with(|| GameRow {
                exes: Vec::new(),
                name: c.name.clone(),
                enabled: false,
                custom: true,
                last_played: None,
                created: None,
                has_icon_url: false,
                has_cover_url: false,
                overrides: prefs.overrides.get(&name_key).cloned(),
            });
            row.exes.push(ExeStatus { exe: c.exe.clone(), enabled });
            row.enabled |= enabled;
            row.last_played = row.last_played.max(prefs.last_played.get(&key).copied());
        }
        for mut row in custom_groups.into_values() {
            row.exes.sort_by(|a, b| a.exe.cmp(&b.exe));
            rows.push(row);
        }

        let mut groups: HashMap<&str, GameRow> = HashMap::new();
        for (exe, hits) in catalog.iter() {
            if prefs.custom.iter().any(|c| c.exe.eq_ignore_ascii_case(exe)) {
                continue; // custom entry overrides the catalog one
            }
            let enabled = !prefs.disabled.contains(exe);
            let played = prefs.last_played.get(exe).copied();
            // One stem can belong to several games now (generic names like
            // launcher.exe, disambiguated at detection time by path) — the
            // stem gets listed under each of them.
            for hit in hits {
                let entry = &hit.entry;
                let row = groups.entry(entry.name.as_str()).or_insert_with(|| GameRow {
                    exes: Vec::new(),
                    name: entry.name.clone(),
                    enabled: false,
                    custom: false,
                    last_played: None,
                    created: None,
                    has_icon_url: entry.icon_url.is_some(),
                    has_cover_url: entry.cover_url.is_some(),
                    overrides: prefs.overrides.get(&entry.name.to_lowercase()).cloned(),
                });
                // A game often lists the same binary both bare and
                // path-qualified — identical stem, one UI row entry.
                if !row.exes.iter().any(|e| e.exe == *exe) {
                    row.exes.push(ExeStatus { exe: exe.clone(), enabled });
                }
                row.enabled |= enabled;
                row.last_played = row.last_played.max(played);
                row.created = row.created.max(entry.created_ms);
                row.has_icon_url |= entry.icon_url.is_some();
                row.has_cover_url |= entry.cover_url.is_some();
            }
        }
        for mut row in groups.into_values() {
            row.exes.sort_by(|a, b| a.exe.cmp(&b.exe));
            rows.push(row);
        }

        // Recently played on top, then catalog newcomers (snowflake date)
        // newest-first, alphabetical as the final tiebreak.
        rows.sort_by(|a, b| {
            b.last_played.unwrap_or(0).cmp(&a.last_played.unwrap_or(0))
                .then_with(|| b.created.unwrap_or(0).cmp(&a.created.unwrap_or(0)))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        rows
    }

    /// Snapshot of (display name, icon_url) pairs for every catalog game
    /// with art — what the sync icon-mirror iterates. Deduped by name,
    /// since one game's several executables share the same entry.
    pub fn all_icon_urls(&self) -> Vec<(String, String)> {
        self.by_exe
            .read()
            .unwrap()
            .values()
            .flatten()
            .filter_map(|h| h.entry.icon_url.clone().map(|u| (h.entry.name.clone(), u)))
            .collect::<HashMap<_, _>>()
            .into_iter()
            .collect()
    }

    /// Every catalog game's display name — used to tell a catalog icon apart
    /// from a custom one sharing a cache key, when migrating stray files out
    /// of the synced icon cache.
    pub fn all_catalog_names(&self) -> HashSet<String> {
        self.by_exe.read().unwrap().values().flatten().map(|h| h.entry.name.clone()).collect()
    }

    /// Parses the detectable-apps JSON into the lookup map. Returns how many
    /// executables were indexed (0 = parse failure, map left untouched).
    fn ingest(&self, bytes: &[u8]) -> usize {
        let Ok(apps) = serde_json::from_slice::<Vec<DetectableApp>>(bytes) else { return 0 };
        let mut map = HashMap::new();
        for app in apps {
            let icon_url = match (&app.id, &app.icon_hash) {
                (Some(id), Some(hash)) => Some(format!("https://cdn.discordapp.com/app-icons/{id}/{hash}.png?size=64")),
                _ => None,
            };
            // Covers live under the same app-icons CDN route, keyed by
            // cover_image hash (per Discord's CDN endpoint table).
            let cover_url = match (&app.id, &app.cover_image_hash) {
                (Some(id), Some(hash)) => Some(format!("https://cdn.discordapp.com/app-icons/{id}/{hash}.png?size=512")),
                _ => None,
            };
            for exe in &app.executables {
                if exe.is_launcher {
                    continue;
                }
                if let Some(os) = &exe.os {
                    if os != "win32" {
                        continue;
                    }
                }
                let stem = exe_stem(&exe.name);
                if stem.is_empty() {
                    continue;
                }
                // "dir/name.exe" entries keep their qualifier as a
                // path-suffix requirement; bare names match by stem alone.
                let normalized = exe.name.replace('\\', "/").to_ascii_lowercase();
                let path_suffix = normalized.contains('/').then_some(normalized);
                map.entry(stem).or_insert_with(Vec::new).push(CatalogHit {
                    path_suffix,
                    entry: GameEntry {
                        name: app.name.clone(),
                        icon_url: icon_url.clone(),
                        cover_url: cover_url.clone(),
                        created_ms: app.id.as_deref().and_then(snowflake_ms),
                    },
                });
            }
        }
        let n = map.len();
        if n > 0 {
            *self.by_exe.write().unwrap() = map;
        }
        n
    }

    /// Custom games take precedence over the catalog; disabled games are invisible to
    /// detection. A stem whose entries are all path-qualified matches nothing without a
    /// `exe_full_path` hit, so a generic name like "launcher.exe" can't light up the wrong game.
    pub fn lookup(&self, exe_stem_query: &str, exe_full_path: Option<&str>) -> Option<GameEntry> {
        let key = exe_stem_query.to_ascii_lowercase();
        if self.is_disabled(&key) {
            return None;
        }
        if let Some(c) = self.prefs.read().unwrap().custom.iter().find(|c| c.exe.eq_ignore_ascii_case(&key)) {
            return Some(GameEntry { name: c.name.clone(), icon_url: None, cover_url: None, created_ms: None });
        }
        let map = self.by_exe.read().unwrap();
        let hits = map.get(&key)?;
        if let Some(path) = exe_full_path {
            let path = path.replace('\\', "/").to_lowercase();
            for h in hits {
                let Some(suffix) = &h.path_suffix else { continue };
                // `ends_with` first guarantees the path is at least as long as
                // the suffix, so the boundary subtraction below can't underflow.
                if !path.ends_with(suffix.as_str()) {
                    continue;
                }
                // Component boundary required: "game/launcher.exe" must not
                // match "othergame/launcher.exe" just by string tail.
                let boundary_ok = path.len() == suffix.len()
                    || path.as_bytes().get(path.len() - suffix.len() - 1) == Some(&b'/');
                if boundary_ok {
                    return Some(h.entry.clone());
                }
            }
        }
        hits.iter().find(|h| h.path_suffix.is_none()).map(|h| h.entry.clone())
    }

    /// Whether `folder_segment` matches some known game's sanitized display name (catalog or
    /// custom), so a game's own recording directory isn't misread as an arbitrary `RecordingFolder`.
    /// Case-insensitive — Windows folder names are, so "grand theft auto v" is that game's folder
    /// just as much as "Grand Theft Auto V" is.
    pub fn is_known_game_folder(&self, folder_segment: &str) -> bool {
        if self.prefs.read().unwrap().custom.iter().any(|c| crate::drive::sanitize_filename(&c.name).eq_ignore_ascii_case(folder_segment)) {
            return true;
        }
        self.by_exe.read().unwrap().values().flatten().any(|h| crate::drive::sanitize_filename(&h.entry.name).eq_ignore_ascii_case(folder_segment))
    }

    pub fn is_empty(&self) -> bool {
        self.by_exe.read().unwrap().is_empty()
    }

    fn cache_age(&self) -> Option<Duration> {
        std::fs::metadata(&self.cache_path).ok()?.modified().ok()?.elapsed().ok()
    }
}

/// Downloads and ingests the catalog unconditionally. Returns the number of
/// indexed executables.
pub async fn refresh_catalog(db: &GamesDb) -> Result<usize, String> {
    let resp = match reqwest::get(URL).await {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => return Err(format!("catalog fetch failed with status {}", r.status())),
        Err(e) => return Err(format!("catalog fetch failed: {e}")),
    };
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    let n = db.ingest(&bytes);
    if n == 0 {
        return Err("catalog response did not parse".into());
    }
    if let Some(parent) = db.cache_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(&db.cache_path, &bytes) {
        log::warn!("games db: failed to write cache: {e}");
    }
    log::info!("games db: refreshed, {n} known game executables");
    Ok(n)
}

/// Refreshes the catalog in the background when the cache is missing or
/// stale. Failures are non-fatal — detection falls back to the fullscreen
/// heuristic until the next app start retries this.
pub fn spawn_refresh(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let db = app.state::<Arc<GamesDb>>();
        let cache_fresh = db.cache_age().map(|a| a < REFRESH_AFTER).unwrap_or(false);
        if cache_fresh && !db.is_empty() {
            return;
        }
        if let Err(e) = refresh_catalog(&db).await {
            log::warn!("games db: {e}");
        }
    });
}

/// Full sync (user-triggered): force-refresh the catalog, then mirror every missing game icon
/// into the icon cache (8 at a time), emitting `games-sync-progress` progress along the way.
pub async fn sync(app: &AppHandle) -> Result<(usize, usize), String> {
    use tauri::Emitter;

    let db = app.state::<Arc<GamesDb>>().inner().clone();
    let games = refresh_catalog(&db).await?;

    let icon_cache = app.state::<Arc<crate::icon_cache::IconCache>>().inner().clone();
    let missing: Vec<(String, String)> = db
        .all_icon_urls()
        .into_iter()
        // Only icons neither cached nor already in the bundled pack; goes to
        // the catalog cache, which is never synced to Drive.
        .filter(|(name, _)| !icon_cache.has_catalog(name) && embedded_icon(app, name).is_none())
        .collect();
    let total = missing.len();
    let mut done = 0usize;
    let mut downloaded = 0usize;

    let client = reqwest::Client::new();
    for chunk in missing.chunks(8) {
        let fetches = chunk.iter().map(|(name, url)| {
            let client = client.clone();
            async move {
                let resp = client.get(url).send().await.ok()?;
                if !resp.status().is_success() {
                    return None;
                }
                let bytes = resp.bytes().await.ok()?;
                Some((name.clone(), bytes))
            }
        });
        for result in futures::future::join_all(fetches).await {
            done += 1;
            if let Some((name, bytes)) = result {
                icon_cache.store_catalog_png(&name, &bytes);
                downloaded += 1;
            }
        }
        let _ = app.emit("games-sync-progress", serde_json::json!({ "done": done, "total": total }));
    }

    log::info!("games sync: {games} games, {downloaded}/{total} icons downloaded");
    Ok((games, downloaded))
}

/// The lazy per-row icon fetch, returning a ready-to-render data URL.
/// Resolution order: embedded pack -> catalog disk cache -> CDN (cached back
/// to the catalog cache). Exclusively catalog art, never the synced cache.
pub async fn fetch_icon(app: &AppHandle, exe: String) -> Result<String, String> {
    use base64::Engine;

    let db = app.state::<Arc<GamesDb>>().inner().clone();
    let icon_cache = app.state::<Arc<crate::icon_cache::IconCache>>().inner().clone();
    // Settings rows resolve by stem; when several games share it, prefer
    // one that actually has art to serve.
    let entry = db.by_exe.read().unwrap().get(&exe.to_ascii_lowercase()).and_then(|hits| {
        hits.iter().find(|h| h.entry.icon_url.is_some()).or_else(|| hits.first()).map(|h| h.entry.clone())
    });
    let entry = entry.ok_or("unknown game")?;

    if let Some(data_url) = embedded_icon_data_url(app, &entry.name) {
        return Ok(data_url);
    }
    if let Some(b64) = icon_cache.get_catalog_base64(&entry.name) {
        return Ok(format!("data:image/png;base64,{b64}"));
    }
    let url = entry.icon_url.ok_or("no icon available")?;
    let resp = reqwest::get(&url).await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("icon fetch failed: {}", resp.status()));
    }
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    icon_cache.store_catalog_png(&entry.name, &bytes);
    Ok(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&bytes)))
}

/// Like `fetch_icon`, but for the wide cover art, cached under `<name>__cover`
/// so a cover downloaded here is reusable everywhere else that key is read.
pub async fn fetch_cover(app: &AppHandle, exe: String) -> Result<String, String> {
    use base64::Engine;

    let db = app.state::<Arc<GamesDb>>().inner().clone();
    let icon_cache = app.state::<Arc<crate::icon_cache::IconCache>>().inner().clone();
    let entry = db.by_exe.read().unwrap().get(&exe.to_ascii_lowercase()).and_then(|hits| {
        hits.iter().find(|h| h.entry.cover_url.is_some()).or_else(|| hits.first()).map(|h| h.entry.clone())
    });
    let entry = entry.ok_or("unknown game")?;
    let cover_key = format!("{}__cover", entry.name);

    // Bundled cover pack before the CDN — instant and offline-safe.
    if let Some(data_url) = packed_cover_data_url(app, &entry.name) {
        return Ok(data_url);
    }
    if let Some(b64) = icon_cache.get_catalog_base64(&cover_key) {
        return Ok(format!("data:image/png;base64,{b64}"));
    }
    let url = entry.cover_url.ok_or("no cover available")?;
    let resp = reqwest::get(&url).await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("cover fetch failed: {}", resp.status()));
    }
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    icon_cache.store_catalog_png(&cover_key, &bytes);
    Ok(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db_with(catalog_json: &str) -> GamesDb {
        let dir = std::env::temp_dir().join(format!("capcove_gdb_test_{}", std::process::id()));
        let db = GamesDb::load(dir);
        assert!(db.ingest(catalog_json.as_bytes()) > 0, "test catalog failed to parse");
        db
    }

    const CATALOG: &str = r#"[
      {"name":"Grand Theft Auto V","executables":[
         {"name":"gta5.exe","os":"win32"},
         {"name":"grand theft auto v/gta5.exe","os":"win32"},
         {"name":"grand theft auto v/gta5_be.exe","os":"win32"}]},
      {"name":"Bloons TD Battles","executables":[
         {"name":"bloons td battles/battles-win.exe","os":"win32"}]},
      {"name":"Some Game","executables":[{"name":"some game/launcher.exe","os":"win32"}]},
      {"name":"Other Game","executables":[{"name":"other game/launcher.exe","os":"win32"}]}
    ]"#;

    #[test]
    fn bare_entry_matches_by_stem_alone() {
        let db = db_with(CATALOG);
        // Mixed-case query, no path at all — the bare "gta5.exe" entry must hit.
        assert_eq!(db.lookup("GTA5", None).unwrap().name, "Grand Theft Auto V");
    }

    #[test]
    fn qualified_entry_matches_full_path_case_insensitively() {
        let db = db_with(CATALOG);
        let entry = db.lookup("GTA5", Some(r"D:\Games\Grand Theft Auto V\GTA5.exe")).unwrap();
        assert_eq!(entry.name, "Grand Theft Auto V");
        let entry = db.lookup("gta5_be", Some(r"C:\GRAND THEFT AUTO V\GTA5_BE.EXE"));
        assert_eq!(entry.unwrap().name, "Grand Theft Auto V");
    }

    #[test]
    fn qualified_only_entry_needs_the_path() {
        let db = db_with(CATALOG);
        // Path matches the required suffix → hit.
        assert_eq!(
            db.lookup("battles-win", Some(r"C:\Games\Bloons TD Battles\battles-win.exe")).unwrap().name,
            "Bloons TD Battles"
        );
        // No path, or a path in the wrong folder → no match (NOT a fallback hit).
        assert!(db.lookup("battles-win", None).is_none());
        assert!(db.lookup("battles-win", Some(r"C:\Other\battles-win.exe")).is_none());
    }

    #[test]
    fn generic_launcher_stem_never_matches_the_wrong_game() {
        let db = db_with(CATALOG);
        assert_eq!(db.lookup("launcher", Some(r"C:\x\Other Game\launcher.exe")).unwrap().name, "Other Game");
        assert_eq!(db.lookup("launcher", Some(r"C:\x\Some Game\launcher.exe")).unwrap().name, "Some Game");
        assert!(db.lookup("launcher", Some(r"C:\x\Unrelated\launcher.exe")).is_none());
        assert!(db.lookup("launcher", None).is_none());
    }

    #[test]
    fn suffix_longer_than_path_does_not_panic() {
        let db = db_with(CATALOG);
        // Regression: a catalog suffix longer than the process path used to
        // underflow the boundary arithmetic and panic the detection thread.
        assert!(db.lookup("battles-win", Some("battles-win.exe")).is_none());
        assert!(db.lookup("launcher", Some("l")).is_none());
    }

    #[test]
    fn suffix_requires_component_boundary() {
        let db = db_with(CATALOG);
        // "...notsome game/launcher.exe" ends with the suffix string but not
        // at a path-component boundary — must not match.
        assert!(db.lookup("launcher", Some(r"C:\notsome game\launcher.exe")).is_none());
    }
}
