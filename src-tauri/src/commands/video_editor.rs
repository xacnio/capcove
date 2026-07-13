//! Video editor backend: probing, waveform rendering, and the timeline
//! exporter. Kept video spans are concatenated while deleted audio spans
//! become silence (keeping tracks in sync), all in a single ffmpeg pass.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::config::ConfigStore;
use crate::drive::DriveClient;
use crate::recording;

#[tauri::command]
pub async fn pick_video_file(app: AppHandle) -> Result<Option<String>, String> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        use tauri_plugin_dialog::DialogExt;
        app.dialog().file().add_filter("Video", &["mp4", "mkv", "mov"]).blocking_pick_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(picked.map(|p| p.to_string()))
}

#[derive(Debug, Clone, Serialize)]
pub struct AudioTrackProbe {
    /// Index among audio streams only (i.e. matches ffmpeg's `0:a:N`
    /// selector) — not the file's overall stream index.
    pub index: usize,
    pub label: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct VideoProbeInfo {
    pub duration_ms: u64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub audio_tracks: Vec<AudioTrackProbe>,
    pub size_bytes: u64,
    /// Unix seconds, when available (creation time isn't tracked on all
    /// filesystems — falls back to last-modified).
    pub created_at: Option<i64>,
}

/// Lists this file's audio streams, labeled from each stream's `title` tag
/// (see `recording::encoder::spawn`) or a generic fallback. The returned
/// `index` is audio-relative (`0:a:N`), as needed by `-map` arguments.
async fn probe_audio_streams(app: &AppHandle, path: &Path) -> Result<Vec<AudioTrackProbe>, String> {
    let cmd = crate::integrity::ffprobe_sidecar(app).map_err(|e| e.to_string())?;
    let output = cmd
        .args(["-v", "error", "-print_format", "json", "-show_streams", &path.to_string_lossy()])
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!("ffprobe failed: {}", String::from_utf8_lossy(&output.stderr)));
    }
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())?;
    let streams = json["streams"].as_array().cloned().unwrap_or_default();

    let mut tracks = Vec::new();
    let mut audio_idx = 0usize;
    for s in &streams {
        if s["codec_type"] == "audio" {
            let label = s["tags"]["title"].as_str().map(|t| t.to_string()).unwrap_or_else(|| format!("Audio {}", audio_idx + 1));
            tracks.push(AudioTrackProbe { index: audio_idx, label });
            audio_idx += 1;
        }
    }
    Ok(tracks)
}

#[tauri::command]
pub async fn probe_video(app: AppHandle, path: PathBuf) -> Result<VideoProbeInfo, String> {
    let cmd = crate::integrity::ffprobe_sidecar(&app).map_err(|e| e.to_string())?;
    let output = cmd
        .args([
            "-v", "error",
            "-print_format", "json",
            "-show_format", "-show_streams",
            &path.to_string_lossy(),
        ])
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!("ffprobe failed: {}", String::from_utf8_lossy(&output.stderr)));
    }
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())?;

    let duration_ms = json["format"]["duration"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .map(|s| (s * 1000.0) as u64)
        .unwrap_or(0);

    let video_stream = json["streams"]
        .as_array()
        .and_then(|streams| streams.iter().find(|s| s["codec_type"] == "video"))
        .ok_or("no video stream found")?;

    let width = video_stream["width"].as_u64().unwrap_or(0) as u32;
    let height = video_stream["height"].as_u64().unwrap_or(0) as u32;
    let fps = video_stream["r_frame_rate"]
        .as_str()
        .and_then(parse_frame_rate)
        .unwrap_or(30.0);

    let audio_tracks = probe_audio_streams(&app, &path).await.unwrap_or_default();

    let meta = std::fs::metadata(&path).ok();
    let size_bytes = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let created_at = meta.as_ref()
        .and_then(|m| m.created().or_else(|_| m.modified()).ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);

    Ok(VideoProbeInfo { duration_ms, width, height, fps, audio_tracks, size_bytes, created_at })
}

/// Renders one audio stream's waveform to a small PNG via ffmpeg's
/// `showwavespic` filter — far simpler than decoding PCM and drawing peaks
/// ourselves, and ffmpeg already has to be on hand for everything else here.
#[tauri::command]
pub async fn render_waveform(app: AppHandle, path: PathBuf, audio_index: usize, width: u32, height: u32) -> Result<String, String> {
    use tauri_plugin_shell::process::CommandEvent;

    let filter = format!("[0:a:{audio_index}]showwavespic=s={width}x{height}:colors=0x34d399:scale=sqrt");
    let cmd = crate::integrity::ffmpeg_sidecar(&app).map_err(|e| e.to_string())?.args([
        "-y", "-i", &path.to_string_lossy(),
        "-filter_complex", &filter,
        "-frames:v", "1",
        "-f", "image2pipe", "-vcodec", "png", "-",
    ]);

    // Not using `Command::output()`: it inserts a `\n` after every stdout
    // chunk, which corrupts binary PNG data. Draining the event channel and
    // concatenating chunks directly avoids that.
    let (mut rx, _child) = cmd.spawn().map_err(|e| e.to_string())?;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_ok = false;
    while let Some(event) = rx.recv().await {
        match event {
            CommandEvent::Stdout(chunk) => stdout.extend(chunk),
            CommandEvent::Stderr(chunk) => stderr.extend(chunk),
            CommandEvent::Terminated(payload) => exit_ok = payload.code == Some(0),
            CommandEvent::Error(e) => stderr.extend(e.into_bytes()),
            _ => {}
        }
    }
    if !exit_ok || stdout.is_empty() {
        return Err(format!("waveform render failed: {}", String::from_utf8_lossy(&stderr)));
    }
    use base64::Engine;
    Ok(base64::engine::general_purpose::STANDARD.encode(&stdout))
}

fn parse_frame_rate(s: &str) -> Option<f64> {
    let (num, den) = s.split_once('/')?;
    let num: f64 = num.parse().ok()?;
    let den: f64 = den.parse().ok()?;
    if den == 0.0 { None } else { Some(num / den) }
}

/// Extracts every audio track of `path` into its own small AAC file so the
/// editor's preview can play tracks individually (an HTML `<video>` only
/// plays the default track). Cached in the temp dir, keyed by path+mtime+size.
#[tauri::command]
pub async fn prepare_edit_audio(app: AppHandle, path: PathBuf) -> Result<Vec<String>, String> {
    use std::hash::{Hash, Hasher};

    let tracks = probe_audio_streams(&app, &path).await.unwrap_or_default();
    if tracks.is_empty() {
        return Ok(vec![]);
    }

    let meta = std::fs::metadata(&path).map_err(|e| e.to_string())?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (path.to_string_lossy().as_ref(), mtime, meta.len()).hash(&mut hasher);
    let key = format!("{:016x}", hasher.finish());

    let dir = std::env::temp_dir().join("dev.xacnio.capcove").join("editor_preview");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let outs: Vec<PathBuf> = tracks.iter().map(|t| dir.join(format!("{key}_a{}.m4a", t.index))).collect();

    if !outs.iter().all(|p| p.exists()) {
        let mut args: Vec<String> = vec!["-y".into(), "-i".into(), path.to_string_lossy().into_owned()];
        for (t, out) in tracks.iter().zip(&outs) {
            args.extend([
                "-map".into(), format!("0:a:{}", t.index),
                "-c:a".into(), "aac".into(),
                "-b:a".into(), "160k".into(),
                out.to_string_lossy().into_owned(),
            ]);
        }
        let cmd = crate::integrity::ffmpeg_sidecar(&app).map_err(|e| e.to_string())?;
        let output = cmd.args(args).output().await.map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(format!("preview audio extraction failed: {}", String::from_utf8_lossy(&output.stderr)));
        }
    }

    Ok(outs.iter().map(|p| p.to_string_lossy().into_owned()).collect())
}

/// A kept span of the source, in source-clip milliseconds.
#[derive(Debug, Clone, Deserialize)]
pub struct EditSegment {
    pub start_ms: u64,
    pub end_ms: u64,
}

/// A kept span of one audio track (source time) with its playback volume
/// (1.0 = unchanged). Source time NOT covered by any segment is silence.
#[derive(Debug, Clone, Deserialize)]
pub struct EditAudioSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub volume: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EditAudioTrack {
    /// Audio-relative stream index (`0:a:N`), matching `AudioTrackProbe`.
    pub index: usize,
    /// Track title written into the output's stream metadata.
    pub label: String,
    pub segments: Vec<EditAudioSegment>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EditExportJob {
    pub source_path: PathBuf,
    /// Kept video spans, in output order. Deleted spans simply aren't here.
    pub video_segments: Vec<EditSegment>,
    pub audio_tracks: Vec<EditAudioTrack>,
}

/// Piecewise volume expression for one video span `[a, b)` of one audio
/// track, in the span's local time (post-`asetpts`): the track's kept
/// segments clipped to the span map to their volume, everything else is 0.
fn volume_expr(track: &EditAudioTrack, a: f64, b: f64) -> String {
    let mut expr = String::from("0");
    // Built inside-out so the first segment ends up as the outermost `if`.
    for s in track.segments.iter().rev() {
        let s0 = (s.start_ms as f64 / 1000.0).max(a);
        let s1 = (s.end_ms as f64 / 1000.0).min(b);
        if s1 <= s0 {
            continue;
        }
        expr = format!("if(between(t,{:.3},{:.3}),{:.4},{})", s0 - a, s1 - a, s.volume, expr);
    }
    expr
}

/// Renders the whole edit in one ffmpeg pass. Deleted spans between kept ones
/// become black video of the same duration so the timeline never shifts.
/// Progress streams via `editor-export-progress` `{done_ms, total_ms}`.
#[tauri::command]
pub async fn export_edit(app: AppHandle, job: EditExportJob) -> Result<PathBuf, String> {
    use tauri_plugin_shell::process::CommandEvent;

    if job.video_segments.is_empty() {
        return Err("Timeline has no video segments".into());
    }

    let store = app.state::<Arc<ConfigStore>>();
    let settings = store.get();
    let video = settings.video.clone();
    let encoder_choice = recording::resolve_encoder(&app, &video.encoder).await;
    let codec = match encoder_choice {
        crate::config::EncoderChoice::NvencH264 => "h264_nvenc",
        crate::config::EncoderChoice::NvencHevc => "hevc_nvenc",
        crate::config::EncoderChoice::NvencAv1 => "av1_nvenc",
        crate::config::EncoderChoice::AmfH264 => "h264_amf",
        crate::config::EncoderChoice::AmfHevc => "hevc_amf",
        crate::config::EncoderChoice::AmfAv1 => "av1_amf",
        crate::config::EncoderChoice::QsvH264 => "h264_qsv",
        crate::config::EncoderChoice::QsvHevc => "hevc_qsv",
        crate::config::EncoderChoice::QsvAv1 => "av1_qsv",
        crate::config::EncoderChoice::X265Software => "libx265",
        crate::config::EncoderChoice::SvtAv1 => "libsvtav1",
        crate::config::EncoderChoice::AomAv1 => "libaom-av1",
        _ => "libx264",
    };

    // Black gap pieces must match the source exactly for concat.
    let probe = probe_video(app.clone(), job.source_path.clone()).await?;
    let (w, h) = (probe.width.max(2) & !1, probe.height.max(2) & !1);
    let fps = if probe.fps > 0.0 { probe.fps } else { 30.0 };

    let n = job.video_segments.len();
    let first_ms = job.video_segments[0].start_ms;
    let last_ms = job.video_segments[n - 1].end_ms;
    let total_ms = last_ms.saturating_sub(first_ms);
    let first_s = first_ms as f64 / 1000.0;
    let last_s = last_ms as f64 / 1000.0;

    // A stream can only feed one filterchain, so fan the video out first
    // (one split leg per kept span). Audio needs no splitting — each track
    // is a single pass-through chain over the whole output window.
    let mut fc = String::new();
    if n > 1 {
        fc.push_str(&format!("[0:v]split={n}"));
        for i in 0..n {
            fc.push_str(&format!("[vs{i}]"));
        }
        fc.push(';');
    }
    // Normalize every piece (kept and black) to the same geometry/format so
    // the concat filter accepts them.
    let norm = format!("scale={w}:{h},setsar=1,format=yuv420p");
    let mut piece_labels: Vec<String> = Vec::new();
    for (i, seg) in job.video_segments.iter().enumerate() {
        let a = seg.start_ms as f64 / 1000.0;
        let b = seg.end_ms as f64 / 1000.0;
        if i > 0 {
            let gap_s = (seg.start_ms.saturating_sub(job.video_segments[i - 1].end_ms)) as f64 / 1000.0;
            if gap_s > 0.001 {
                fc.push_str(&format!(
                    "color=c=black:s={w}x{h}:r={fps:.3},trim=duration={gap_s:.3},setpts=PTS-STARTPTS,setsar=1,format=yuv420p[g{i}];"
                ));
                piece_labels.push(format!("[g{i}]"));
            }
        }
        let vin = if n > 1 { format!("[vs{i}]") } else { "[0:v]".to_string() };
        fc.push_str(&format!("{vin}trim=start={a:.3}:end={b:.3},setpts=PTS-STARTPTS,{norm}[v{i}];"));
        piece_labels.push(format!("[v{i}]"));
    }
    for l in &piece_labels {
        fc.push_str(l);
    }
    fc.push_str(&format!("concat=n={}:v=1:a=0[vout]", piece_labels.len()));
    // Each incoming track gets its own piecewise-volume trim, then all of
    // them are mixed down into one final output track.
    for track in &job.audio_tracks {
        let k = track.index;
        fc.push_str(&format!(
            ";[0:a:{k}]atrim=start={first_s:.3}:end={last_s:.3},asetpts=PTS-STARTPTS,volume='{}':eval=frame[atr{k}]",
            volume_expr(track, first_s, last_s)
        ));
    }
    if !job.audio_tracks.is_empty() {
        let inputs: String = job.audio_tracks.iter().map(|t| format!("[atr{}]", t.index)).collect();
        fc.push_str(&format!(";{inputs}amix=inputs={}:duration=longest:normalize=0[aout]", job.audio_tracks.len()));
    }

    // Exports land straight in the library (no save dialog), named like any
    // other capture so gallery listing and Drive sync treat them natively.
    let output_path = recording::make_video_save_path(&settings.resolved_recordings_dir(), "mp4", "")
        .map_err(|e| e.to_string())?;

    let mut args: Vec<String> = vec![
        "-y".into(),
        "-i".into(), job.source_path.to_string_lossy().into_owned(),
        "-filter_complex".into(), fc,
        "-map".into(), "[vout]".into(),
    ];
    if !job.audio_tracks.is_empty() {
        args.push("-map".into());
        args.push("[aout]".into());
        args.push("-metadata:s:a:0".into());
        let title = job.audio_tracks.iter().map(|t| t.label.as_str()).collect::<Vec<_>>().join(" + ");
        args.push(format!("title={title}"));
    }
    args.extend([
        "-c:v".into(), codec.to_string(),
        "-b:v".into(), format!("{}k", video.bitrate_kbps),
        "-pix_fmt".into(), "yuv420p".into(),
    ]);
    if !job.audio_tracks.is_empty() {
        let (aname, abitrate) = video.audio_codec.ffmpeg_args();
        args.extend(["-c:a".into(), aname.into()]);
        if let Some(b) = abitrate {
            args.extend(["-b:a".into(), b.into()]);
        }
        if !matches!(video.audio_codec, crate::config::AudioCodec::Aac) {
            args.extend(["-strict".into(), "-2".into()]);
        }
    }
    args.extend(["-progress".into(), "pipe:1".into(), "-nostats".into()]);
    args.push(output_path.to_string_lossy().into_owned());

    let cmd = crate::integrity::ffmpeg_sidecar(&app).map_err(|e| e.to_string())?.args(args);
    let (mut rx, _child) = cmd.spawn().map_err(|e| e.to_string())?;
    let mut stderr = Vec::new();
    let mut exit_ok = false;
    let mut line_buf = String::new();
    while let Some(event) = rx.recv().await {
        match event {
            CommandEvent::Stdout(chunk) => {
                line_buf.push_str(&String::from_utf8_lossy(&chunk));
                while let Some(pos) = line_buf.find('\n') {
                    let line: String = line_buf.drain(..=pos).collect();
                    // `-progress` reports `out_time_us=` / `out_time_ms=`;
                    // both values are in MICROseconds (historical ffmpeg
                    // quirk — out_time_ms was never actually milliseconds).
                    let line = line.trim();
                    if let Some(v) = line.strip_prefix("out_time_us=").or_else(|| line.strip_prefix("out_time_ms=")) {
                        if let Ok(us) = v.parse::<i64>() {
                            let done_ms = (us / 1000).clamp(0, total_ms as i64);
                            let _ = app.emit("editor-export-progress", serde_json::json!({ "done_ms": done_ms, "total_ms": total_ms }));
                        }
                    }
                }
            }
            CommandEvent::Stderr(chunk) => stderr.extend(chunk),
            CommandEvent::Terminated(payload) => exit_ok = payload.code == Some(0),
            CommandEvent::Error(e) => stderr.extend(e.into_bytes()),
            _ => {}
        }
    }
    if !exit_ok {
        let _ = std::fs::remove_file(&output_path);
        return Err(format!("ffmpeg export failed: {}", String::from_utf8_lossy(&stderr)));
    }
    let _ = app.emit("editor-export-progress", serde_json::json!({ "done_ms": total_ms, "total_ms": total_ms }));

    // Stamp the export as a "clip", inheriting the source recording's
    // game/app metadata when the source lives in the library.
    let file_name = output_path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
    let meta_store = app.state::<Arc<crate::meta::MetaStore>>();
    let source_meta = job
        .source_path
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| meta_store.get(n));
    meta_store.set(
        file_name.clone(),
        crate::meta::VideoMeta {
            title: source_meta.as_ref().and_then(|m| m.title.clone()),
            app: source_meta.as_ref().and_then(|m| m.app.clone()),
            created: Some(chrono::Utc::now().timestamp()),
            kind: Some("clip".to_string()),
            stream_info: None,
            duration_secs: None,
            youtube_video_id: None,
            tags: Vec::new(),
            favorite: false,
        },
    );
    let _ = app.emit("video-saved", serde_json::json!({
        "path": output_path.to_string_lossy(),
        "name": file_name,
    }));

    Ok(output_path)
}

#[derive(Debug, Clone, Serialize)]
pub struct TrimClipResult {
    pub path: PathBuf,
    /// Relative to the recordings root — what the player needs to reopen
    /// the new clip (`ensure_playable_video`, `get_video_waveform`, ...).
    pub name: String,
    pub app: Option<String>,
}

/// Explicit overrides from the trim tool's "Advanced Export" modal — absent
/// (`None` at the call site), the quick "Save Clip" button just uses
/// whatever the app's own recording quality settings currently are.
#[derive(Debug, Clone, Deserialize)]
pub struct AdvancedTrimOptions {
    /// "native" (no scaling) or one of `RESOLUTION_ROWS`'s keys.
    pub resolution: String,
    pub encoder: crate::config::EncoderChoice,
    pub container: crate::config::VideoContainer,
    pub audio_codec: crate::config::AudioCodec,
    pub rate_control: crate::config::RateControl,
    pub bitrate_kbps: u32,
    /// QP/CRF value — only used by `Cqp`/`VbrCq` (ignored otherwise), same
    /// as the live recording settings' `quality` field.
    pub quality: u32,
}

fn resolution_target_height(resolution: &str) -> Option<u32> {
    match resolution {
        "p2160" => Some(2160),
        "p1440" => Some(1440),
        "p1080" => Some(1080),
        "p720" => Some(720),
        "p480" => Some(480),
        _ => None,
    }
}

/// The player's inline trim tool: a single fast cut, re-encoded next to the
/// source file (not the recordings root, unlike `export_edit`) — tagged as
/// a "clip" like any other saved clip. Without `advanced`, uses the app's
/// current recording quality settings as-is (same container/extension as
/// the source); with it, applies the chosen resolution/encoder/format/
/// bitrate/audio codec instead.
#[tauri::command]
pub async fn export_trim_clip(
    app: AppHandle,
    path: PathBuf,
    start_ms: u64,
    end_ms: u64,
    advanced: Option<AdvancedTrimOptions>,
) -> Result<TrimClipResult, String> {
    if end_ms <= start_ms {
        return Err("End must be after start".into());
    }

    let store = app.state::<Arc<ConfigStore>>();
    let settings = store.get();
    let video = settings.video.clone();

    let encoder_pref = advanced.as_ref().map(|a| a.encoder.clone()).unwrap_or_else(|| video.encoder.clone());
    let encoder_choice = recording::resolve_encoder(&app, &encoder_pref).await;
    let bitrate_kbps = advanced.as_ref().map(|a| a.bitrate_kbps).unwrap_or(video.bitrate_kbps);
    let rate_control = advanced.as_ref().map(|a| a.rate_control).unwrap_or(video.rate_control);
    let quality = advanced.as_ref().map(|a| a.quality).unwrap_or(video.quality);
    // Same vendor-specific CBR/VBR/CQP/VBR+CQ/Lossless flags the live
    // recording encoder uses, rather than always forcing a flat `-b:v`.
    let (codec, rate_args) = recording::encoder::encoder_args(&encoder_choice, bitrate_kbps, rate_control, quality);
    let is_av1 = matches!(
        encoder_choice,
        crate::config::EncoderChoice::NvencAv1
            | crate::config::EncoderChoice::AmfAv1
            | crate::config::EncoderChoice::QsvAv1
            | crate::config::EncoderChoice::SvtAv1
            | crate::config::EncoderChoice::AomAv1
    );
    let container = advanced.as_ref().map(|a| a.container.compatible_with_av1(is_av1));
    let audio_codec = advanced.as_ref().map(|a| a.audio_codec.clone()).unwrap_or_else(|| video.audio_codec.clone());

    let dir = path.parent().map(Path::to_path_buf).unwrap_or_else(|| settings.resolved_recordings_dir());
    let ext = container.as_ref().map(|c| c.extension()).unwrap_or_else(|| path.extension().and_then(|e| e.to_str()).unwrap_or("mp4"));
    let output_path = recording::make_video_save_path(&dir, ext, "Clip_").map_err(|e| e.to_string())?;

    let start_s = start_ms as f64 / 1000.0;
    let duration_s = (end_ms - start_ms) as f64 / 1000.0;

    let mut args: Vec<String> = vec![
        "-y".into(),
        "-ss".into(), format!("{start_s:.3}"),
        "-i".into(), path.to_string_lossy().into_owned(),
        "-t".into(), format!("{duration_s:.3}"),
        "-map".into(), "0".into(),
    ];
    // Only ever shrinks (`min(H,ih)`) — exporting at a resolution above the
    // source's own would just upscale, not add real detail.
    if let Some(height) = advanced.as_ref().and_then(|a| resolution_target_height(&a.resolution)) {
        args.extend(["-vf".into(), format!("scale=-2:min({height}\\,ih)")]);
    }
    args.extend(["-c:v".into(), codec.to_string()]);
    args.extend(rate_args);
    args.extend(["-pix_fmt".into(), "yuv420p".into()]);
    let (aname, abitrate) = audio_codec.ffmpeg_args();
    args.extend(["-c:a".into(), aname.into()]);
    if let Some(b) = abitrate {
        args.extend(["-b:a".into(), b.into()]);
    }
    if !matches!(audio_codec, crate::config::AudioCodec::Aac) {
        args.extend(["-strict".into(), "-2".into()]);
    }
    if let Some(flags) = container.as_ref().and_then(|c| c.movflags()) {
        args.extend(["-movflags".into(), flags.into()]);
    }
    args.extend(["-progress".into(), "pipe:1".into(), "-nostats".into()]);
    args.push(output_path.to_string_lossy().into_owned());

    // Streamed the same way `export_edit` reports progress: ffmpeg's own
    // `-progress` output, not a guess from elapsed wall-clock time.
    use tauri_plugin_shell::process::CommandEvent;
    let total_ms = end_ms - start_ms;
    let cmd = crate::integrity::ffmpeg_sidecar(&app).map_err(|e| e.to_string())?.args(args);
    let (mut rx, _child) = cmd.spawn().map_err(|e| e.to_string())?;
    let mut stderr = Vec::new();
    let mut exit_ok = false;
    let mut line_buf = String::new();
    while let Some(event) = rx.recv().await {
        match event {
            CommandEvent::Stdout(chunk) => {
                line_buf.push_str(&String::from_utf8_lossy(&chunk));
                while let Some(pos) = line_buf.find('\n') {
                    let line: String = line_buf.drain(..=pos).collect();
                    let line = line.trim();
                    if let Some(v) = line.strip_prefix("out_time_us=").or_else(|| line.strip_prefix("out_time_ms=")) {
                        if let Ok(us) = v.parse::<i64>() {
                            let done_ms = (us / 1000).clamp(0, total_ms as i64);
                            let _ = app.emit("trim-export-progress", serde_json::json!({ "done_ms": done_ms, "total_ms": total_ms }));
                        }
                    }
                }
            }
            CommandEvent::Stderr(chunk) => stderr.extend(chunk),
            CommandEvent::Terminated(payload) => exit_ok = payload.code == Some(0),
            CommandEvent::Error(e) => stderr.extend(e.into_bytes()),
            _ => {}
        }
    }
    if !exit_ok {
        let _ = std::fs::remove_file(&output_path);
        return Err(format!("ffmpeg trim failed: {}", String::from_utf8_lossy(&stderr)));
    }
    let _ = app.emit("trim-export-progress", serde_json::json!({ "done_ms": total_ms, "total_ms": total_ms }));

    let file_name = recording::relative_video_name(&app, &output_path);
    let meta_store = app.state::<Arc<crate::meta::MetaStore>>();
    let source_key = recording::relative_video_name(&app, &path);
    let source_meta = meta_store.get(&source_key);
    let inherited_app = source_meta.as_ref().and_then(|m| m.app.clone());
    meta_store.set(
        file_name.clone(),
        crate::meta::VideoMeta {
            title: source_meta.as_ref().and_then(|m| m.title.clone()),
            app: inherited_app.clone(),
            created: Some(chrono::Utc::now().timestamp()),
            kind: Some("clip".to_string()),
            ..Default::default()
        },
    );
    let _ = app.emit("video-saved", serde_json::json!({
        "path": output_path.to_string_lossy(),
        "name": file_name,
    }));
    crate::toast::show(&app, "info", crate::toast::ToastCategory::Clip, "Clip saved", &file_name);
    crate::sound::play(&settings.sound_effects.clip_saved);

    Ok(TrimClipResult { path: output_path, name: file_name, app: inherited_app })
}

#[tauri::command]
pub async fn upload_video_to_youtube(
    app: AppHandle,
    config: State<'_, Arc<ConfigStore>>,
    drive: State<'_, Arc<DriveClient>>,
    path: PathBuf,
    title: String,
    description: String,
    privacy: String,
) -> Result<String, String> {
    if !drive.is_connected() {
        return Err("not_connected".into());
    }
    let settings = config.get();
    let cid = settings.effective_google_client_id().to_string();
    let csec = settings.effective_google_client_secret().to_string();

    let progress_app = app.clone();
    let on_progress = move |sent: u64, total: u64, bps: u64| {
        let _ = progress_app.emit(
            "youtube-upload-progress",
            serde_json::json!({ "sent": sent, "total": total, "bps": bps }),
        );
    };

    let video_id = drive
        .upload_video_to_youtube(&cid, &csec, &path, &title, &description, &privacy, on_progress)
        .await
        .map_err(|e| e.to_string())?;

    // Full relative name, not just the bare file name — `MetaStore` keys by
    // that, so a video inside a subfolder was stamping the wrong entry.
    let file_name = recording::relative_video_name(&app, &path);
    if !file_name.is_empty() {
        let meta_store = app.state::<Arc<crate::meta::MetaStore>>();
        let mut m = meta_store.get(&file_name).unwrap_or_default();
        m.youtube_video_id = Some(video_id.clone());
        meta_store.set(file_name.clone(), m);
        let _ = app.emit("video-saved", serde_json::json!({ "name": file_name }));
    }

    Ok(format!("https://www.youtube.com/watch?v={video_id}"))
}
