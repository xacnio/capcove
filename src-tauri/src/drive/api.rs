use super::{DriveClient, DriveFile, API, UPLOAD_API};
use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use std::time::Duration;

/// At or above this fraction of the Drive quota, automatic uploads pause and
/// the app warns the user (freeing space or upgrading storage lets them
/// resume). Manual actions elsewhere are unaffected.
pub const DRIVE_FULL_RATIO: f64 = 0.9;

impl DriveClient {
    pub async fn ensure_folder(
        &self,
        client_id: &str,
        client_secret: &str,
        name: &str,
    ) -> Result<String> {
        if let Some(id) = self.folder_id.lock().unwrap().clone() {
            return Ok(id);
        }
        let _init_guard = self.folder_init_lock.lock().await;
        if let Some(id) = self.folder_id.lock().unwrap().clone() {
            return Ok(id);
        }
        let token = self.access_token(client_id, client_secret).await?;
        let query = format!(
            "name='{}' and mimeType='application/vnd.google-apps.folder' and trashed=false",
            name.replace('\'', "\\'")
        );
        let resp = self
            .http
            .get(format!("{API}/files"))
            .query(&[("q", query.as_str()), ("fields", "files(id,name)")])
            .bearer_auth(&token)
            .send()
            .await?;
        #[derive(Deserialize)]
        struct FileList { files: Vec<FileMeta> }
        #[derive(Deserialize)]
        struct FileMeta { id: String }
        if resp.status().is_success() {
            let list: FileList = resp.json().await?;
            if let Some(f) = list.files.into_iter().next() {
                *self.folder_id.lock().unwrap() = Some(f.id.clone());
                return Ok(f.id);
            }
        }
        let resp = self
            .http
            .post(format!("{API}/files"))
            .bearer_auth(&token)
            .json(&serde_json::json!({
                "name": name,
                "mimeType": "application/vnd.google-apps.folder",
            }))
            .send()
            .await?;
        if !resp.status().is_success() {
            bail!("failed to create Drive folder: {}", resp.text().await.unwrap_or_default());
        }
        let meta: FileMeta = resp.json().await?;
        *self.folder_id.lock().unwrap() = Some(meta.id.clone());
        Ok(meta.id)
    }

    /// Uploads into an already-resolved parent folder id — lets a caller
    /// target a nested (per-game/per-folder) Drive subfolder that mirrors
    /// the local directory structure.
    pub async fn upload_file_with_progress_to<F>(
        &self,
        client_id: &str,
        client_secret: &str,
        folder_id: &str,
        path: &std::path::Path,
        drive_file_name: &str,
        on_progress: F,
    ) -> Result<String>
    where
        F: Fn(u64, u64, u64) + Send + Sync + 'static,
    {
        use futures::StreamExt;
        use std::sync::{Arc, atomic::{AtomicU64, Ordering}};

        let token = self.access_token(client_id, client_secret).await?;
        let file_name = drive_file_name.to_string();
        // Streamed straight off disk instead of `tokio::fs::read` + a second
        // `Vec<Vec<u8>>` copy for chunking — a recording can be multiple GB,
        // and the upload worker runs several of these concurrently (see
        // `sync.rs`'s upload semaphore), so reading + duplicating whole files
        // could multiply into tens of GB of RAM under a bulk sync.
        let total = tokio::fs::metadata(path).await.context("failed to stat file")?.len();
        let mime = mime_for_ext(path);
        let modified_time = modified_rfc3339(path).await;
        let path = path.to_path_buf();

        const CHUNK: usize = 65536;
        let on_prog = Arc::new(on_progress);
        let mut backoff = Duration::from_millis(800);

        for attempt in 0u8..6 {
            // Reopened each attempt: a stream can only be read once, and a
            // retry (e.g. after a 429) needs to resend from the start.
            let file = tokio::fs::File::open(&path).await.context("failed to open file")?;
            let file_stream = tokio_util::io::ReaderStream::with_capacity(file, CHUNK);
            let sent_ctr = Arc::new(AtomicU64::new(0));
            let start = std::time::Instant::now();
            let sent2 = sent_ctr.clone();
            let prog2 = on_prog.clone();
            let progress_stream = file_stream.map(move |chunk| {
                chunk.map(|bytes| {
                    let n = bytes.len() as u64;
                    let sent = sent2.fetch_add(n, Ordering::Relaxed) + n;
                    let ms = start.elapsed().as_millis() as u64;
                    let bps = if ms > 0 { sent * 1000 / ms } else { 0 };
                    prog2(sent, total, bps);
                    http_body::Frame::data(bytes)
                })
            });
            let body = reqwest::Body::wrap(http_body_util::StreamBody::new(progress_stream));
            let metadata = {
                let mut obj = serde_json::json!({ "name": &file_name, "parents": [&folder_id] });
                if let Some(ref t) = modified_time { obj["modifiedTime"] = serde_json::Value::String(t.clone()); }
                obj
            };
            let meta_part = reqwest::multipart::Part::text(metadata.to_string()).mime_str("application/json; charset=UTF-8")?;
            let file_part = reqwest::multipart::Part::stream_with_length(body, total).mime_str(mime)?;
            let form = reqwest::multipart::Form::new().part("metadata", meta_part).part("file", file_part);
            let resp = self.http
                .post(format!("{UPLOAD_API}/files?uploadType=multipart&fields=id"))
                .bearer_auth(&token).multipart(form).send().await?;
            if resp.status().as_u16() == 429 {
                if attempt < 5 {
                    let wait = resp.headers().get("Retry-After")
                        .and_then(|v| v.to_str().ok()).and_then(|s| s.parse::<u64>().ok())
                        .map(Duration::from_secs).unwrap_or(backoff);
                    tokio::time::sleep(wait).await;
                    backoff = (backoff * 2).min(Duration::from_secs(30));
                    continue;
                }
                bail!("Drive rate limit: failed after 5 attempts");
            }
            if !resp.status().is_success() {
                bail!("upload failed: {}", resp.text().await.unwrap_or_default());
            }
            #[derive(Deserialize)]
            struct Uploaded { id: String }
            let up: Uploaded = resp.json().await?;
            return Ok(up.id);
        }
        bail!("upload: maximum retry count reached")
    }

    /// Lists every recording under the top-level Drive folder, recursing into
    /// subfolders. Returns each file paired with its `/`-joined relative path.
    /// Skips the reserved `data` subfolder (metadata.json + icon_cache).
    pub async fn list_files(
        &self,
        client_id: &str,
        client_secret: &str,
        folder_name: &str,
        on_page: impl Fn(usize, usize),
    ) -> Result<(Vec<(DriveFile, String)>, bool)> {
        {
            let cache = self.cached_files.lock().unwrap();
            if let Some((ref files, ref instant)) = *cache {
                if instant.elapsed() < Duration::from_secs(30) {
                    return Ok((files.clone(), true));
                }
            }
        }
        let root_id = self.ensure_folder(client_id, client_secret, folder_name).await?;
        let token = self.access_token(client_id, client_secret).await?;
        let mut all_files: Vec<(DriveFile, String)> = Vec::new();
        // (folder_id, relative path prefix — empty at the root)
        let mut stack: Vec<(String, String)> = vec![(root_id, String::new())];

        while let Some((folder_id, prefix)) = stack.pop() {
            let query = format!("'{}' in parents and trashed=false", folder_id);
            let mut page_token: Option<String> = None;
            // Local to this folder's own listing — recordings live in one
            // subfolder per game, so each folder's query almost always fits
            // in a single 1000-item page. A counter shared across every
            // folder visited would report e.g. "page 22" for a 22-folder,
            // 22-video library that never actually needed real pagination
            // anywhere; resetting per folder means this only climbs above 1
            // when a single folder genuinely has more than 1000 files in it.
            let mut folder_page: usize = 0;
            loop {
                let mut params: Vec<(&str, String)> = vec![
                    ("q", query.clone()),
                    ("fields", "nextPageToken,files(id,name,createdTime,webViewLink,size,mimeType,videoMediaMetadata(width,height,durationMillis))".to_string()),
                    ("pageSize", "1000".to_string()),
                ];
                if let Some(ref pt) = page_token { params.push(("pageToken", pt.clone())); }
                let resp = self.http.get(format!("{API}/files")).query(&params).bearer_auth(&token).send().await?;
                if !resp.status().is_success() {
                    bail!("failed to list Drive files: {}", resp.text().await.unwrap_or_default());
                }
                #[derive(Deserialize)]
                struct L {
                    files: Vec<DriveFile>,
                    #[serde(rename = "nextPageToken")] next_page_token: Option<String>,
                }
                let l: L = resp.json().await?;
                for f in l.files {
                    let is_folder = f.mime_type.as_deref() == Some("application/vnd.google-apps.folder");
                    if is_folder {
                        if prefix.is_empty() && f.name == "data" { continue; }
                        let next_prefix = if prefix.is_empty() { f.name.clone() } else { format!("{prefix}/{}", f.name) };
                        stack.push((f.id.clone(), next_prefix));
                        continue;
                    }
                    // Filter by extension (matches `sync::is_image`'s own check).
                    let is_video = std::path::Path::new(&f.name)
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(|e| matches!(e.to_ascii_lowercase().as_str(), "mp4" | "mkv" | "mov"))
                        .unwrap_or(false);
                    if !is_video { continue; }
                    let rel = if prefix.is_empty() { f.name.clone() } else { format!("{prefix}/{}", f.name) };
                    all_files.push((f, rel));
                }
                folder_page += 1;
                on_page(all_files.len(), folder_page);
                page_token = l.next_page_token;
                if page_token.is_none() { break; }
            }
        }
        let mut cache = self.cached_files.lock().unwrap();
        *cache = Some((all_files.clone(), std::time::Instant::now()));
        Ok((all_files, false))
    }

    pub async fn thumbnail(
        &self, client_id: &str, client_secret: &str, file_id: &str, size: u32,
    ) -> Result<Vec<u8>> {
        let token = self.access_token(client_id, client_secret).await?;
        let resp = self.http
            .get(format!("{API}/files/{file_id}?fields=thumbnailLink"))
            .bearer_auth(&token).send().await?;
        #[derive(Deserialize)]
        struct M { #[serde(rename = "thumbnailLink")] thumbnail_link: Option<String> }
        let m: M = resp.json().await?;
        let mut link = m.thumbnail_link.ok_or_else(|| anyhow!("thumbnail link not available"))?;
        if let Some(pos) = link.rfind("=s") { link.truncate(pos); }
        link.push_str(&format!("=s{size}"));
        let resp = self.http.get(&link).bearer_auth(&token).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            bail!("failed to download thumbnail ({status}): {text}");
        }
        Ok(resp.bytes().await?.to_vec())
    }

    pub async fn storage_quota(&self, client_id: &str, client_secret: &str) -> Result<(Option<u64>, u64)> {
        // 30s cache: the storage indicator, capacity gate, and settings page can
        // all ask within a short window; the quota barely moves that fast.
        {
            let cache = self.quota_cache.lock().unwrap();
            if let Some((q, at)) = *cache {
                if at.elapsed() < std::time::Duration::from_secs(30) {
                    return Ok(q);
                }
            }
        }
        #[derive(Deserialize)]
        struct Quota { limit: Option<String>, usage: Option<String> }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct AboutResp { storage_quota: Quota }
        let token = self.access_token(client_id, client_secret).await?;
        let resp = self.http.get(format!("{API}/about?fields=storageQuota")).bearer_auth(&token).send().await?;
        let about: AboutResp = resp.json().await?;
        let limit = about.storage_quota.limit.as_deref().and_then(|s| s.parse().ok());
        let usage = about.storage_quota.usage.as_deref().and_then(|s| s.parse().ok()).unwrap_or(0);
        *self.quota_cache.lock().unwrap() = Some(((limit, usage), std::time::Instant::now()));
        Ok((limit, usage))
    }

    /// Fraction of the account's Drive quota in use (0.0–1.0), or `None` when
    /// there's no finite limit (unlimited/pooled storage). Uses the cached quota.
    pub async fn used_ratio(&self, client_id: &str, client_secret: &str) -> Result<Option<f64>> {
        let (limit, usage) = self.storage_quota(client_id, client_secret).await?;
        Ok(limit.filter(|l| *l > 0).map(|l| usage as f64 / l as f64))
    }

    /// Refreshes the cached "Drive nearly full" flag (>= 90%) the upload worker
    /// gates on. Returns the used ratio (if a finite limit exists).
    pub async fn refresh_capacity(&self, client_id: &str, client_secret: &str) -> Result<Option<f64>> {
        let ratio = self.used_ratio(client_id, client_secret).await?;
        let full = ratio.map(|r| r >= DRIVE_FULL_RATIO).unwrap_or(false);
        self.over_capacity.store(full, std::sync::atomic::Ordering::SeqCst);
        Ok(ratio)
    }

    pub fn is_over_capacity(&self) -> bool {
        self.over_capacity.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub async fn delete_file(&self, client_id: &str, client_secret: &str, file_id: &str) -> Result<()> {
        let token = self.access_token(client_id, client_secret).await?;
        let resp = self.http.delete(format!("{API}/files/{file_id}")).bearer_auth(&token).send().await?;
        // 404 = the file is already gone from Drive. Deletion is idempotent, so
        // treat "not found" as success rather than surfacing a scary error.
        if resp.status().as_u16() == 404 {
            return Ok(());
        }
        if !resp.status().is_success() {
            bail!("failed to delete file: {}", resp.text().await.unwrap_or_default());
        }
        Ok(())
    }

    pub async fn download_file(&self, client_id: &str, client_secret: &str, file_id: &str) -> Result<Vec<u8>> {
        self.download_file_with_progress(client_id, client_secret, file_id, |_, _, _| {}).await
    }

    pub async fn download_file_with_progress<F>(
        &self, client_id: &str, client_secret: &str, file_id: &str, mut on_progress: F,
    ) -> Result<Vec<u8>>
    where F: FnMut(u64, u64, u64) {
        use futures::StreamExt;
        let token = self.access_token(client_id, client_secret).await?;
        let resp = self.http.get(format!("{API}/files/{file_id}?alt=media")).bearer_auth(&token).send().await?;
        if !resp.status().is_success() {
            bail!("failed to download file: {}", resp.text().await.unwrap_or_default());
        }
        let total = resp.content_length().unwrap_or(0);
        let mut received: u64 = 0;
        let mut buf: Vec<u8> = Vec::with_capacity(total as usize);
        let start = std::time::Instant::now();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            received += chunk.len() as u64;
            buf.extend_from_slice(&chunk);
            let elapsed = start.elapsed().as_secs_f64().max(0.001);
            let bps = (received as f64 / elapsed) as u64;
            on_progress(received, total, bps);
        }
        Ok(buf)
    }

    /// Like `download_file_with_progress`, but writes each chunk straight to
    /// `dest_path` instead of buffering the whole body in a `Vec<u8>` first —
    /// use this one for recordings (can be multiple GB); the `Vec<u8>` variant
    /// above is still fine for genuinely small downloads (metadata.json, icon
    /// PNGs), where an in-memory buffer costs nothing and callers want the
    /// bytes directly (e.g. to `serde_json::from_slice` them).
    pub async fn download_file_to_path_with_progress<F>(
        &self, client_id: &str, client_secret: &str, file_id: &str, dest_path: &std::path::Path, mut on_progress: F,
    ) -> Result<()>
    where F: FnMut(u64, u64, u64) {
        use futures::StreamExt;
        use tokio::io::AsyncWriteExt;
        let token = self.access_token(client_id, client_secret).await?;
        let resp = self.http.get(format!("{API}/files/{file_id}?alt=media")).bearer_auth(&token).send().await?;
        if !resp.status().is_success() {
            bail!("failed to download file: {}", resp.text().await.unwrap_or_default());
        }
        let total = resp.content_length().unwrap_or(0);
        let mut received: u64 = 0;
        let start = std::time::Instant::now();
        let mut stream = resp.bytes_stream();
        let mut file = tokio::fs::File::create(dest_path).await.context("failed to create destination file")?;
        // Unlike the `Vec<u8>` variant (which only ever touches disk once the
        // whole body is in hand), writing as chunks arrive means a mid-stream
        // failure below would otherwise leave a truncated file at `dest_path`
        // — clean that up on any error path instead of silently leaving a
        // broken "recording" behind.
        let result: Result<()> = async {
            while let Some(chunk) = stream.next().await {
                let chunk = chunk?;
                received += chunk.len() as u64;
                file.write_all(&chunk).await.context("failed to write downloaded chunk")?;
                let elapsed = start.elapsed().as_secs_f64().max(0.001);
                let bps = (received as f64 / elapsed) as u64;
                on_progress(received, total, bps);
            }
            file.flush().await.context("failed to flush downloaded file")?;
            Ok(())
        }.await;
        if result.is_err() {
            drop(file);
            let _ = tokio::fs::remove_file(dest_path).await;
        }
        result
    }

    pub async fn file_exists(&self, client_id: &str, client_secret: &str, file_id: &str) -> Result<bool> {
        let token = self.access_token(client_id, client_secret).await?;
        let resp = self.http.get(format!("{API}/files/{file_id}")).query(&[("fields", "id,trashed")]).bearer_auth(&token).send().await?;
        if resp.status().is_success() {
            #[derive(Deserialize)]
            struct F { #[serde(default)] trashed: bool }
            let f: F = resp.json().await?;
            Ok(!f.trashed)
        } else if resp.status() == reqwest::StatusCode::NOT_FOUND {
            Ok(false)
        } else {
            bail!("file check failed: {}", resp.text().await.unwrap_or_default());
        }
    }

    pub async fn find_item_in_folder(
        &self, client_id: &str, client_secret: &str, parent_id: &str, name: &str,
    ) -> Result<Option<String>> {
        let token = self.access_token(client_id, client_secret).await?;
        let query = format!("'{}' in parents and name='{}' and trashed=false", parent_id, name.replace('\'', "\\'"));
        let resp = self.http.get(format!("{API}/files"))
            .query(&[("q", query.as_str()), ("fields", "files(id)")])
            .bearer_auth(&token).send().await?;
        #[derive(Deserialize)]
        struct FileList { files: Vec<FileMeta> }
        #[derive(Deserialize)]
        struct FileMeta { id: String }
        if resp.status().is_success() {
            let list: FileList = resp.json().await?;
            if let Some(f) = list.files.into_iter().next() {
                return Ok(Some(f.id));
            }
        }
        Ok(None)
    }

    pub async fn ensure_subfolder(
        &self, client_id: &str, client_secret: &str, parent_id: &str, name: &str,
    ) -> Result<String> {
        let cache_key = format!("{parent_id}/{name}");
        if let Some(id) = self.subfolder_cache.lock().unwrap().get(&cache_key).cloned() {
            return Ok(id);
        }
        // Serialize the find-or-create sequence: without a lock, concurrent
        // uploads into the same not-yet-existing subfolder would each create
        // their own duplicate (Drive doesn't enforce unique names per parent).
        let _guard = self.subfolder_init_lock.lock().await;
        // Re-check: another task may have resolved/created it while we waited.
        if let Some(id) = self.subfolder_cache.lock().unwrap().get(&cache_key).cloned() {
            return Ok(id);
        }
        if let Some(id) = self.find_item_in_folder(client_id, client_secret, parent_id, name).await? {
            self.subfolder_cache.lock().unwrap().insert(cache_key, id.clone());
            return Ok(id);
        }
        let token = self.access_token(client_id, client_secret).await?;
        let resp = self.http.post(format!("{API}/files"))
            .bearer_auth(&token)
            .json(&serde_json::json!({
                "name": name,
                "mimeType": "application/vnd.google-apps.folder",
                "parents": [parent_id],
            }))
            .send().await?;
        if !resp.status().is_success() {
            bail!("failed to create subfolder: {}", resp.text().await.unwrap_or_default());
        }
        #[derive(Deserialize)]
        struct FileMeta { id: String }
        let meta: FileMeta = resp.json().await?;
        self.subfolder_cache.lock().unwrap().insert(cache_key, meta.id.clone());
        Ok(meta.id)
    }

    /// Resolves the nested Drive folder id for a `/`-joined local relative
    /// path's directory part, creating each level under `root_id` as needed.
    /// Empty `segments` = the root itself.
    pub async fn ensure_nested_folder(
        &self, client_id: &str, client_secret: &str, root_id: &str, segments: &[String],
    ) -> Result<String> {
        let mut parent = root_id.to_string();
        for seg in segments {
            parent = self.ensure_subfolder(client_id, client_secret, &parent, seg).await?;
        }
        Ok(parent)
    }

    /// Renames the Drive folder mirroring a local recording-folder rename.
    /// `parent_segments` addresses the parent of the renamed folder;
    /// `old_name`/`new_name` are already-sanitized. No-op if never created on Drive.
    pub async fn rename_nested_folder(
        &self,
        client_id: &str,
        client_secret: &str,
        root_id: &str,
        parent_segments: &[String],
        old_name: &str,
        new_name: &str,
    ) -> Result<()> {
        if old_name == new_name {
            return Ok(());
        }
        let parent_id = self
            .ensure_nested_folder(client_id, client_secret, root_id, parent_segments)
            .await?;
        let token = self.access_token(client_id, client_secret).await?;
        let query = format!(
            "'{}' in parents and name='{}' and mimeType='application/vnd.google-apps.folder' and trashed=false",
            parent_id,
            old_name.replace('\'', "\\'")
        );
        let resp = self
            .http
            .get(format!("{API}/files"))
            .query(&[("q", query.as_str()), ("fields", "files(id)")])
            .bearer_auth(&token)
            .send()
            .await?;
        #[derive(Deserialize)]
        struct FileList { files: Vec<FileMeta> }
        #[derive(Deserialize)]
        struct FileMeta { id: String }
        if !resp.status().is_success() {
            bail!("failed to look up Drive folder to rename: {}", resp.text().await.unwrap_or_default());
        }
        let list: FileList = resp.json().await?;
        let Some(folder_id) = list.files.into_iter().next().map(|f| f.id) else {
            return Ok(());
        };

        let resp = self
            .http
            .patch(format!("{API}/files/{folder_id}"))
            .bearer_auth(&token)
            .json(&serde_json::json!({ "name": new_name }))
            .send()
            .await?;
        if !resp.status().is_success() {
            bail!("failed to rename Drive folder: {}", resp.text().await.unwrap_or_default());
        }

        let mut cache = self.subfolder_cache.lock().unwrap();
        cache.remove(&format!("{parent_id}/{old_name}"));
        cache.insert(format!("{parent_id}/{new_name}"), folder_id);
        Ok(())
    }

    pub async fn list_files_in_folder(
        &self, client_id: &str, client_secret: &str, folder_id: &str,
    ) -> Result<Vec<DriveFile>> {
        let token = self.access_token(client_id, client_secret).await?;
        let query = format!("'{}' in parents and trashed=false", folder_id);
        let resp = self.http.get(format!("{API}/files"))
            .query(&[("q", query.as_str()), ("fields", "files(id,name,createdTime,webViewLink,size,mimeType)"), ("pageSize", "1000")])
            .bearer_auth(&token).send().await?;
        if !resp.status().is_success() {
            bail!("failed to list folder files: {}", resp.text().await.unwrap_or_default());
        }
        #[derive(Deserialize)]
        struct L { files: Vec<DriveFile> }
        let l: L = resp.json().await?;
        Ok(l.files)
    }

    pub async fn upload_bytes(
        &self, client_id: &str, client_secret: &str, parent_id: &str,
        file_name: &str, mime_type: &str, bytes: Vec<u8>,
    ) -> Result<String> {
        let token = self.access_token(client_id, client_secret).await?;
        let metadata = serde_json::json!({ "name": file_name, "parents": [parent_id] });
        #[derive(Deserialize)]
        struct Uploaded { id: String }
        let mut backoff = std::time::Duration::from_millis(800);
        for attempt in 0u8..6 {
            let meta_part = reqwest::multipart::Part::text(metadata.to_string()).mime_str("application/json; charset=UTF-8")?;
            let file_part = reqwest::multipart::Part::bytes(bytes.clone()).mime_str(mime_type)?;
            let form = reqwest::multipart::Form::new().part("metadata", meta_part).part("file", file_part);
            let resp = self.http
                .post(format!("{UPLOAD_API}/files?uploadType=multipart&fields=id"))
                .bearer_auth(&token).multipart(form).send().await?;
            if resp.status().as_u16() == 429 {
                if attempt < 5 {
                    let wait = resp.headers().get("Retry-After")
                        .and_then(|v| v.to_str().ok()).and_then(|s| s.parse::<u64>().ok())
                        .map(std::time::Duration::from_secs).unwrap_or(backoff);
                    tokio::time::sleep(wait).await;
                    backoff = (backoff * 2).min(std::time::Duration::from_secs(30));
                    continue;
                }
                bail!("Drive rate limit: failed after 5 attempts");
            }
            if !resp.status().is_success() {
                bail!("upload failed: {}", resp.text().await.unwrap_or_default());
            }
            let up: Uploaded = resp.json().await?;
            return Ok(up.id);
        }
        bail!("upload: maximum retry count reached")
    }

    pub async fn list_root_folders(&self, client_id: &str, client_secret: &str) -> Result<Vec<(String, String)>> {
        let token = self.access_token(client_id, client_secret).await?;
        let query = "mimeType='application/vnd.google-apps.folder' and 'root' in parents and trashed=false";
        let params = vec![
            ("q", query.to_string()),
            ("fields", "files(id,name)".to_string()),
            ("pageSize", "100".to_string()),
            ("orderBy", "name".to_string()),
        ];
        let resp = self.http.get(format!("{API}/files")).query(&params).bearer_auth(&token).send().await?;
        #[derive(Deserialize)]
        struct Folder { id: String, name: String }
        #[derive(Deserialize)]
        struct Resp { files: Vec<Folder> }
        let body: Resp = resp.json().await?;
        Ok(body.files.into_iter().map(|f| (f.id, f.name)).collect())
    }

    /// Deletes every folder under the recordings root that ends up empty —
    /// no files and no non-empty subfolders (a folder containing only empty
    /// subfolders is itself empty and goes too). These accumulate when all of
    /// a game/folder's recordings are deleted from Drive but its mirrored
    /// Drive folder is left behind (uploads create folders, deletes never
    /// remove them). The root and the reserved `data` folder are never
    /// touched. Returns the number of delete calls made.
    pub async fn delete_empty_folders(
        &self, client_id: &str, client_secret: &str, folder_name: &str,
    ) -> Result<usize> {
        struct Node { id: String, parent: Option<usize>, children: Vec<usize>, has_files: bool }

        let root_id = self.ensure_folder(client_id, client_secret, folder_name).await?;
        // Discover the whole folder tree (BFS via an explicit stack), noting
        // for each folder whether it directly holds any files.
        let mut nodes: Vec<Node> = vec![Node { id: root_id, parent: None, children: vec![], has_files: false }];
        let mut stack = vec![0usize];
        while let Some(idx) = stack.pop() {
            let folder_id = nodes[idx].id.clone();
            let children = self.list_files_in_folder(client_id, client_secret, &folder_id).await?;
            for c in children {
                let is_folder = c.mime_type.as_deref() == Some("application/vnd.google-apps.folder");
                if !is_folder {
                    nodes[idx].has_files = true;
                    continue;
                }
                // Reserved metadata/icon store — never a candidate.
                if idx == 0 && c.name == "data" { continue; }
                let child_idx = nodes.len();
                nodes.push(Node { id: c.id, parent: Some(idx), children: vec![], has_files: false });
                nodes[idx].children.push(child_idx);
                stack.push(child_idx);
            }
        }

        // Bottom-up emptiness: memoized so each node is computed once.
        fn is_empty(nodes: &[Node], idx: usize, memo: &mut [Option<bool>]) -> bool {
            if let Some(v) = memo[idx] { return v; }
            let children = nodes[idx].children.clone();
            let empty = !nodes[idx].has_files && children.iter().all(|&c| is_empty(nodes, c, memo));
            memo[idx] = Some(empty);
            empty
        }
        let mut memo = vec![None; nodes.len()];
        for idx in 0..nodes.len() {
            is_empty(&nodes, idx, &mut memo);
        }

        // Every node beneath `idx` (for purging the id cache after a delete).
        fn descendant_indices(nodes: &[Node], idx: usize) -> Vec<usize> {
            let mut out = Vec::new();
            let mut stack = nodes[idx].children.clone();
            while let Some(i) = stack.pop() {
                out.push(i);
                stack.extend(nodes[i].children.iter().copied());
            }
            out
        }

        // Delete only the topmost empty folder in each chain — removing a
        // parent trashes its (empty) descendants too, so deleting them
        // separately is wasted API calls. Never the root (idx 0).
        let mut deleted = 0;
        for idx in 1..nodes.len() {
            if memo[idx] != Some(true) { continue; }
            let parent_is_deletable = match nodes[idx].parent {
                Some(0) | None => false, // parent is root (never deleted) → delete this directly
                Some(p) => memo[p] == Some(true),
            };
            if parent_is_deletable { continue; }
            if let Err(e) = self.delete_file(client_id, client_secret, &nodes[idx].id).await {
                log::warn!("empty-folder cleanup: could not delete folder {}: {e}", nodes[idx].id);
            } else {
                deleted += 1;
                // Deleting a parent trashes its cached-but-now-gone empty
                // descendants too; drop every `subfolder_cache` entry whose id
                // was removed, so a later re-upload recreates the folder
                // instead of getting handed a stale, trashed id.
                let gone: std::collections::HashSet<&str> = std::iter::once(idx)
                    .chain(descendant_indices(&nodes, idx))
                    .map(|i| nodes[i].id.as_str())
                    .collect();
                self.subfolder_cache.lock().unwrap().retain(|_, v| !gone.contains(v.as_str()));
            }
        }
        if deleted > 0 {
            self.clear_cache();
        }
        Ok(deleted)
    }

    pub async fn list_folder_file_names(
        &self, client_id: &str, client_secret: &str, folder_id: &str, limit: u32,
    ) -> Result<Vec<String>> {
        let token = self.access_token(client_id, client_secret).await?;
        let query = format!("'{folder_id}' in parents and trashed=false and mimeType!='application/vnd.google-apps.folder'");
        let params = vec![
            ("q", query),
            ("fields", "files(name)".to_string()),
            ("pageSize", limit.to_string()),
        ];
        let resp = self.http.get(format!("{API}/files")).query(&params).bearer_auth(&token).send().await?;
        #[derive(Deserialize)]
        struct F { name: String }
        #[derive(Deserialize)]
        struct Resp { files: Vec<F> }
        let body: Resp = resp.json().await?;
        Ok(body.files.into_iter().map(|f| f.name).collect())
    }

    pub async fn update_bytes(
        &self, client_id: &str, client_secret: &str, file_id: &str, bytes: Vec<u8>,
    ) -> Result<()> {
        let token = self.access_token(client_id, client_secret).await?;
        let resp = self.http
            .patch(format!("{UPLOAD_API}/files/{file_id}?uploadType=media"))
            .bearer_auth(&token).body(bytes).send().await?;
        if !resp.status().is_success() {
            bail!("update failed: {}", resp.text().await.unwrap_or_default());
        }
        Ok(())
    }
}

fn mime_for_ext(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("png")  => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("avif") => "image/avif",
        Some("bmp")  => "image/bmp",
        _ => "application/octet-stream",
    }
}

async fn modified_rfc3339(path: &std::path::Path) -> Option<String> {
    tokio::fs::metadata(path).await.ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| {
            chrono::DateTime::<chrono::Utc>::from_timestamp(d.as_secs() as i64, 0)
                .unwrap_or_default()
                .to_rfc3339()
        })
}
