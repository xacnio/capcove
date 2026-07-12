use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_updater::{Update, UpdaterExt};
use tokio::sync::Mutex;

#[derive(Default)]
pub struct PendingUpdate(pub Mutex<Option<Update>>);

#[derive(Serialize, Clone)]
pub struct UpdateInfo {
    pub version: String,
    pub current_version: String,
    pub body: Option<String>,
    pub date: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct DownloadProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
}

#[tauri::command]
pub fn get_app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

fn update_info(update: &Update) -> UpdateInfo {
    UpdateInfo {
        version: update.version.clone(),
        current_version: update.current_version.clone(),
        body: update.body.clone(),
        date: update.date.map(|d| d.to_string()),
    }
}

#[tauri::command]
pub async fn check_for_update(
    app: AppHandle,
    pending: State<'_, Arc<PendingUpdate>>,
) -> Result<Option<UpdateInfo>, String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    let update = updater.check().await.map_err(|e| e.to_string())?;
    match update {
        Some(update) => {
            let info = update_info(&update);
            *pending.0.lock().await = Some(update);
            Ok(Some(info))
        }
        None => {
            *pending.0.lock().await = None;
            Ok(None)
        }
    }
}

/// Reads back whatever the startup auto-check already found, without
/// triggering a new network check — lets the gallery surface it once a
/// window actually exists to show it in.
#[tauri::command]
pub async fn get_pending_update(pending: State<'_, Arc<PendingUpdate>>) -> Result<Option<UpdateInfo>, String> {
    Ok(pending.0.lock().await.as_ref().map(update_info))
}

#[tauri::command]
pub async fn download_and_install_update(
    app: AppHandle,
    pending: State<'_, Arc<PendingUpdate>>,
) -> Result<(), String> {
    let update = pending.0.lock().await.take().ok_or("No update available to install")?;

    let mut downloaded: u64 = 0;
    let progress_app = app.clone();
    update
        .download_and_install(
            move |chunk_len, total| {
                downloaded += chunk_len as u64;
                let _ = progress_app.emit(
                    "update-download-progress",
                    DownloadProgress { downloaded, total },
                );
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;

    app.request_restart();
    Ok(())
}

#[derive(Serialize)]
pub struct ReleaseInfo {
    pub version: String,
    pub name: Option<String>,
    pub body: Option<String>,
    pub published_at: Option<String>,
    pub url: String,
}

#[tauri::command]
pub async fn get_release_history() -> Result<Vec<ReleaseInfo>, String> {
    #[derive(serde::Deserialize)]
    struct GhRelease {
        tag_name: String,
        name: Option<String>,
        body: Option<String>,
        published_at: Option<String>,
        html_url: String,
        draft: bool,
        prerelease: bool,
    }

    let client = reqwest::Client::new();
    let releases: Vec<GhRelease> = client
        .get("https://api.github.com/repos/xacnio/capcove/releases")
        .header("User-Agent", "capcove-app")
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;

    Ok(releases
        .into_iter()
        .filter(|r| !r.draft && !r.prerelease)
        .take(20)
        .map(|r| ReleaseInfo {
            version: r.tag_name,
            name: r.name,
            body: r.body,
            published_at: r.published_at,
            url: r.html_url,
        })
        .collect())
}
