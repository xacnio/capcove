//! Integrity checks for ffmpeg/ffprobe and the game-art resource packs,
//! which ship as loose files next to `capcove.exe` rather than compiled
//! into it. Hashes are pinned from this build's exact bytes and checked
//! once at startup. `ffmpeg_sidecar`/`ffprobe_sidecar` are the only
//! sanctioned way to get a sidecar `Command` elsewhere in the codebase.
//!
//! `capcove.exe` itself ships unsigned, so these pins only guard against a
//! loose file being swapped independently of the exe.

#[cfg(windows)]
mod win {
    use sha2::{Digest, Sha256};
    use std::path::PathBuf;
    use std::sync::OnceLock;
    use tauri::{AppHandle, Manager};

    // Pinned to BtbN/FFmpeg-Builds' autobuild-2026-07-12-13-16 (N-125551-ga09be9b91e).
    // Update alongside release.yml/build-artifacts.yml's download step when upgrading.
    #[cfg(target_arch = "x86_64")]
    const FFMPEG_SHA256: &str = "b6e37c0e4bf1c18bd019e8926c5809ecf734249bd48c227efacf019bd6528b92";
    #[cfg(target_arch = "x86_64")]
    const FFPROBE_SHA256: &str = "fa86ff8d675b24b31979d65da1f569a9f0418e69db184ec6688f3c6d7f9ffa14";
    #[cfg(target_arch = "aarch64")]
    const FFMPEG_SHA256: &str = "dcb8e91a5dc0ed2cd29e8a4bf33fc5cf074996094818b0acc1bb2616b52381f9";
    #[cfg(target_arch = "aarch64")]
    const FFPROBE_SHA256: &str = "0fdb93d7771e8db63ddc0469cf4e151e53a0ffb9f2dfe1e50a9654da6fa5d680";

    fn sha256_hex(bytes: &[u8]) -> String {
        let digest = Sha256::digest(bytes);
        digest.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn hash_matches(path: &std::path::Path, expected: &str) -> bool {
        match std::fs::read(path) {
            Ok(bytes) => {
                let actual = sha256_hex(&bytes);
                let ok = actual.eq_ignore_ascii_case(expected);
                if !ok {
                    log::error!("integrity check failed for {}: expected {expected}, got {actual}", path.display());
                }
                ok
            }
            Err(e) => {
                log::error!("integrity check: couldn't read {}: {e}", path.display());
                false
            }
        }
    }

    /// Sidecars are resolved by `tauri-plugin-shell` as `<current_exe's dir>/<name>.exe` —
    /// see `relative_command_path` in that crate — so this mirrors that exactly.
    fn sidecar_path(filename: &str) -> Option<PathBuf> {
        let exe = std::env::current_exe().ok()?;
        Some(exe.parent()?.join(filename))
    }

    /// Verifies both sidecars once and caches the result — hashing two ~140MB
    /// binaries isn't something to redo before every one of the many places
    /// that spawn ffmpeg/ffprobe.
    fn sidecars_trusted() -> bool {
        static RESULT: OnceLock<bool> = OnceLock::new();
        *RESULT.get_or_init(|| {
            let ffmpeg_ok = sidecar_path("ffmpeg.exe").is_some_and(|p| hash_matches(&p, FFMPEG_SHA256));
            let ffprobe_ok = sidecar_path("ffprobe.exe").is_some_and(|p| hash_matches(&p, FFPROBE_SHA256));
            if !ffmpeg_ok || !ffprobe_ok {
                log::error!("ffmpeg/ffprobe failed their integrity check — recording features are disabled this session");
            }
            ffmpeg_ok && ffprobe_ok
        })
    }

    pub fn sidecar<R: tauri::Runtime>(
        shell: &tauri_plugin_shell::Shell<R>,
        name: &'static str,
    ) -> Result<tauri_plugin_shell::process::Command, String> {
        if !sidecars_trusted() {
            return Err(format!(
                "{name} failed its integrity check (the installed copy doesn't match this app's build) — refusing to run it"
            ));
        }
        shell.sidecar(name).map_err(|e| e.to_string())
    }

    // Loose resource-pack files; a mismatch degrades to "pack unavailable".
    const RESOURCE_HASHES: &[(&str, &str)] = &[
        ("resources/game_icons.pack", "b06f7115463f7e6a142c1480523cf72d2e864bc88c30232c2dd1f59bcff8a72a"),
        ("resources/game_covers.pack", "ff6e750f1602ff42560bb974c0814bba2a1fa9b204b2062570e980b3610233bc"),
    ];

    /// `true` only when `resource_rel_path` resolves and matches its pinned
    /// hash. Cached per path so repeated pack lookups don't re-hash tens of
    /// MB each time.
    pub fn resource_trusted(app: &AppHandle, resource_rel_path: &str) -> bool {
        static CACHE: OnceLock<std::sync::Mutex<std::collections::HashMap<String, bool>>> = OnceLock::new();
        let cache = CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
        if let Some(hit) = cache.lock().unwrap().get(resource_rel_path) {
            return *hit;
        }
        let Some((_, expected)) = RESOURCE_HASHES.iter().find(|(p, _)| *p == resource_rel_path) else {
            return true; // not a pinned file
        };
        let ok = app
            .path()
            .resolve(resource_rel_path, tauri::path::BaseDirectory::Resource)
            .ok()
            .is_some_and(|p| hash_matches(&p, expected));
        cache.lock().unwrap().insert(resource_rel_path.to_string(), ok);
        ok
    }
}

#[cfg(windows)]
pub use win::{resource_trusted, sidecar};

#[cfg(not(windows))]
pub fn sidecar<R: tauri::Runtime>(
    shell: &tauri_plugin_shell::Shell<R>,
    name: &'static str,
) -> Result<tauri_plugin_shell::process::Command, String> {
    shell.sidecar(name).map_err(|e| e.to_string())
}

#[cfg(not(windows))]
pub fn resource_trusted(_app: &tauri::AppHandle, _resource_rel_path: &str) -> bool {
    true
}

pub fn ffmpeg_sidecar<R: tauri::Runtime>(app: &impl tauri::Manager<R>) -> Result<tauri_plugin_shell::process::Command, String> {
    use tauri_plugin_shell::ShellExt;
    sidecar(app.shell(), "ffmpeg")
}

pub fn ffprobe_sidecar<R: tauri::Runtime>(app: &impl tauri::Manager<R>) -> Result<tauri_plugin_shell::process::Command, String> {
    use tauri_plugin_shell::ShellExt;
    sidecar(app.shell(), "ffprobe")
}
