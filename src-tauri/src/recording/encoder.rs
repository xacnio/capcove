//! FFmpeg sidecar process management: builds the encode command for the
//! chosen codec/vendor, spawns it, and pipes raw BGRA frames to its stdin.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_shell::process::{CommandChild, CommandEvent};

use crate::config::EncoderChoice;

/// Caches `resolve_auto`'s result for the app's lifetime — hardware encoder
/// support doesn't change between recordings, so there's no need to re-probe.
#[derive(Default)]
pub struct AutoEncoderCache(Mutex<Option<EncoderChoice>>);

#[derive(Debug, Clone, Serialize)]
pub struct EncoderInfo {
    pub kind: EncoderChoice,
    pub label: String,
    pub available: bool,
}

/// All concrete (non-`Auto`) encoder candidates, in the order they're shown
/// to the user. Probed via a real 1-frame dry-run so "available" reflects
/// actual hardware support, not just whether FFmpeg was built with the codec.
pub async fn list_available_encoders(app: &AppHandle) -> Vec<EncoderInfo> {
    let candidates = [
        (EncoderChoice::NvencH264, "NVIDIA NVENC (H.264)"),
        (EncoderChoice::NvencHevc, "NVIDIA NVENC (HEVC)"),
        (EncoderChoice::NvencAv1, "NVIDIA NVENC (AV1)"),
        (EncoderChoice::AmfH264, "AMD AMF (H.264)"),
        (EncoderChoice::AmfHevc, "AMD AMF (HEVC)"),
        (EncoderChoice::AmfAv1, "AMD AMF (AV1)"),
        (EncoderChoice::QsvH264, "Intel QSV (H.264)"),
        (EncoderChoice::QsvHevc, "Intel QSV (HEVC)"),
        (EncoderChoice::QsvAv1, "Intel QSV (AV1)"),
        (EncoderChoice::X264Software, "Software x264 (H.264)"),
        (EncoderChoice::X265Software, "Software x265 (HEVC)"),
        (EncoderChoice::SvtAv1, "Software SVT-AV1"),
        (EncoderChoice::AomAv1, "Software AOM AV1"),
    ];
    let mut out = Vec::with_capacity(candidates.len());
    for (kind, label) in candidates {
        // Software encoders always work; skip the (pointless) dry-run for them.
        let available = matches!(
            kind,
            EncoderChoice::X264Software | EncoderChoice::X265Software | EncoderChoice::SvtAv1 | EncoderChoice::AomAv1
        ) || probe_encoder(app, Path::new("ffmpeg"), &kind).await;
        out.push(EncoderInfo { kind, label: label.into(), available });
    }
    out
}

/// Maps a user/auto encoder choice to the concrete ffmpeg `-c:v` name and its
/// rate-control args. `Auto` is already resolved by the caller.
/// Rotating-segment container for the disk-mode replay buffer. Both keep the
/// "currently-open segment is always valid" property the buffer relies on (no
/// moov index to finalize per rotation); the codec decides which one is legal.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SegmentContainer {
    /// MPEG-TS — the robust live-capture default for H.264/HEVC.
    MpegTs,
    /// Fragmented MP4 — for codecs MPEG-TS can't carry as a recognizable
    /// stream (AV1, which TS muxes as opaque private data / stream_type 0x06).
    FragMp4,
}

impl SegmentContainer {
    pub fn ext(self) -> &'static str {
        match self {
            SegmentContainer::MpegTs => "ts",
            SegmentContainer::FragMp4 => "mp4",
        }
    }
}

/// Picks the segment container that suits the resolved encoder's codec: AV1
/// needs fragmented MP4 (MPEG-TS can't represent it), everything else uses the
/// battle-tested MPEG-TS path.
pub fn segment_container_for(encoder: &EncoderChoice) -> SegmentContainer {
    match encoder {
        EncoderChoice::NvencAv1
        | EncoderChoice::AmfAv1
        | EncoderChoice::QsvAv1
        | EncoderChoice::SvtAv1
        | EncoderChoice::AomAv1 => SegmentContainer::FragMp4,
        _ => SegmentContainer::MpegTs,
    }
}

pub(crate) fn encoder_args(choice: &EncoderChoice, bitrate_kbps: u32, rate_control: crate::config::RateControl, quality: u32) -> (&'static str, Vec<String>) {
    let (codec, mut args): (&'static str, Vec<String>) = match choice {
        EncoderChoice::NvencH264 => ("h264_nvenc", vec!["-preset".into(), "p4".into()]),
        EncoderChoice::NvencHevc => ("hevc_nvenc", vec!["-preset".into(), "p4".into(), "-tune".into(), "hq".into()]),
        EncoderChoice::NvencAv1 => ("av1_nvenc", vec!["-preset".into(), "p4".into(), "-tune".into(), "hq".into()]),
        EncoderChoice::AmfH264 => ("h264_amf", vec!["-usage".into(), "transcoding".into()]),
        EncoderChoice::AmfHevc => ("hevc_amf", vec!["-usage".into(), "transcoding".into()]),
        EncoderChoice::AmfAv1 => ("av1_amf", vec!["-usage".into(), "transcoding".into()]),
        EncoderChoice::QsvH264 => ("h264_qsv", vec!["-preset".into(), "medium".into()]),
        EncoderChoice::QsvHevc => ("hevc_qsv", vec!["-preset".into(), "medium".into()]),
        EncoderChoice::QsvAv1 => ("av1_qsv", vec!["-preset".into(), "medium".into()]),
        EncoderChoice::X264Software => ("libx264", vec!["-preset".into(), "veryfast".into()]),
        EncoderChoice::X265Software => ("libx265", vec!["-preset".into(), "veryfast".into()]),
        // Software AV1, tuned for realtime capture — anything slower can't
        // keep up with a live 60fps feed.
        EncoderChoice::SvtAv1 => ("libsvtav1", vec!["-preset".into(), "10".into()]),
        EncoderChoice::AomAv1 => ("libaom-av1", vec!["-usage".into(), "realtime".into(), "-cpu-used".into(), "8".into(), "-row-mt".into(), "1".into()]),
        EncoderChoice::Auto => unreachable!("Auto must be resolved before encoder_args"),
    };
    args.append(&mut rate_control_args(choice, bitrate_kbps, rate_control, quality));
    (codec, args)
}

/// Vendor-appropriate rate-control flags for the local encode (recording
/// file / replay-buffer segments) — the live stream ignores `rate_control`/
/// `quality` entirely and always forces its own steady feed via
/// `live_cbr_args` (streaming platforms need that regardless of what local
/// files prefer). See `RateControl`'s doc comment for what each mode means;
/// `quality` is the QP/CRF value `Cqp`/`VbrCq` use (ignored otherwise).
///
/// Per-vendor notes on the less-standard modes:
/// - NVENC has a genuine capped-quality mode (`-rc vbr -cq`), so `VbrCq`
///   there is exact.
/// - AMF has no equivalent knob for a *capped* quality mode; `VbrCq`
///   approximates it by combining its QP controls with a bitrate ceiling,
///   which isn't guaranteed to behave identically across driver versions.
/// - QSV's `-global_quality` (paired with `-look_ahead`) is its ICQ mode,
///   close to NVENC's but not identically tuned.
/// - x264/x265/software-AV1 get the well-known "capped CRF" recipe
///   (`-crf` + `-maxrate` + `-bufsize`) for `VbrCq`, which is the standard
///   technique for this on those encoders.
/// - True lossless is native on x264 (`-qp 0`) and x265
///   (`-x265-params lossless=1`); everywhere else `Lossless` falls back to
///   each vendor's minimum-QP mode, which is extremely high quality but not
///   bit-exact lossless.
fn rate_control_args(choice: &EncoderChoice, bitrate_kbps: u32, rate_control: crate::config::RateControl, quality: u32) -> Vec<String> {
    use crate::config::RateControl;
    let b = format!("{bitrate_kbps}k");
    let vbr_max = format!("{}k", (bitrate_kbps as f32 * 1.5).round() as u32);
    let q = quality.to_string();
    match (choice, rate_control) {
        (EncoderChoice::NvencH264 | EncoderChoice::NvencHevc | EncoderChoice::NvencAv1, RateControl::Cbr) => {
            vec!["-rc".into(), "cbr".into(), "-b:v".into(), b]
        }
        (EncoderChoice::NvencH264 | EncoderChoice::NvencHevc | EncoderChoice::NvencAv1, RateControl::Vbr) => {
            vec!["-rc".into(), "vbr".into(), "-b:v".into(), b, "-maxrate".into(), vbr_max]
        }
        (EncoderChoice::NvencH264 | EncoderChoice::NvencHevc | EncoderChoice::NvencAv1, RateControl::Cqp) => {
            vec!["-rc".into(), "constqp".into(), "-qp".into(), q]
        }
        (EncoderChoice::NvencH264 | EncoderChoice::NvencHevc | EncoderChoice::NvencAv1, RateControl::VbrCq) => {
            vec!["-rc".into(), "vbr".into(), "-cq".into(), q, "-b:v".into(), b, "-maxrate".into(), vbr_max]
        }
        (EncoderChoice::NvencH264 | EncoderChoice::NvencHevc | EncoderChoice::NvencAv1, RateControl::Lossless) => {
            vec!["-rc".into(), "constqp".into(), "-qp".into(), "0".into()]
        }

        (EncoderChoice::AmfH264 | EncoderChoice::AmfHevc | EncoderChoice::AmfAv1, RateControl::Cbr) => {
            vec!["-rc".into(), "cbr".into(), "-b:v".into(), b]
        }
        (EncoderChoice::AmfH264 | EncoderChoice::AmfHevc | EncoderChoice::AmfAv1, RateControl::Vbr) => {
            // AMF's peak-constrained VBR — `-b:v` is the average, `-maxrate` the ceiling.
            vec!["-rc".into(), "vbr_peak".into(), "-b:v".into(), b, "-maxrate".into(), vbr_max]
        }
        (EncoderChoice::AmfH264 | EncoderChoice::AmfHevc | EncoderChoice::AmfAv1, RateControl::Cqp) => {
            vec!["-rc".into(), "cqp".into(), "-qp_i".into(), q.clone(), "-qp_p".into(), q.clone(), "-qp_b".into(), q]
        }
        (EncoderChoice::AmfH264 | EncoderChoice::AmfHevc | EncoderChoice::AmfAv1, RateControl::VbrCq) => {
            vec!["-rc".into(), "vbr_peak".into(), "-qp_i".into(), q.clone(), "-qp_p".into(), q, "-b:v".into(), b, "-maxrate".into(), vbr_max]
        }
        (EncoderChoice::AmfH264 | EncoderChoice::AmfHevc | EncoderChoice::AmfAv1, RateControl::Lossless) => {
            vec!["-rc".into(), "cqp".into(), "-qp_i".into(), "0".into(), "-qp_p".into(), "0".into(), "-qp_b".into(), "0".into()]
        }

        (EncoderChoice::QsvH264 | EncoderChoice::QsvHevc | EncoderChoice::QsvAv1, RateControl::Cbr) => {
            vec!["-b:v".into(), b.clone(), "-maxrate".into(), b.clone(), "-minrate".into(), b]
        }
        (EncoderChoice::QsvH264 | EncoderChoice::QsvHevc | EncoderChoice::QsvAv1, RateControl::Vbr) => {
            vec!["-b:v".into(), b, "-maxrate".into(), vbr_max]
        }
        (EncoderChoice::QsvH264 | EncoderChoice::QsvHevc | EncoderChoice::QsvAv1, RateControl::Cqp) => {
            vec!["-global_quality".into(), q]
        }
        (EncoderChoice::QsvH264 | EncoderChoice::QsvHevc | EncoderChoice::QsvAv1, RateControl::VbrCq) => {
            vec!["-look_ahead".into(), "1".into(), "-global_quality".into(), q, "-b:v".into(), b, "-maxrate".into(), vbr_max]
        }
        (EncoderChoice::QsvH264 | EncoderChoice::QsvHevc | EncoderChoice::QsvAv1, RateControl::Lossless) => {
            vec!["-global_quality".into(), "1".into()]
        }

        (EncoderChoice::X264Software, RateControl::Cbr) => vec![
            "-b:v".into(), b.clone(), "-maxrate".into(), b.clone(), "-bufsize".into(), b,
            "-x264-params".into(), "nal-hrd=cbr:force-cfr=1".into(),
        ],
        (EncoderChoice::X264Software, RateControl::Vbr) => vec!["-b:v".into(), b, "-maxrate".into(), vbr_max],
        (EncoderChoice::X264Software, RateControl::Cqp) => vec!["-crf".into(), q],
        (EncoderChoice::X264Software, RateControl::VbrCq) => vec!["-crf".into(), q, "-maxrate".into(), b.clone(), "-bufsize".into(), b],
        (EncoderChoice::X264Software, RateControl::Lossless) => vec!["-qp".into(), "0".into()],

        (EncoderChoice::X265Software, RateControl::Cbr) => vec![
            "-b:v".into(), b.clone(), "-maxrate".into(), b.clone(), "-bufsize".into(), b,
            "-x265-params".into(), "hrd=1:strict-cbr=1".into(),
        ],
        (EncoderChoice::X265Software, RateControl::Vbr) => vec!["-b:v".into(), b, "-maxrate".into(), vbr_max],
        (EncoderChoice::X265Software, RateControl::Cqp) => vec!["-crf".into(), q],
        (EncoderChoice::X265Software, RateControl::VbrCq) => vec!["-crf".into(), q, "-maxrate".into(), b.clone(), "-bufsize".into(), b],
        (EncoderChoice::X265Software, RateControl::Lossless) => vec!["-x265-params".into(), "lossless=1".into()],

        (EncoderChoice::SvtAv1 | EncoderChoice::AomAv1, RateControl::Cbr) => {
            vec!["-b:v".into(), b.clone(), "-maxrate".into(), b.clone(), "-bufsize".into(), b]
        }
        (EncoderChoice::SvtAv1 | EncoderChoice::AomAv1, RateControl::Vbr) => {
            vec!["-b:v".into(), b, "-maxrate".into(), vbr_max]
        }
        (EncoderChoice::SvtAv1 | EncoderChoice::AomAv1, RateControl::Cqp) => vec!["-crf".into(), q],
        (EncoderChoice::SvtAv1 | EncoderChoice::AomAv1, RateControl::VbrCq) => {
            vec!["-crf".into(), q, "-maxrate".into(), b.clone(), "-bufsize".into(), b]
        }
        // Not bit-exact lossless (AV1 software lossless isn't reliably
        // exposed via these simple ffmpeg flags) — the lowest CRF is the
        // closest practical approximation.
        (EncoderChoice::SvtAv1 | EncoderChoice::AomAv1, RateControl::Lossless) => vec!["-crf".into(), "0".into()],

        (EncoderChoice::Auto, _) => unreachable!("Auto must be resolved before encoder_args"),
    }
}

/// Extra rate-control args forcing genuine constant bitrate on the live feed
/// only — plain `-b:v` alone lets several encoders drift ABR-ish. Flags are
/// per-vendor; `buffer_secs` sets the VBV buffer size.
fn live_cbr_args(encoder: &EncoderChoice, bitrate_kbps: u32, buffer_secs: f32) -> Vec<String> {
    let b = format!("{bitrate_kbps}k");
    let bufsize_kbits = ((bitrate_kbps as f32 * buffer_secs).round() as u32).max(1);
    let buf = format!("{bufsize_kbits}k");
    match encoder {
        EncoderChoice::NvencH264 | EncoderChoice::NvencHevc | EncoderChoice::NvencAv1
        | EncoderChoice::AmfH264 | EncoderChoice::AmfHevc | EncoderChoice::AmfAv1 => {
            vec!["-bufsize".into(), buf]
        }
        EncoderChoice::SvtAv1 => vec!["-bufsize".into(), buf],
        EncoderChoice::X264Software => vec![
            "-maxrate".into(), b.clone(), "-bufsize".into(), buf,
            "-x264-params".into(), "nal-hrd=cbr:force-cfr=1".into(),
        ],
        EncoderChoice::X265Software => vec![
            "-maxrate".into(), b.clone(), "-bufsize".into(), buf,
            "-x265-params".into(), "hrd=1:strict-cbr=1".into(),
        ],
        _ => vec!["-maxrate".into(), b.clone(), "-minrate".into(), b, "-bufsize".into(), buf],
    }
}

/// Live-stream-only encode tweaks; never affect the local file, which keeps
/// its own resolution/bitrate/fps regardless.
#[derive(Clone)]
pub struct LiveStreamParams {
    pub rtmp_url: String,
    /// YouTube-capped bitrate/resolution for the live feed only — already
    /// the effective (capped) values, never the raw local-recording settings.
    pub bitrate_kbps: u32,
    pub resolution: crate::config::RecordingResolution,
    pub max_fps: u32,
    pub keyframe_interval_secs: u32,
    pub audio_codec: crate::config::AudioCodec,
    pub audio_sample_rate: u32,
    /// VBV buffer size, in seconds of the target bitrate — see `live_cbr_args`.
    pub cbr_buffer_secs: f32,
}

/// `Auto` resolution order: NVENC H.264 > AMF H.264 > software x264. Cached
/// per app run — see `AutoEncoderCache`.
pub async fn resolve_auto(app: &AppHandle, ffmpeg_path: &Path) -> EncoderChoice {
    use tauri::Manager;
    if let Some(cache) = app.try_state::<AutoEncoderCache>() {
        if let Some(cached) = cache.0.lock().unwrap().clone() {
            return cached;
        }
    }
    let mut resolved = EncoderChoice::X264Software;
    for candidate in [EncoderChoice::NvencH264, EncoderChoice::AmfH264, EncoderChoice::QsvH264] {
        if probe_encoder(app, ffmpeg_path, &candidate).await {
            resolved = candidate;
            break;
        }
    }
    if let Some(cache) = app.try_state::<AutoEncoderCache>() {
        *cache.0.lock().unwrap() = Some(resolved.clone());
    }
    resolved
}

async fn probe_encoder(app: &AppHandle, _ffmpeg_path: &Path, choice: &EncoderChoice) -> bool {
    let (codec, _) = encoder_args(choice, 1000, crate::config::RateControl::Cbr, 23);
    let Ok(cmd) = crate::integrity::ffmpeg_sidecar(app) else { return false };
    // 640x360, not something tiny: hardware encoders enforce minimum frame
    // dimensions (NVENC rejects 64x64 with "Frame Dimension less than the
    // minimum supported value", reading as "unavailable" on capable GPUs).
    let args = [
        "-hide_banner", "-loglevel", "error", "-y",
        "-f", "lavfi", "-i", "color=black:s=640x360",
        "-frames:v", "1", "-c:v", codec, "-f", "null", "-",
    ];
    let Ok(output) = cmd.args(args).output().await else { return false };
    let ok = output.status.success();
    if !ok {
        log::debug!("encoder probe {codec}: {}", String::from_utf8_lossy(&output.stderr).lines().next().unwrap_or(""));
    }
    ok
}

/// The frame size ffmpeg expects via `-s` — `yuv420p` needs even dimensions.
/// Every raw frame buffer must be fit to this same size first (see
/// `fit_frame_to_dims`), or a stride mismatch shows up as a diagonal shear.
pub(crate) fn even_dims(width: u32, height: u32) -> (u32, u32) {
    (width - (width % 2), height - (height % 2))
}

/// Scales a raw BGRA buffer to exactly `dst_w`x`dst_h` — needed because the
/// source (e.g. a recorded window) can resize mid-recording. ffmpeg's raw-video stdin pipe is opened once with a fixed `-s WxH` and
/// can never change size for the life of that process; every frame after
/// that point must keep matching those exact bytes, or the row stride
/// ffmpeg assumes no longer lines up with what's actually being sent and
/// every subsequent frame decodes as sheared/corrupted garbage. Plain
/// stretching (not aspect-ratio-preserving) — simple, and a distorted frame
/// beats a corrupted one.
///
/// `image::imageops::resize` only does per-channel weighted averaging, so
/// treating BGRA bytes as if they were RGBA for this purpose is safe — nothing
/// here is colorspace-aware, it doesn't matter which channel is which.
pub(crate) fn fit_frame_to_dims(bgra: &[u8], src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> std::borrow::Cow<'_, [u8]> {
    if src_w == dst_w && src_h == dst_h {
        return std::borrow::Cow::Borrowed(bgra);
    }
    let Some(src_img) = image::RgbaImage::from_raw(src_w, src_h, bgra.to_vec()) else {
        return std::borrow::Cow::Owned(vec![0u8; dst_w as usize * dst_h as usize * 4]);
    };
    let resized = image::imageops::resize(&src_img, dst_w, dst_h, image::imageops::FilterType::Triangle);
    std::borrow::Cow::Owned(resized.into_raw())
}

pub struct EncodeJob {
    pub child: CommandChild,
}

impl EncodeJob {
    /// Writes one frame's raw BGRA bytes to ffmpeg's stdin. Blocking — call
    /// from a dedicated writer thread, never from the capture callback.
    pub fn write_frame(&mut self, bgra: &[u8]) -> Result<(), String> {
        self.child.write(bgra).map_err(|e| e.to_string())
    }

    /// Closes ffmpeg's stdin (EOF) — the graceful-stop path, not a kill.
    /// `allow_kill`: only the live job may be force-killed past a grace
    /// period (no file at risk); local waits indefinitely to avoid writing
    /// a corrupt recording.
    pub fn finish(self, allow_kill: bool) {
        let pid = self.child.pid();
        drop(self.child);
        if allow_kill {
            crate::win_util::wait_or_kill_process(pid, 15_000);
        }
    }
}

/// Owns one active `EncodeJob` on its own thread. Frames arrive via `try_send`
/// on a bounded channel, so a busy feeder drops frames instead of blocking
/// the caller's own pacing loop on a stalled ffmpeg — critical for any writer
/// loop that also paces real-time capture (a blocking write there stalls the
/// whole loop, so fewer frames land per real elapsed second than the encoder
/// is told (`-r fps`) to assume, which silently shortens the encoded
/// duration and makes playback look sped up).
pub struct JobFeeder {
    tx: Option<std::sync::mpsc::SyncSender<Arc<[u8]>>>,
    thread: Option<JoinHandle<()>>,
}

impl JobFeeder {
    pub fn spawn(mut job: EncodeJob, kind: &'static str) -> Self {
        // A couple of frames of slack — absorbs a brief hiccup without
        // becoming a stale-frame latency buffer once the job falls behind.
        let (tx, rx) = std::sync::mpsc::sync_channel::<Arc<[u8]>>(2);
        // Only the live job's stop may force-kill (see `EncodeJob::finish`).
        let allow_kill = kind == "live stream";
        let thread = std::thread::spawn(move || {
            while let Ok(frame) = rx.recv() {
                if let Err(e) = job.write_frame(&frame) {
                    log::error!("failed to write frame to {kind}: {e}");
                    break;
                }
            }
            job.finish(allow_kill);
        });
        Self { tx: Some(tx), thread: Some(thread) }
    }

    /// Hands a frame to this job's thread. Returns `false` once that thread
    /// has exited (write error or ffmpeg quit) so the caller drops the feeder.
    pub fn send(&self, frame: Arc<[u8]>) -> bool {
        match &self.tx {
            Some(tx) => !matches!(tx.try_send(frame), Err(std::sync::mpsc::TrySendError::Disconnected(_))),
            None => false,
        }
    }

    /// Disconnects this job's channel; the feeder thread then finishes the
    /// job itself. Doesn't wait — session-stop disconnects both jobs before
    /// joining either, so their catch-up times don't serialize.
    pub fn disconnect(&mut self) {
        self.tx = None;
    }

    /// `disconnect()` plus the blocking join, for a single-job toggle-off.
    pub fn stop(mut self) {
        self.disconnect();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Appends the downscale filter for a resolution cap (`min(H, ih)` — never
/// upscales; input dimensions are already even, `-2` keeps width even).
fn push_scale_args(args: &mut Vec<String>, resolution: &crate::config::RecordingResolution) {
    if let Some(h) = resolution.height() {
        args.push("-vf".into());
        args.push(format!("scale=-2:'min({h},ih)'"));
    }
}

/// `push_scale_args`'s scaling merged with an explicit `fps=` filter into one
/// chain — ffmpeg only accepts one `-vf` per output. The `fps` filter
/// duplicates/drops frames to conform the input's actual timestamps to a
/// constant rate; see the replay buffer's `spawn_segmented`/`spawn_fmp4_stream`
/// for why that matters beyond capping the live feed's rate.
fn scale_and_fps_filter(resolution: &crate::config::RecordingResolution, fps: u32) -> String {
    let mut parts = Vec::new();
    if let Some(h) = resolution.height() {
        parts.push(format!("scale=-2:'min({h},ih)'"));
    }
    parts.push(format!("fps={fps}"));
    parts.join(",")
}

/// Appends the `-c:a` (and bitrate) args for the configured audio codec.
/// Non-AAC codecs also get `-strict -2`, since opus/flac in mp4/mov are
/// gated behind "experimental" in some ffmpeg builds.
fn push_audio_codec_args(args: &mut Vec<String>, audio_count: usize, codec: &crate::config::AudioCodec) {
    if audio_count == 0 {
        return;
    }
    let (name, bitrate) = codec.ffmpeg_args();
    args.push("-c:a".into());
    args.push(name.into());
    if let Some(b) = bitrate {
        args.push("-b:a".into());
        args.push(b.into());
    }
    if !matches!(codec, crate::config::AudioCodec::Aac) {
        args.push("-strict".into());
        args.push("-2".into());
    }
}

/// One audio source's connection details by the time it reaches ffmpeg: a
/// local TCP relay ffmpeg connects to as a network input, since the shell
/// plugin only exposes a single stdin pipe (already used by the video stream).
pub struct AudioStreamSpec {
    pub port: u16,
    pub sample_rate: u32,
    pub channels: u16,
    pub sample_fmt: &'static str, // "s16le" | "f32le"
    pub label: String,
    /// Whether this source is part of the first ("Mix") track — the track
    /// YouTube (live and uploads) and most players treat as THE audio. See
    /// `push_audio_track_maps`.
    pub main_mix: bool,
    /// Single-track mode (separate-tracks off): everything collapses into
    /// one weighted mixdown, no individual tracks. Set uniformly on every
    /// spec of a session by `start_configured_audio_sources`.
    pub mix_only: bool,
    /// This source's `amix` weight in single-track mode (1.0 = untouched;
    /// ducked sources get less) — carries the input-vs-game priority.
    pub weight: f32,
}

/// The weighted `amix` filter string for the given (1-based-input) indices.
fn amix_filter(audio: &[AudioStreamSpec], indices: &[usize], out: &str) -> String {
    let inputs: String = indices.iter().map(|i| format!("[{}:a]", i + 1)).collect();
    let weighted = indices.iter().any(|&i| (audio[i].weight - 1.0).abs() > f32::EPSILON);
    let weights = if weighted {
        format!(
            ":weights={}",
            indices.iter().map(|&i| format!("{}", audio[i].weight)).collect::<Vec<_>>().join(" ")
        )
    } else {
        String::new()
    };
    format!("{inputs}amix=inputs={}:duration=longest:normalize=0{weights}[{out}]", indices.len())
}

/// Maps audio inputs onto output tracks, main-mix-first: with 2+ sources
/// marked for the mix, track 1 is an `amix` of them followed by each source
/// verbatim. Returns the track count mapped.
fn push_audio_track_maps(args: &mut Vec<String>, audio: &[AudioStreamSpec]) -> usize {
    if audio.is_empty() {
        return 0;
    }

    // Single-track mode: one weighted mixdown of everything, nothing else.
    if audio.iter().any(|a| a.mix_only) {
        if audio.len() == 1 {
            args.push("-map".into());
            args.push("1:a".into());
        } else {
            let all: Vec<usize> = (0..audio.len()).collect();
            args.push("-filter_complex".into());
            args.push(amix_filter(audio, &all, "amain"));
            args.push("-map".into());
            args.push("[amain]".into());
        }
        args.push("-metadata:s:a:0".into());
        args.push("title=Audio".into());
        return 1;
    }

    let mains: Vec<usize> = audio.iter().enumerate().filter(|(_, a)| a.main_mix).map(|(i, _)| i).collect();

    if audio.len() >= 2 && mains.len() >= 2 {
        args.push("-filter_complex".into());
        args.push(amix_filter(audio, &mains, "amain"));
        args.push("-map".into());
        args.push("[amain]".into());
        args.push("-metadata:s:a:0".into());
        args.push("title=Mix".into());
        for (i, a) in audio.iter().enumerate() {
            args.push("-map".into());
            args.push(format!("{}:a", i + 1));
            args.push(format!("-metadata:s:a:{}", i + 1));
            args.push(format!("title={}", a.label));
        }
        return audio.len() + 1;
    }

    // One (or zero) mix members: order the chosen one first, no filtering.
    let order: Vec<usize> = mains
        .iter()
        .copied()
        .chain((0..audio.len()).filter(|i| !mains.contains(i)))
        .collect();
    for (t, i) in order.iter().enumerate() {
        args.push("-map".into());
        args.push(format!("{}:a", i + 1));
        args.push(format!("-metadata:s:a:{t}"));
        args.push(format!("title={}", audio[*i].label));
    }
    audio.len()
}

/// The per-track title labels `push_audio_track_maps` assigns, in the same
/// order and count (mirror its branches — keep in sync). Needed because
/// MPEG-TS (the replay buffer's segment format for H.264/HEVC) has no place
/// to carry the `-metadata:s:a:N title=...` tags the encoder writes: they're
/// silently dropped on mux, so a segment read back has no track titles at
/// all. The replay buffer save path re-derives this list from the same audio
/// specs and reapplies it as fresh `-metadata:s:a:N` on the final concat,
/// which — as an output-side option — attaches regardless of what the
/// intermediate segments carried.
pub fn audio_track_titles(audio: &[AudioStreamSpec]) -> Vec<String> {
    if audio.is_empty() {
        return Vec::new();
    }
    if audio.iter().any(|a| a.mix_only) {
        return vec!["Audio".to_string()];
    }
    let mains: Vec<usize> = audio.iter().enumerate().filter(|(_, a)| a.main_mix).map(|(i, _)| i).collect();
    if audio.len() >= 2 && mains.len() >= 2 {
        let mut titles = vec!["Mix".to_string()];
        titles.extend(audio.iter().map(|a| a.label.clone()));
        return titles;
    }
    let order: Vec<usize> = mains.iter().copied().chain((0..audio.len()).filter(|i| !mains.contains(i))).collect();
    order.iter().map(|&i| audio[i].label.clone()).collect()
}

/// Shared rawvideo-from-stdin + per-source TCP-relay input args, common to
/// every encode process regardless of what it eventually outputs to.
fn raw_input_args(even_w: u32, even_h: u32, fps: u32, audio: &[AudioStreamSpec]) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-y".into(),
        "-f".into(), "rawvideo".into(),
        "-pix_fmt".into(), "bgra".into(),
        "-s".into(), format!("{even_w}x{even_h}"),
        "-r".into(), fps.to_string(),
        "-i".into(), "-".into(),
    ];
    for a in audio {
        args.push("-f".into());
        args.push(a.sample_fmt.into());
        args.push("-ar".into());
        args.push(a.sample_rate.to_string());
        args.push("-ac".into());
        args.push(a.channels.to_string());
        args.push("-i".into());
        args.push(format!("tcp://127.0.0.1:{}", a.port));
    }
    // Machine-readable progress on stdout once a second, for the finish
    // watcher's "actual bitrate" stat.
    args.extend(["-progress".into(), "pipe:1".into(), "-stats_period".into(), "1".into(), "-nostats".into()]);
    args
}

fn spawn_ffmpeg(app: &AppHandle, args: Vec<String>) -> Result<(EncodeJob, tauri::async_runtime::Receiver<CommandEvent>), String> {
    let cmd = crate::integrity::ffmpeg_sidecar(app).map_err(|e| e.to_string())?;
    let (rx, child) = cmd.args(args).spawn().map_err(|e| e.to_string())?;
    Ok((EncodeJob { child }, rx))
}

/// Spawns ffmpeg for the local recording file only — uncapped resolution/
/// bitrate, one output track per audio source. Fully independent from
/// `spawn_live`, so starting/stopping one never touches the other.
#[allow(clippy::too_many_arguments)]
pub fn spawn_local(
    app: &AppHandle,
    width: u32,
    height: u32,
    fps: u32,
    encoder: &EncoderChoice,
    bitrate_kbps: u32,
    rate_control: crate::config::RateControl,
    quality: u32,
    audio: &[AudioStreamSpec],
    audio_codec: &crate::config::AudioCodec,
    resolution: &crate::config::RecordingResolution,
    container: &crate::config::VideoContainer,
    output_path: &Path,
) -> Result<(EncodeJob, tauri::async_runtime::Receiver<CommandEvent>), String> {
    let (even_w, even_h) = even_dims(width, height);
    let mut args = raw_input_args(even_w, even_h, fps, audio);

    let (codec, mut rate_args) = encoder_args(encoder, bitrate_kbps, rate_control, quality);
    args.push("-map".into());
    args.push("0:v".into());
    args.push("-c:v".into());
    args.push(codec.into());
    args.append(&mut rate_args);
    args.push("-pix_fmt".into());
    args.push("yuv420p".into());
    push_scale_args(&mut args, resolution);
    // A short, explicit GOP (muxers write in keyframe-aligned chunks) keeps
    // a reader tailing the growing file close to the live edge.
    args.push("-g".into());
    args.push((fps * 2).to_string());
    // Flushes each muxed chunk to disk immediately instead of sitting in
    // ffmpeg's internal I/O buffer — otherwise the short GOP buys nothing.
    args.push("-flush_packets".into());
    args.push("1".into());

    let track_count = push_audio_track_maps(&mut args, audio);
    push_audio_codec_args(&mut args, track_count, audio_codec);

    if let Some(flags) = container.movflags() {
        args.push("-movflags".into());
        args.push(flags.into());
    }
    args.push(output_path.to_string_lossy().into_owned());

    spawn_ffmpeg(app, args)
}

/// Spawns ffmpeg for the live feed only — a separate process from
/// `spawn_local`. FLV carries exactly one audio track, so every source is
/// always mixed down here regardless of the local file's track layout.
pub fn spawn_live(
    app: &AppHandle,
    width: u32,
    height: u32,
    fps: u32,
    encoder: &EncoderChoice,
    audio: &[AudioStreamSpec],
    params: &LiveStreamParams,
) -> Result<(EncodeJob, tauri::async_runtime::Receiver<CommandEvent>), String> {
    let (even_w, even_h) = even_dims(width, height);
    let mut args = raw_input_args(even_w, even_h, fps, audio);

    args.push("-map".into());
    args.push("0:v".into());
    // Mix the sources the user marked for the main mix (all of them in
    // single-track mode or when none are marked; normalize=0 keeps
    // original loudness over 1/n scaling, weights carry mix_priority).
    let mix_only = audio.iter().any(|a| a.mix_only);
    let mut mix: Vec<usize> = if mix_only {
        (0..audio.len()).collect()
    } else {
        audio.iter().enumerate().filter(|(_, a)| a.main_mix).map(|(i, _)| i).collect()
    };
    if mix.is_empty() {
        mix = (0..audio.len()).collect();
    }
    if mix.len() > 1 {
        args.push("-filter_complex".into());
        args.push(amix_filter(audio, &mix, "aout"));
        args.push("-map".into());
        args.push("[aout]".into());
    } else if mix.len() == 1 {
        args.push("-map".into());
        args.push(format!("{}:a", mix[0] + 1));
    }
    // Always CBR regardless of the local recording's rate-control preference
    // — `live_cbr_args` below layers its own strict constraints on top
    // either way, and a steady feed is what RTMP/YouTube actually need.
    let (live_codec, mut live_rate_args) = encoder_args(encoder, params.bitrate_kbps, crate::config::RateControl::Cbr, 23);
    args.push("-c:v".into());
    args.push(live_codec.into());
    args.append(&mut live_rate_args);
    args.append(&mut live_cbr_args(encoder, params.bitrate_kbps, params.cbr_buffer_secs));
    args.push("-pix_fmt".into());
    args.push("yuv420p".into());
    // One combined filter — resolution cap and the live-only fps ceiling
    // both resample the same stream, and ffmpeg only takes one `-vf` chain
    // per output.
    args.push("-vf".into());
    args.push(scale_and_fps_filter(&params.resolution, params.max_fps));
    args.push("-g".into());
    args.push((fps * params.keyframe_interval_secs).to_string());
    if !audio.is_empty() {
        let (aname, abitrate) = params.audio_codec.ffmpeg_args();
        args.extend(["-c:a".into(), aname.into()]);
        if let Some(ab) = abitrate {
            args.extend(["-b:a".into(), ab.into()]);
        }
        args.extend(["-ar".into(), params.audio_sample_rate.to_string()]);
    }
    args.extend(["-f".into(), "flv".into(), params.rtmp_url.clone()]);

    spawn_ffmpeg(app, args)
}

/// Spawns ffmpeg for the memory replay buffer, muxed as a continuous
/// **fragmented MP4** stream to stdout. Unlike MPEG-TS (the old ring format),
/// fMP4 carries H.264/HEVC/AV1 and every audio codec we offer; its `ftyp`+
/// `moov` init segment followed by one keyframe-started `moof`+`mdat` fragment
/// per GOP is exactly what lets the RAM ring keep the init once and evict/cut
/// on fragment boundaries, so any age-trimmed snapshot still remuxes cleanly
/// from a keyframe (see `MemRing`).
pub fn spawn_fmp4_stream(
    app: &AppHandle,
    width: u32,
    height: u32,
    fps: u32,
    encoder: &EncoderChoice,
    bitrate_kbps: u32,
    rate_control: crate::config::RateControl,
    quality: u32,
    audio: &[AudioStreamSpec],
    audio_codec: &crate::config::AudioCodec,
    resolution: &crate::config::RecordingResolution,
) -> Result<(EncodeJob, tauri::async_runtime::Receiver<CommandEvent>), String> {
    let (codec, mut rate_args) = encoder_args(encoder, bitrate_kbps, rate_control, quality);
    let (even_w, even_h) = even_dims(width, height);

    let mut args: Vec<String> = vec![
        "-y".into(),
        "-f".into(), "rawvideo".into(),
        "-pix_fmt".into(), "bgra".into(),
        "-s".into(), format!("{even_w}x{even_h}"),
        // No `-r` here: rawvideo has its own fixed-count-based PTS generator
        // that overrides `-use_wallclock_as_timestamps` whenever `-r` is also
        // given, silently defeating it (confirmed empirically). Without `-r`,
        // each frame is timestamped from when it actually arrived — critical
        // because a stalled/overloaded encoder can otherwise make fewer
        // frames land per real second than a fixed rate assumes, which
        // silently shrinks the encoded duration below the real elapsed
        // capture time (plays back sped up). `scale_and_fps_filter`'s `fps=`
        // filter below reconforms this to a strict constant rate afterwards,
        // duplicating/dropping based on these real timestamps.
        "-use_wallclock_as_timestamps".into(), "1".into(),
        "-i".into(), "-".into(),
    ];
    for a in audio {
        args.push("-f".into());
        args.push(a.sample_fmt.into());
        args.push("-ar".into());
        args.push(a.sample_rate.to_string());
        args.push("-ac".into());
        args.push(a.channels.to_string());
        args.push("-i".into());
        args.push(format!("tcp://127.0.0.1:{}", a.port));
    }

    args.push("-map".into());
    args.push("0:v".into());
    args.push("-c:v".into());
    args.push(codec.into());
    args.append(&mut rate_args);
    args.push("-pix_fmt".into());
    args.push("yuv420p".into());
    args.push("-vf".into());
    args.push(scale_and_fps_filter(resolution, fps));
    // Frequent keyframes bound how much footage is unrecoverable when
    // remuxing from an arbitrary cut (~2s).
    args.push("-g".into());
    args.push((fps * 2).to_string());

    let track_count = push_audio_track_maps(&mut args, audio);
    push_audio_codec_args(&mut args, track_count, audio_codec);

    // Fragmented MP4 to stdout: `empty_moov` emits the init segment up front,
    // `frag_keyframe` starts a fresh fragment at every keyframe (one per GOP,
    // via `-g` above), and `default_base_moof` keeps each fragment positionally
    // self-contained. All three together are also what make the muxer
    // pipe-safe (it never needs to seek back to patch a header).
    args.extend([
        "-movflags".into(), "+frag_keyframe+empty_moov+default_base_moof".into(),
        "-f".into(), "mp4".into(), "pipe:1".into(),
    ]);

    let cmd = crate::integrity::ffmpeg_sidecar(app).map_err(|e| e.to_string())?;
    let (rx, child) = cmd.args(args).spawn().map_err(|e| e.to_string())?;
    Ok((EncodeJob { child }, rx))
}

pub fn spawn_segmented(
    app: &AppHandle,
    width: u32,
    height: u32,
    fps: u32,
    encoder: &EncoderChoice,
    bitrate_kbps: u32,
    rate_control: crate::config::RateControl,
    quality: u32,
    audio: &[AudioStreamSpec],
    audio_codec: &crate::config::AudioCodec,
    resolution: &crate::config::RecordingResolution,
    segment_seconds: u32,
    segment_dir: &Path,
) -> Result<(EncodeJob, tauri::async_runtime::Receiver<CommandEvent>), String> {
    let (codec, mut rate_args) = encoder_args(encoder, bitrate_kbps, rate_control, quality);
    let (even_w, even_h) = even_dims(width, height);
    let container = segment_container_for(encoder);

    let pattern = segment_dir.join(format!("seg_%Y%m%d_%H%M%S.{}", container.ext()));
    let mut args: Vec<String> = vec![
        "-y".into(),
        "-f".into(), "rawvideo".into(),
        "-pix_fmt".into(), "bgra".into(),
        "-s".into(), format!("{even_w}x{even_h}"),
        // No `-r` — see `spawn_fmp4_stream`'s identical input args for why:
        // it silently overrides `-use_wallclock_as_timestamps`, defeating the
        // real-time-accurate PTS that keeps a stalled encoder from shrinking
        // the encoded duration below the real elapsed capture time (which
        // otherwise makes saved clips play back sped up).
        "-use_wallclock_as_timestamps".into(), "1".into(),
        "-i".into(), "-".into(),
    ];
    for a in audio {
        args.push("-f".into());
        args.push(a.sample_fmt.into());
        args.push("-ar".into());
        args.push(a.sample_rate.to_string());
        args.push("-ac".into());
        args.push(a.channels.to_string());
        args.push("-i".into());
        args.push(format!("tcp://127.0.0.1:{}", a.port));
    }

    args.push("-map".into());
    args.push("0:v".into());
    args.push("-c:v".into());
    args.push(codec.into());
    args.append(&mut rate_args);
    args.push("-pix_fmt".into());
    args.push("yuv420p".into());
    args.push("-vf".into());
    args.push(scale_and_fps_filter(resolution, fps));
    // Frequent keyframes keep each 15s segment boundary close to a real
    // keyframe — the segment muxer can only cut at one, so without this an
    // encoder's default (much longer) GOP could push a cut well past 15s.
    args.push("-g".into());
    args.push((fps * 2).to_string());

    let track_count = push_audio_track_maps(&mut args, audio);
    push_audio_codec_args(&mut args, track_count, audio_codec);

    args.push("-f".into());
    args.push("segment".into());
    // Both inner containers keep the "currently-open segment is always
    // valid/readable" property the buffer relies on (no moov index to finalize
    // per rotation). MPEG-TS is the robust default for H.264/HEVC; AV1 must use
    // fragmented MP4 instead, because MPEG-TS muxes AV1 as opaque private data
    // (stream_type 0x06), which is undecodable and makes the save-clip concat
    // produce an empty file. `segment_container_for` picks per codec.
    match container {
        SegmentContainer::MpegTs => {
            args.push("-segment_format".into());
            args.push("mpegts".into());
        }
        SegmentContainer::FragMp4 => {
            args.push("-segment_format".into());
            args.push("mp4".into());
            args.push("-segment_format_options".into());
            args.push("movflags=+frag_keyframe+empty_moov+default_base_moof".into());
        }
    }
    args.push("-segment_time".into());
    args.push(segment_seconds.to_string());
    args.push("-reset_timestamps".into());
    args.push("1".into());
    args.push("-strftime".into());
    args.push("1".into());
    args.push(pattern.to_string_lossy().into_owned());

    let cmd = crate::integrity::ffmpeg_sidecar(app).map_err(|e| e.to_string())?;
    let (rx, child) = cmd.args(args).spawn().map_err(|e| e.to_string())?;
    Ok((EncodeJob { child }, rx))
}
