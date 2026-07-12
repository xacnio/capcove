mod api;
mod auth;
pub mod youtube;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use zeroize::{Zeroize, ZeroizeOnDrop};

pub(super) const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub(super) const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
pub(super) const API: &str = "https://www.googleapis.com/drive/v3";
pub(super) const UPLOAD_API: &str = "https://www.googleapis.com/upload/drive/v3";

/// Shared client for every Drive/YouTube call. Only a connect timeout is set,
/// since upload/download bodies can legitimately run for minutes.
fn build_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .build()
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Token structs
// ---------------------------------------------------------------------------

/// Full in-memory token set. ZeroizeOnDrop ensures sensitive strings are wiped
/// from the heap when this value is dropped (None assignment, scope exit, etc.).
#[derive(Debug, Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
    pub account_email: Option<String>,
    #[serde(default)]
    pub account_name: Option<String>,
    /// Not encrypted separately — stored in photo_cache file (not sensitive).
    #[serde(default)]
    pub account_photo: Option<String>,
}

// Secure token store: Windows Credential Manager on Windows, Keychain/Secret
// Service elsewhere via the `keyring` crate; falls back to a plain local file
// if no such store is reachable.

/// `slot` names the credential-store entry ("google-tokens" for the main
/// account, "youtube-tokens" for the optional dedicated YouTube channel
/// token) — two independent secrets, same storage mechanics.
pub(super) fn token_store_write(slot: &str, data: &[u8], fallback_path: &std::path::Path) -> anyhow::Result<()> {
    platform::write(slot, data, fallback_path)
}

pub(super) fn token_store_read(slot: &str, fallback_path: &std::path::Path) -> anyhow::Result<Vec<u8>> {
    platform::read(slot, fallback_path)
}

pub(super) fn token_store_delete(slot: &str, fallback_path: &std::path::Path) {
    platform::delete(slot, fallback_path);
}

pub(super) const MAIN_TOKEN_SLOT: &str = "google-tokens";
pub(super) const YT_TOKEN_SLOT: &str = "youtube-tokens";

// Windows: Windows Credential Manager

#[cfg(target_os = "windows")]
mod platform {
    use anyhow::Result;
    use std::path::Path;
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Security::Credentials::{
        CredDeleteW, CredFree, CredReadW, CredWriteW,
        CREDENTIALW, CRED_FLAGS, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
    };

    // Names visible in Control Panel → Credential Manager → Windows
    // Credentials, one per token slot ("Capcove/google-tokens",
    // "Capcove/youtube-tokens").

    // Max blob for CRED_TYPE_GENERIC: 5 * 512 = 2560 bytes.
    const CRED_MAX_BLOB: usize = 2560;

    fn target_for(slot: &str) -> String {
        format!("Capcove/{slot}")
    }

    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn write(slot: &str, data: &[u8], _fallback_path: &Path) -> Result<()> {
        if data.len() > CRED_MAX_BLOB {
            anyhow::bail!(
                "token data too large for Credential Manager ({} bytes, max {})",
                data.len(),
                CRED_MAX_BLOB
            );
        }
        let target = to_wide(&target_for(slot));
        let username = to_wide(slot);
        unsafe {
            let cred = CREDENTIALW {
                Flags: CRED_FLAGS(0),
                Type: CRED_TYPE_GENERIC,
                TargetName: PWSTR(target.as_ptr() as *mut u16),
                Comment: PWSTR::null(),
                CredentialBlobSize: data.len() as u32,
                CredentialBlob: data.as_ptr() as *mut u8,
                Persist: CRED_PERSIST_LOCAL_MACHINE,
                AttributeCount: 0,
                Attributes: std::ptr::null_mut(),
                TargetAlias: PWSTR::null(),
                UserName: PWSTR(username.as_ptr() as *mut u16),
                ..Default::default()
            };
            CredWriteW(&cred, 0)
                .map_err(|e| anyhow::anyhow!("CredWriteW failed: {e}"))?;
        }
        Ok(())
    }

    pub fn read(slot: &str, _fallback_path: &Path) -> Result<Vec<u8>> {
        read_target(&target_for(slot))
    }

    fn read_target(target_name: &str) -> Result<Vec<u8>> {
        let target = to_wide(target_name);
        unsafe {
            let mut pcred: *mut CREDENTIALW = std::ptr::null_mut();
            CredReadW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, 0, &mut pcred)
                .map_err(|e| anyhow::anyhow!("CredReadW failed: {e}"))?;
            let cred = &*pcred;
            let data = std::slice::from_raw_parts(
                cred.CredentialBlob,
                cred.CredentialBlobSize as usize,
            )
            .to_vec();
            CredFree(pcred as *const core::ffi::c_void);
            Ok(data)
        }
    }

    pub fn delete(slot: &str, _fallback_path: &Path) {
        let target = to_wide(&target_for(slot));
        unsafe {
            let _ = CredDeleteW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, 0);
        }
    }
}

// Non-Windows: OS credential store via the `keyring` crate (see header comment above).

#[cfg(not(target_os = "windows"))]
mod platform {
    use anyhow::Result;
    use keyring::Entry;
    use std::path::Path;

    const SERVICE: &str = "Capcove";

    fn entry(slot: &str) -> keyring::Result<Entry> {
        Entry::new(SERVICE, slot)
    }

    pub fn write(slot: &str, data: &[u8], fallback_path: &Path) -> Result<()> {
        match entry(slot).and_then(|e| e.set_secret(data)) {
            Ok(()) => Ok(()),
            Err(e) => {
                log::warn!("no OS credential store available ({e}); storing tokens in a local file instead");
                write_fallback(data, fallback_path)
            }
        }
    }

    pub fn read(slot: &str, fallback_path: &Path) -> Result<Vec<u8>> {
        match entry(slot).and_then(|e| e.get_secret()) {
            Ok(data) => Ok(data),
            Err(keyring::Error::NoEntry) => read_fallback(fallback_path),
            Err(e) => {
                log::warn!("no OS credential store available ({e}); reading tokens from a local file instead");
                read_fallback(fallback_path)
            }
        }
    }

    pub fn delete(slot: &str, fallback_path: &Path) {
        if let Ok(e) = entry(slot) {
            let _ = e.delete_credential();
        }
        let _ = std::fs::remove_file(fallback_path);
    }

    fn write_fallback(data: &[u8], path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, data)?;
        Ok(())
    }

    fn read_fallback(path: &Path) -> Result<Vec<u8>> {
        Ok(std::fs::read(path)?)
    }
}

// ---------------------------------------------------------------------------
// Other types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub(super) struct TokenResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DriveFile {
    pub id: String,
    pub name: String,
    #[serde(rename = "createdTime")]
    pub created_time: Option<String>,
    #[serde(rename = "webViewLink")]
    pub web_view_link: Option<String>,
    pub size: Option<String>,
    #[serde(rename = "mimeType")]
    pub mime_type: Option<String>,
    #[serde(rename = "videoMediaMetadata")]
    pub video_media_metadata: Option<VideoMediaMetadata>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VideoMediaMetadata {
    pub width: Option<u32>,
    pub height: Option<u32>,
    #[serde(rename = "durationMillis")]
    pub duration_millis: Option<String>,
}

/// Makes a filename compatible with all platforms.
pub fn sanitize_filename(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        match ch {
            '\\' | '/' | ':' | '*' | '?' | '<' | '>' | '|' => out.push('_'),
            '"' => out.push('_'),
            c if (c as u32) < 32 || c as u32 == 127 => out.push('_'),
            '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{FEFF}' | '\u{00AD}' => {}
            '\u{2018}' | '\u{2019}' => out.push('\''),
            '\u{201C}' | '\u{201D}' => out.push('"'),
            c => out.push(c),
        }
    }
    let trimmed = out.trim_end_matches(|c: char| c == '.' || c == ' ');
    let trimmed = trimmed.trim_start_matches(' ');
    if trimmed.is_empty() { "file".to_string() } else { trimmed.to_string() }
}

// ---------------------------------------------------------------------------
// DriveClient
// ---------------------------------------------------------------------------

pub struct DriveClient {
    pub(super) http: reqwest::Client,
    /// Fallback token file path (used on non-Windows only; ignored on Windows).
    pub(super) tokens_file_path: PathBuf,
    /// Fallback path for the optional dedicated YouTube-channel token.
    pub(super) yt_tokens_file_path: PathBuf,
    /// Profile photo — large base64 blob, not sensitive, stored plain.
    pub(super) photo_cache_path: PathBuf,
    /// Disk cache for Drive file list (cache/drive_list.json).
    pub(super) drive_list_cache_path: PathBuf,
    pub(super) tokens: Mutex<Option<Tokens>>,
    /// Second, optional OAuth identity used only by YouTube API calls, so
    /// uploads/streams can target a separate channel while the main token
    /// stays on the user's primary account. `None` = YouTube uses the main token.
    pub(super) yt_tokens: Mutex<Option<Tokens>>,
    pub(super) folder_id: Mutex<Option<String>>,
    pub(super) folder_init_lock: tokio::sync::Mutex<()>,
    /// Serializes `ensure_subfolder`'s find-or-create sequence so concurrent
    /// uploads into the same not-yet-existing subfolder don't each create
    /// their own duplicate (Drive has no unique-name constraint per parent).
    pub(super) subfolder_init_lock: tokio::sync::Mutex<()>,
    pub(super) cached_files: Mutex<Option<(Vec<(DriveFile, String)>, std::time::Instant)>>,
    /// Caches `ensure_subfolder` results, keyed by `"{parent_id}/{name}"`,
    /// so repeated per-recording uploads into the same folder skip the
    /// find-or-create round trip.
    pub(super) subfolder_cache: Mutex<std::collections::HashMap<String, String>>,
    /// Set while an OAuth `authorize()` call is waiting for the browser
    /// redirect; lets `cancel_authorize` interrupt it early.
    pub(super) auth_cancel: Mutex<Option<std::sync::Arc<std::sync::atomic::AtomicBool>>>,
    /// `true` once the connected account's Drive is ~90%+ full — set by
    /// `refresh_capacity`. The upload worker holds off while this is set, and
    /// the UI warns about it. Cached quota + last-check time avoid re-querying
    /// the quota API on a tight loop.
    pub(super) over_capacity: std::sync::atomic::AtomicBool,
    pub(super) quota_cache: Mutex<Option<((Option<u64>, u64), std::time::Instant)>>,
}

impl DriveClient {
    pub fn new(config_dir: PathBuf) -> Self {
        let tokens_file_path      = config_dir.join("tokens.dat");
        let yt_tokens_file_path   = config_dir.join("yt_tokens.dat");
        let photo_cache_path      = config_dir.join("profile_photo.cache");
        let drive_list_cache_path = config_dir.join("cache").join("drive_list.json");

        let tokens: Option<Tokens> = token_store_read(MAIN_TOKEN_SLOT, &tokens_file_path)
            .ok()
            .and_then(|mut bytes| {
                let result = serde_json::from_slice::<Tokens>(&bytes).ok();
                bytes.zeroize();
                result
            })
            .map(|mut t| {
                t.account_photo = std::fs::read_to_string(&photo_cache_path).ok();
                t
            });

        let yt_tokens: Option<Tokens> = token_store_read(YT_TOKEN_SLOT, &yt_tokens_file_path)
            .ok()
            .and_then(|mut bytes| {
                let result = serde_json::from_slice::<Tokens>(&bytes).ok();
                bytes.zeroize();
                result
            });

        Self {
            http: build_http_client(),
            tokens_file_path,
            yt_tokens_file_path,
            photo_cache_path,
            drive_list_cache_path,
            tokens: Mutex::new(tokens),
            yt_tokens: Mutex::new(yt_tokens),
            folder_id: Mutex::new(None),
            folder_init_lock: tokio::sync::Mutex::new(()),
            subfolder_init_lock: tokio::sync::Mutex::new(()),
            cached_files: Mutex::new(None),
            subfolder_cache: Mutex::new(std::collections::HashMap::new()),
            auth_cancel: Mutex::new(None),
            over_capacity: std::sync::atomic::AtomicBool::new(false),
            quota_cache: Mutex::new(None),
        }
    }

    /// Like `new`, but never reads real saved tokens — on Windows the
    /// Credential Manager slot is machine-wide, not per-`config_dir`, so
    /// pointing at an isolated config dir alone isn't enough to avoid
    /// surfacing the user's real Google account. Used by the
    /// store-screenshot automation, which must never touch it.
    #[cfg(debug_assertions)]
    pub fn new_isolated(config_dir: PathBuf) -> Self {
        Self {
            http: build_http_client(),
            tokens_file_path: config_dir.join("tokens.dat"),
            yt_tokens_file_path: config_dir.join("yt_tokens.dat"),
            photo_cache_path: config_dir.join("profile_photo.cache"),
            drive_list_cache_path: config_dir.join("cache").join("drive_list.json"),
            tokens: Mutex::new(None),
            yt_tokens: Mutex::new(None),
            folder_id: Mutex::new(None),
            folder_init_lock: tokio::sync::Mutex::new(()),
            subfolder_init_lock: tokio::sync::Mutex::new(()),
            cached_files: Mutex::new(None),
            subfolder_cache: Mutex::new(std::collections::HashMap::new()),
            auth_cancel: Mutex::new(None),
            over_capacity: std::sync::atomic::AtomicBool::new(false),
            quota_cache: Mutex::new(None),
        }
    }

    /// Interrupts an in-flight `authorize()` call, if any, so the UI doesn't
    /// stay stuck on "waiting for browser approval" when the user closed the
    /// tab without finishing the OAuth flow.
    pub fn cancel_authorize(&self) {
        if let Some(flag) = self.auth_cancel.lock().unwrap().as_ref() {
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    pub fn is_connected(&self) -> bool {
        self.tokens.lock().unwrap().is_some()
    }

    pub fn account_email(&self) -> Option<String> {
        self.tokens.lock().unwrap().as_ref().and_then(|t| t.account_email.clone())
    }

    /// The Google account that owns the connected YouTube channel — the
    /// dedicated YouTube token's email if one is connected, else the main
    /// account's. Used to pin YouTube Studio deep links to the right account.
    pub fn youtube_account_email(&self) -> Option<String> {
        self.yt_tokens.lock().unwrap().as_ref()
            .and_then(|t| t.account_email.clone())
            .or_else(|| self.account_email())
    }

    pub fn account_name(&self) -> Option<String> {
        self.tokens.lock().unwrap().as_ref().and_then(|t| t.account_name.clone())
    }

    pub fn account_photo(&self) -> Option<String> {
        self.tokens.lock().unwrap().as_ref().and_then(|t| t.account_photo.clone())
    }

    // No server-side revocation on disconnect: Google's revoke endpoint kills
    // the whole grant, which would also invalidate the other main/YouTube
    // token sharing the same client id. Disconnect just forgets locally.

    pub fn disconnect(&self) {
        *self.tokens.lock().unwrap() = None; // ZeroizeOnDrop fires here
        *self.folder_id.lock().unwrap() = None;
        self.subfolder_cache.lock().unwrap().clear();
        token_store_delete(MAIN_TOKEN_SLOT, &self.tokens_file_path);
        let _ = std::fs::remove_file(&self.photo_cache_path);
        let _ = std::fs::remove_file(&self.drive_list_cache_path);
        // A dedicated YouTube token makes no sense without the main account.
        self.disconnect_youtube();
        self.clear_cache();
    }

    /// Whether a dedicated YouTube-channel token is connected (uploads and
    /// streams target it instead of the main account's channel).
    pub fn youtube_dedicated(&self) -> bool {
        self.yt_tokens.lock().unwrap().is_some()
    }

    /// Drops the dedicated YouTube token — YouTube features are off until a
    /// channel is connected again.
    pub fn disconnect_youtube(&self) {
        *self.yt_tokens.lock().unwrap() = None; // ZeroizeOnDrop fires here
        token_store_delete(YT_TOKEN_SLOT, &self.yt_tokens_file_path);
    }

    pub fn clear_cache(&self) {
        *self.cached_files.lock().unwrap() = None;
    }

    /// Surgically drops one file from the in-memory Drive list cache by id,
    /// keeping the rest. Called right after a successful `files.delete` so a
    /// `list_videos` before the 30s cache expires (an F5, say) doesn't
    /// re-surface the just-deleted file as a `drive_only` card. Drive's own
    /// `files.list` lags a few seconds behind a delete, but our cache doesn't
    /// have to — and unlike `clear_cache`, this won't force the next listing
    /// to re-fetch from a Drive that may still be reporting the stale file.
    pub fn forget_cached_file(&self, drive_id: &str) {
        if let Some((files, _)) = self.cached_files.lock().unwrap().as_mut() {
            files.retain(|(f, _)| f.id != drive_id);
        }
    }

    pub fn clear_folder_id(&self) {
        *self.folder_id.lock().unwrap() = None;
        self.subfolder_cache.lock().unwrap().clear();
    }
}

pub(super) fn urlencode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}
