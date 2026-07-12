use super::DriveClient;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::path::Path;
use std::time::Duration;

const YOUTUBE_UPLOAD_API: &str = "https://www.googleapis.com/upload/youtube/v3/videos";

#[derive(Debug, Clone, serde::Serialize)]
pub struct YouTubeChannelInfo {
    pub id: String,
    pub title: String,
    pub thumbnail: Option<String>,
}

/// Parses a YouTube `contentDetails.duration` value ("PT1H2M3S" etc, the
/// subset of ISO 8601 durations YouTube actually emits — hours/minutes/
/// seconds only, no date components) into whole seconds.
fn parse_iso8601_duration(s: &str) -> Option<u64> {
    let s = s.strip_prefix("PT")?;
    let mut total = 0u64;
    let mut num = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            num.push(c);
            continue;
        }
        let n: u64 = num.parse().ok()?;
        num.clear();
        total += match c {
            'H' => n * 3600,
            'M' => n * 60,
            'S' => n,
            _ => return None,
        };
    }
    Some(total)
}

/// Gallery-card status of a (live or finished) broadcast.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LiveVideoInfo {
    pub title: String,
    pub live: bool,
    pub duration_secs: Option<u64>,
    pub viewers: Option<u64>,
    pub thumbnail: Option<String>,
}

/// A created (private) live broadcast, ready to be fed over RTMP.
#[derive(Debug, Clone)]
pub struct LiveBroadcast {
    pub broadcast_id: String,
    /// Full ingest URL (`rtmp://.../streamkey`) for ffmpeg's flv output.
    pub rtmp_url: String,
    /// The (reusable) liveStream this broadcast is bound to — persisted so
    /// later sessions bind to the same stream key instead of minting new ones.
    pub stream_id: String,
    /// The broadcast's YouTube title ("Game — date"), shown on the card.
    pub title: String,
    /// Zombie broadcasts (stuck "upcoming" from failed sessions) deleted to
    /// make room — the caller drops their gallery entries. Left bound,
    /// auto-start would put the new session's data live on the oldest one.
    pub cleaned_up: Vec<String>,
}

impl DriveClient {
    /// The YouTube channel this client's token is bound to (uploads land
    /// there). Needs the `youtube.readonly` scope — tokens issued before it
    /// was added to the connect flow will get a 403 until reconnect.
    pub async fn youtube_channel(&self, client_id: &str, client_secret: &str) -> Result<YouTubeChannelInfo> {
        let token = self.yt_access_token(client_id, client_secret).await?;
        let resp = self
            .http
            .get("https://www.googleapis.com/youtube/v3/channels?part=snippet&mine=true")
            .bearer_auth(&token)
            .send()
            .await?;
        if !resp.status().is_success() {
            bail!("channel lookup failed: {} {}", resp.status(), resp.text().await.unwrap_or_default());
        }
        let json: serde_json::Value = resp.json().await?;
        let item = json["items"].get(0).context("no YouTube channel on this account")?;
        Ok(YouTubeChannelInfo {
            id: item["id"].as_str().unwrap_or_default().to_string(),
            title: item["snippet"]["title"].as_str().unwrap_or_default().to_string(),
            thumbnail: item["snippet"]["thumbnails"]["default"]["url"].as_str().map(|s| s.to_string()),
        })
    }

    /// The reusable "Capcove" liveStream (stream key): verifies the stored
    /// one still exists, otherwise creates it. Returns (stream_id, rtmp_url).
    async fn get_or_create_live_stream(&self, token: &str, reuse_stream_id: Option<&str>) -> Result<(String, String)> {
        let ingest_url = |st: &serde_json::Value| -> Option<(String, String)> {
            let ingest = &st["cdn"]["ingestionInfo"];
            // Prefers the encrypted `rtmps://` address (needed for Enhanced
            // RTMP's HEVC/AV1 codec signaling), falling back to the plain
            // address if a stream predates that field.
            let address = ingest["rtmpsIngestionAddress"].as_str().or_else(|| ingest["ingestionAddress"].as_str())?;
            let name = ingest["streamName"].as_str()?;
            Some((st["id"].as_str()?.to_string(), format!("{address}/{name}")))
        };

        if let Some(id) = reuse_stream_id {
            let resp = self
                .http
                .get(format!("https://www.googleapis.com/youtube/v3/liveStreams?part=id,cdn&id={id}"))
                .bearer_auth(token)
                .send()
                .await?;
            if resp.status().is_success() {
                let json: serde_json::Value = resp.json().await?;
                if let Some(found) = json["items"].get(0).and_then(ingest_url) {
                    return Ok(found);
                }
                log::info!("youtube live: stored stream {id} no longer exists, creating a new one");
            }
        }

        let st_resp = self
            .http
            .post("https://www.googleapis.com/youtube/v3/liveStreams?part=snippet,cdn")
            .bearer_auth(token)
            .json(&serde_json::json!({
                "snippet": { "title": "Capcove" },
                "cdn": { "ingestionType": "rtmp", "resolution": "variable", "frameRate": "variable" },
            }))
            .send()
            .await?;
        if !st_resp.status().is_success() {
            bail!("liveStreams.insert failed: {} {}", st_resp.status(), st_resp.text().await.unwrap_or_default());
        }
        let st: serde_json::Value = st_resp.json().await?;
        ingest_url(&st).context("stream ingestion info missing")
    }

    /// Deletes broadcasts still stuck in "upcoming" that are bound to our
    /// stream — leftovers of sessions that never sent data. Returns the
    /// deleted ids; failures are non-fatal (worst case they linger).
    async fn cleanup_zombie_broadcasts(&self, token: &str, stream_id: &str) -> Vec<String> {
        let mut deleted = Vec::new();
        let resp = self
            .http
            .get("https://www.googleapis.com/youtube/v3/liveBroadcasts?part=id,contentDetails&broadcastStatus=upcoming&maxResults=50")
            .bearer_auth(token)
            .send()
            .await;
        let Ok(resp) = resp else { return deleted };
        let Ok(json) = resp.json::<serde_json::Value>().await else { return deleted };
        for item in json["items"].as_array().map(|a| a.as_slice()).unwrap_or_default() {
            if item["contentDetails"]["boundStreamId"].as_str() != Some(stream_id) {
                continue; // not ours — never touch broadcasts on other streams
            }
            let Some(id) = item["id"].as_str() else { continue };
            let del = self
                .http
                .delete(format!("https://www.googleapis.com/youtube/v3/liveBroadcasts?id={id}"))
                .bearer_auth(token)
                .send()
                .await;
            if matches!(del, Ok(r) if r.status().is_success()) {
                log::info!("youtube live: deleted zombie broadcast {id}");
                deleted.push(id.to_string());
            }
        }
        deleted
    }

    /// Creates a private YouTube live broadcast bound to the app's reusable
    /// stream key. `enableAutoStart`/`enableAutoStop` make it go live when
    /// data arrives and end when it stops, with no explicit transitions needed.
    pub async fn create_live_broadcast(
        &self,
        client_id: &str,
        client_secret: &str,
        title: &str,
        privacy: &str,
        reuse_stream_id: Option<&str>,
    ) -> Result<LiveBroadcast> {
        let privacy = match privacy {
            "public" | "unlisted" => privacy,
            _ => "private",
        };
        let token = self.yt_access_token(client_id, client_secret).await?;

        let (stream_id, rtmp_url) = self.get_or_create_live_stream(&token, reuse_stream_id).await?;
        let cleaned_up = self.cleanup_zombie_broadcasts(&token, &stream_id).await;

        let bc_resp = self
            .http
            .post("https://www.googleapis.com/youtube/v3/liveBroadcasts?part=snippet,status,contentDetails")
            .bearer_auth(&token)
            .json(&serde_json::json!({
                "snippet": {
                    "title": title,
                    "scheduledStartTime": chrono::Utc::now().to_rfc3339(),
                },
                "status": { "privacyStatus": privacy, "selfDeclaredMadeForKids": false },
                "contentDetails": {
                    "enableAutoStart": true,
                    "enableAutoStop": true,
                    "latencyPreference": "ultraLow",
                },
            }))
            .send()
            .await?;
        if !bc_resp.status().is_success() {
            bail!("liveBroadcasts.insert failed: {} {}", bc_resp.status(), bc_resp.text().await.unwrap_or_default());
        }
        let bc: serde_json::Value = bc_resp.json().await?;
        let broadcast_id = bc["id"].as_str().context("broadcast id missing")?.to_string();

        let bind_resp = self
            .http
            .post(format!(
                "https://www.googleapis.com/youtube/v3/liveBroadcasts/bind?id={broadcast_id}&streamId={stream_id}&part=id"
            ))
            .bearer_auth(&token)
            // Bodyless POST: Google rejects it with 411 unless
            // Content-Length is present — set it explicitly.
            .header(reqwest::header::CONTENT_LENGTH, "0")
            .body("")
            .send()
            .await?;
        if !bind_resp.status().is_success() {
            bail!("liveBroadcasts.bind failed: {} {}", bind_resp.status(), bind_resp.text().await.unwrap_or_default());
        }

        Ok(LiveBroadcast { broadcast_id, rtmp_url, stream_id, title: title.to_string(), cleaned_up })
    }

    /// Live status of a broadcast/video for the gallery card: title, live
    /// state, running/final duration, viewers, and an authorized thumbnail
    /// URL (private videos serve no public `i.ytimg.com` thumbnail).
    pub async fn live_video_info(&self, client_id: &str, client_secret: &str, video_id: &str) -> Result<LiveVideoInfo> {
        let token = self.yt_access_token(client_id, client_secret).await?;
        let resp = self
            .http
            .get(format!(
                "https://www.googleapis.com/youtube/v3/videos?part=snippet,liveStreamingDetails,contentDetails&id={video_id}"
            ))
            .bearer_auth(&token)
            .send()
            .await?;
        if !resp.status().is_success() {
            bail!("videos.list failed: {} {}", resp.status(), resp.text().await.unwrap_or_default());
        }
        let json: serde_json::Value = resp.json().await?;
        let item = json["items"].get(0).context("video not found")?;

        let details = &item["liveStreamingDetails"];
        let parse_ts = |v: &serde_json::Value| {
            v.as_str().and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok()).map(|d| d.timestamp())
        };
        let start = parse_ts(&details["actualStartTime"]);
        let end = parse_ts(&details["actualEndTime"]);
        let live = start.is_some() && end.is_none();
        let duration_secs = match (start, end) {
            (Some(s), Some(e)) => Some((e - s).max(0) as u64),
            (Some(s), None) => Some((chrono::Utc::now().timestamp() - s).max(0) as u64),
            // Not a live broadcast (or one with no liveStreamingDetails at
            // all) — fall back to the plain upload's encoded duration.
            _ => item["contentDetails"]["duration"].as_str().and_then(parse_iso8601_duration),
        };

        let thumbs = &item["snippet"]["thumbnails"];
        let thumbnail = ["medium", "high", "default"]
            .iter()
            .find_map(|k| thumbs[k]["url"].as_str())
            .map(|s| s.to_string());

        Ok(LiveVideoInfo {
            title: item["snippet"]["title"].as_str().unwrap_or_default().to_string(),
            live,
            duration_secs,
            viewers: details["concurrentViewers"].as_str().and_then(|s| s.parse().ok()),
            thumbnail,
        })
    }

    /// Ends a live broadcast now via an explicit `complete` transition instead
    /// of waiting for `enableAutoStop`. Retries since the transition is
    /// rejected until the broadcast reaches the `live` state.
    pub async fn end_live_broadcast(&self, client_id: &str, client_secret: &str, broadcast_id: &str) {
        let token = match self.yt_access_token(client_id, client_secret).await {
            Ok(t) => t,
            Err(e) => {
                log::warn!("youtube live: can't end broadcast {broadcast_id}, token unavailable: {e}");
                return;
            }
        };
        for attempt in 1u8..=4 {
            let resp = self
                .http
                .post(format!(
                    "https://www.googleapis.com/youtube/v3/liveBroadcasts/transition?broadcastStatus=complete&id={broadcast_id}&part=status"
                ))
                .bearer_auth(&token)
                // Bodyless POST needs an explicit Content-Length (see bind).
                .header(reqwest::header::CONTENT_LENGTH, "0")
                .body("")
                .send()
                .await;
            match resp {
                Ok(r) if r.status().is_success() => {
                    log::info!("youtube live: broadcast {broadcast_id} ended");
                    return;
                }
                Ok(r) => {
                    let status = r.status();
                    let body = r.text().await.unwrap_or_default();
                    // Already complete = mission accomplished.
                    if body.contains("redundantTransition") || body.contains("invalidTransition") && attempt == 4 {
                        log::info!("youtube live: broadcast {broadcast_id} already ending/ended ({status})");
                        return;
                    }
                    log::warn!("youtube live: end transition attempt {attempt} failed: {status} {}", body.chars().take(300).collect::<String>());
                }
                Err(e) => log::warn!("youtube live: end transition attempt {attempt} failed: {e}"),
            }
            tokio::time::sleep(std::time::Duration::from_secs(4)).await;
        }
        log::warn!("youtube live: gave up ending broadcast {broadcast_id}; enableAutoStop will close it when the stream drops");
    }

    /// Uploads a video file via the resumable upload protocol, returning the
    /// new video's id. Reuses this client's Drive OAuth tokens, since the
    /// connect flow requests `youtube.upload` alongside the Drive scopes.
    pub async fn upload_video_to_youtube<F>(
        &self,
        client_id: &str,
        client_secret: &str,
        path: &Path,
        title: &str,
        description: &str,
        privacy: &str,
        on_progress: F,
    ) -> Result<String>
    where
        F: Fn(u64, u64, u64) + Send + Sync + 'static,
    {
        use futures::StreamExt;
        use std::sync::{atomic::{AtomicU64, Ordering}, Arc};

        let privacy = match privacy {
            "public" | "unlisted" | "private" => privacy,
            _ => "private",
        };

        // Streamed straight off disk per attempt below, instead of reading the
        // whole exported video into a `Vec<u8>` (and duplicating it again via
        // `.chunks().map(.to_vec())`) — an exported video can be multiple GB.
        let total = tokio::fs::metadata(path).await.context("failed to stat exported video")?.len();

        let mut backoff = Duration::from_millis(1000);
        let on_prog = Arc::new(on_progress);

        for attempt in 0u8..5 {
            let token = self.yt_access_token(client_id, client_secret).await?;

            let init_resp = self
                .http
                .post(format!("{YOUTUBE_UPLOAD_API}?uploadType=resumable&part=snippet,status"))
                .bearer_auth(&token)
                .header("X-Upload-Content-Type", "video/*")
                .header("X-Upload-Content-Length", total.to_string())
                .json(&serde_json::json!({
                    "snippet": { "title": title, "description": description, "categoryId": "22" },
                    "status": { "privacyStatus": privacy },
                }))
                .send()
                .await?;

            if !init_resp.status().is_success() {
                let status = init_resp.status();
                let body = init_resp.text().await.unwrap_or_default();
                if (status.is_server_error() || status.as_u16() == 403) && attempt < 4 {
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(30));
                    continue;
                }
                bail!("failed to start YouTube upload: {status} {body}");
            }

            let upload_url = init_resp
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
                .context("YouTube did not return an upload URL")?;

            let sent_ctr = Arc::new(AtomicU64::new(0));
            let start = std::time::Instant::now();
            let sent2 = sent_ctr.clone();
            let prog2 = on_prog.clone();
            const CHUNK: usize = 262_144;
            // Reopened each attempt: a retry here starts an entirely new
            // resumable session (see the `init_resp` call above), so the body
            // needs to resend from byte 0 regardless.
            let file = tokio::fs::File::open(path).await.context("failed to open exported video")?;
            let file_stream = tokio_util::io::ReaderStream::with_capacity(file, CHUNK);
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

            let put_resp = self
                .http
                .put(&upload_url)
                .header("Content-Length", total.to_string())
                .header("Content-Type", "video/*")
                .body(body)
                .send()
                .await?;

            if put_resp.status().as_u16() == 403 || put_resp.status().is_server_error() {
                if attempt < 4 {
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(30));
                    continue;
                }
                bail!("YouTube upload failed after retries: {}", put_resp.text().await.unwrap_or_default());
            }
            if !put_resp.status().is_success() {
                bail!("YouTube upload failed: {}", put_resp.text().await.unwrap_or_default());
            }

            #[derive(Deserialize)]
            struct Uploaded { id: String }
            let uploaded: Uploaded = put_resp.json().await?;
            return Ok(uploaded.id);
        }
        bail!("YouTube upload: maximum retry count reached")
    }
}
