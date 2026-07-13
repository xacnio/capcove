pub mod audio_capture;
pub mod capture_session;
pub mod encoder;
#[cfg(windows)]
pub mod game_detect;
pub mod hud;
pub mod placeholder;
pub mod replay_buffer;

use std::io::Write;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use chrono::Local;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::config::{AudioSource, ConfigStore, EncoderChoice};
use audio_capture::{AudioCaptureHandle, AudioFlow};
use capture_session::{CaptureHandle, FrameMailbox};
use encoder::AudioStreamSpec;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RecordTarget {
    Window { hwnd: u32, title: String, app: String },
    Monitor,
    Area { x: i32, y: i32, w: u32, h: u32 },
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingSession {
    pub id: String,
    pub target: RecordTarget,
    pub output_path: PathBuf,
    pub started_at: i64,
    pub fps: u32,
    /// Whether this session is also streamed to YouTube; refreshed from the
    /// live atomics on every read, so mid-session toggles show immediately.
    #[serde(default)]
    pub live: bool,
    /// Whether the local file is currently being written.
    #[serde(default = "default_true")]
    pub local: bool,
    /// Resolved (never `Auto`) encoder and effective bitrate/resolution the
    /// session actually started with, after per-game overrides.
    pub encoder: crate::config::EncoderChoice,
    pub bitrate_kbps: u32,
    pub resolution: crate::config::RecordingResolution,
    /// The live feed's own target bitrate/fps/resolution (post YouTube cap),
    /// distinct from the local file's; `None` whenever `live` is false.
    #[serde(default)]
    pub live_bitrate_kbps: Option<u32>,
    #[serde(default)]
    pub live_fps: Option<u32>,
    #[serde(default)]
    pub live_resolution: Option<crate::config::RecordingResolution>,
    /// Relative name of the local file currently being written; `None` when
    /// `local` is false. Can differ from `output_path`: each local off/on
    /// cycle starts a fresh file.
    #[serde(default)]
    pub current_local_name: Option<String>,
    /// Same file as an absolute path, for playing the growing file in the gallery.
    #[serde(default)]
    pub current_local_path: Option<PathBuf>,
}

/// Runtime toggle messages for the writer thread — the local file and the
/// live stream toggle independently while the underlying capture keeps running.
enum WriterControl {
    /// (Re)start the local file at a fresh path; each off/on cycle produces
    /// a separate file rather than resuming the old one.
    EnableLocal(PathBuf),
    DisableLocal,
    /// (Re)start the live feed; the broadcast must already exist (creating
    /// one is an async call the sync writer thread can't make).
    EnableLive(Box<encoder::LiveStreamParams>),
    DisableLive,
}

pub struct ActiveRecording {
    pub session: RecordingSession,
    capture_handle: Option<CaptureHandle>,
    mailbox: Arc<FrameMailbox>,
    writer_thread: Option<JoinHandle<()>>,
    audio_handles: Vec<RelayedAudioCapture>,
    /// User-requested pause: while set, the writer skips video frames and
    /// the audio callbacks drop their bytes (all tracks stay in lockstep).
    manual_paused: Arc<std::sync::atomic::AtomicBool>,
    /// Current YouTube broadcast id while streaming is active; `None` once
    /// toggled off. Closed out best-effort on stop/cancel.
    live_broadcast: Arc<Mutex<Option<String>>>,
    /// Whether the local file / live feed are active right now, each
    /// independently toggleable.
    local_active: Arc<std::sync::atomic::AtomicBool>,
    live_active: Arc<std::sync::atomic::AtomicBool>,
    /// Every local file this session produced (one per local on/off cycle);
    /// Discard deletes all of them.
    local_files: Arc<Mutex<Vec<PathBuf>>>,
    /// Tells the writer thread to (re)start/stop either output.
    control_tx: std::sync::mpsc::Sender<WriterControl>,
    /// Context needed to rebuild a fresh local file path or live broadcast
    /// mid-session.
    target: RecordTarget,
    video: crate::config::VideoSettings,
    output_dir: PathBuf,
}

#[derive(Default)]
pub struct RecordingManager {
    pub active: Mutex<Option<ActiveRecording>>,
    /// Set for the whole span between a start being accepted and `active`
    /// actually being populated. `prepare()` through the point each entry
    /// point (`start_window_recording_live` etc.) finally sets `active` does
    /// a lot of `.await`ing — encoder resolution, audio device negotiation,
    /// capture startup — so checking only `active.is_some()` at the top left
    /// a wide race window where two concurrent start calls could both pass
    /// that check and both fully start, the second silently overwriting the
    /// first's still-running session (leaking its capture/writer/audio
    /// resources forever, since nothing keeps a handle to stop them anymore).
    starting: std::sync::atomic::AtomicBool,
    /// The detached teardown thread `teardown_active` spawns for the most
    /// recent `stop_recording` — starting a new session while the previous
    /// one's capture/encoder/audio threads are still shutting down risks
    /// contending for the same capture device or GPU encoder session.
    /// `prepare()` joins this (if present) before proceeding.
    pending_teardown: Mutex<Option<JoinHandle<()>>>,
}

/// Releases `RecordingManager::starting` on drop — including on an early `?`
/// return from anywhere between `prepare()` acquiring it and the caller
/// finally setting `active`, so a failed start never leaves new attempts
/// permanently locked out.
struct StartGuard(Arc<RecordingManager>);

impl Drop for StartGuard {
    fn drop(&mut self) {
        self.0.starting.store(false, std::sync::atomic::Ordering::Release);
    }
}

impl RecordingManager {
    pub fn is_recording(&self) -> bool {
        self.active.lock().unwrap().is_some()
    }

    pub fn current_session(&self) -> Option<RecordingSession> {
        self.active.lock().unwrap().as_ref().map(|a| {
            let mut s = a.session.clone();
            s.live = a.live_active.load(std::sync::atomic::Ordering::Relaxed);
            s.local = a.local_active.load(std::sync::atomic::Ordering::Relaxed);
            s
        })
    }

    /// True if `path` is the local file an in-progress recording is writing
    /// to (checked against both `output_path` and `current_local_path`) —
    /// used to keep ffmpeg/ffprobe from touching a still-growing file.
    pub fn is_recording_path(&self, path: &std::path::Path) -> bool {
        self.active.lock().unwrap().as_ref().is_some_and(|a| {
            a.session.output_path == path || a.session.current_local_path.as_deref() == Some(path)
        })
    }

    /// Pauses/resumes the active recording. Returns false when nothing is
    /// recording.
    pub fn set_paused(&self, paused: bool) -> bool {
        match self.active.lock().unwrap().as_ref() {
            Some(a) => {
                a.manual_paused.store(paused, std::sync::atomic::Ordering::Relaxed);
                true
            }
            None => false,
        }
    }

    pub fn is_paused(&self) -> bool {
        self.active
            .lock()
            .unwrap()
            .as_ref()
            .map(|a| a.manual_paused.load(std::sync::atomic::Ordering::Relaxed))
            .unwrap_or(false)
    }
}

/// Smallest frame the encoders are asked to handle — hardware encoders
/// reject tiny surfaces, so writers wait for a frame at least this big.
pub(crate) const MIN_CAPTURE_W: u32 = 240;
pub(crate) const MIN_CAPTURE_H: u32 = 160;

/// Hardware encoders' real minimum (NVENC rejects 64x64 outright) — below
/// `MIN_CAPTURE_W/H` writers wait for a bigger frame, but only up to
/// `TINY_FRAME_GRACE` before falling back to whatever size clears this floor,
/// so a legitimately small window (e.g. a compact media player) isn't stuck
/// waiting forever for a resize that will never happen.
pub(crate) const HARD_MIN_CAPTURE_W: u32 = 96;
pub(crate) const HARD_MIN_CAPTURE_H: u32 = 96;
pub(crate) const TINY_FRAME_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

/// Generates a collision-free destination path without writing any data.
/// `prefix` (e.g. `"Clip_"` for replay-buffer saves, `""` for a full
/// recording) goes before the timestamp — `video_thumb`'s
/// `timestamp_from_filename` strips it back off before parsing the date.
pub(crate) fn make_video_save_path(dir: &std::path::Path, ext: &str, prefix: &str) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let now = Local::now();
    let stamp = format!("{prefix}{}", now.format("%Y-%m-%d_%H-%M-%S-%3f"));
    let mut path = dir.join(format!("{stamp}.{ext}"));
    let mut counter = 1;
    while path.exists() {
        path = dir.join(format!("{stamp}_{counter}.{ext}"));
        counter += 1;
    }
    Ok(path)
}

/// `path` relative to the recordings root — the key every video-file lookup
/// uses. Falls back to the bare filename when not under the root.
pub(crate) fn relative_video_name(app: &AppHandle, path: &std::path::Path) -> String {
    let recordings_dir = app.state::<Arc<ConfigStore>>().get().resolved_recordings_dir();
    path.strip_prefix(&recordings_dir)
        .ok()
        .and_then(|p| p.to_str())
        .map(|s| s.replace('\\', "/"))
        .or_else(|| path.file_name().and_then(|n| n.to_str()).map(str::to_string))
        .unwrap_or_default()
}

pub(crate) async fn resolve_encoder(app: &AppHandle, choice: &EncoderChoice) -> EncoderChoice {
    match choice {
        EncoderChoice::Auto => encoder::resolve_auto(app, std::path::Path::new("ffmpeg")).await,
        other => other.clone(),
    }
}

/// Substitutes `{game}`, `{date}`, `{time}`, `{datetime}` tokens in the live
/// title template; `{game}` falls back to "Capcove" when no game is detected.
fn render_title_template(template: &str, game: Option<&str>) -> String {
    let now = Local::now();
    template
        .replace("{game}", game.unwrap_or("Capcove"))
        .replace("{date}", &now.format("%Y-%m-%d").to_string())
        .replace("{time}", &now.format("%H:%M").to_string())
        .replace("{datetime}", &now.format("%Y-%m-%d %H:%M").to_string())
}

/// The more restrictive of two resolution caps — applies the YouTube ceiling
/// without upscaling past the local recording's own cap.
fn tighter_resolution(a: crate::config::RecordingResolution, b: crate::config::RecordingResolution) -> crate::config::RecordingResolution {
    match (a.height(), b.height()) {
        (None, None) => a,
        (None, Some(_)) => b,
        (Some(_), None) => a,
        (Some(ha), Some(hb)) => if ha <= hb { a } else { b },
    }
}

/// Creates a YouTube live broadcast for a recording about to start; needs a
/// connected Google account. Returns `None` on any failure (after notifying
/// the user) so the caller falls back to a normal local-only recording.
pub(crate) async fn try_start_live_broadcast(app: &AppHandle, game: Option<&str>) -> Option<crate::drive::youtube::LiveBroadcast> {
    let drive = app.state::<Arc<crate::drive::DriveClient>>();
    if !drive.is_connected() {
        crate::notify_error(app, "YouTube live needs a connected Google account (Settings → Google Drive)");
        return None;
    }

    let settings = app.state::<Arc<ConfigStore>>().get();
    let title = render_title_template(&settings.video.youtube_live.title_template, game);
    let privacy = settings.video.youtube_live.privacy.clone();
    let cid = settings.effective_google_client_id().to_string();
    let csec = settings.effective_google_client_secret().to_string();
    match drive.create_live_broadcast(&cid, &csec, &title, &privacy, settings.youtube_stream_id.as_deref()).await {
        Ok(live) => {
            log::info!("youtube live: broadcast {} created ('{title}', {privacy}, stream {})", live.broadcast_id, live.stream_id);
            // Remember the stream key so every later session reuses it.
            if settings.youtube_stream_id.as_deref() != Some(live.stream_id.as_str()) {
                let store = app.state::<Arc<ConfigStore>>();
                let mut fresh = store.get();
                fresh.youtube_stream_id = Some(live.stream_id.clone());
                if let Err(e) = store.save(fresh) {
                    log::warn!("youtube live: failed to persist stream id: {e}");
                }
            }
            // Broadcasts deleted on YouTube leave dead links — drop their gallery entries.
            if !live.cleaned_up.is_empty() {
                let meta = app.state::<Arc<crate::meta::MetaStore>>();
                for id in &live.cleaned_up {
                    meta.remove(&format!("yt_{id}"));
                }
                let _ = app.emit("video-saved", serde_json::json!({ "name": "" }));
            }
            crate::toast::show(app, "info", crate::toast::ToastCategory::Stream, "Stream started",
                &format!("Streaming to YouTube ({privacy}): https://youtube.com/watch?v={}", live.broadcast_id));
            Some(live)
        }
        Err(e) => {
            log::warn!("youtube live: broadcast creation failed: {e}");
            crate::notify_error(app, "YouTube live could not start — recording locally only");
            None
        }
    }
}

/// Starts one configured audio source: a local TCP relay ffmpeg connects to
/// as a network input, fed by the matching WASAPI capture thread.
fn start_audio_source(
    source: &AudioSource,
    // While set, captured bytes are dropped, shortening the track in step
    // with the video frames the writer skips.
    capture_paused: Arc<std::sync::atomic::AtomicBool>,
    // Recorded game's pid when a dedicated Game track exists — default-endpoint
    // system sources switch to exclude-mode loopback to avoid double capture.
    exclude_game_pid: Option<u32>,
) -> Result<(AudioStreamSpec, RelayedAudioCapture), String> {
    // Per-app sources capture via process loopback instead of a device.
    if let AudioSource::Application { exe, label, .. } = source {
        let pid = audio_capture::find_session_pid(exe)
            .ok_or_else(|| format!("{exe} has no active audio session (not running?)"))?;
        let label = if label.is_empty() { exe.clone() } else { label.clone() };
        let main_mix = source.in_main_mix();
        return start_relayed(label, main_mix, capture_paused, move |on_data| audio_capture::start_process_capture(pid, on_data));
    }

    let (device_id, flow, label) = match source {
        AudioSource::SystemOutput { device_id, label, .. } => (
            device_id.clone(),
            AudioFlow::Render,
            if label.is_empty() { "System Audio".to_string() } else { label.clone() },
        ),
        AudioSource::Microphone { device_id, label, .. } => (
            device_id.clone(),
            AudioFlow::Capture,
            if label.is_empty() { "Microphone".to_string() } else { label.clone() },
        ),
        AudioSource::Application { .. } => unreachable!("handled above"),
    };

    // Exclude-mode loopback is pinned to the default endpoint by the API —
    // only swap it in when this source is actually the default device.
    if let (Some(game_pid), AudioFlow::Render) = (exclude_game_pid, flow) {
        let is_default = audio_capture::default_render_device_id()
            .map(|d| d == device_id)
            .unwrap_or(false);
        if is_default {
            return start_relayed(label, source.in_main_mix(), capture_paused, move |on_data| {
                audio_capture::start_process_exclude_capture(game_pid, on_data)
            });
        }
        log::info!("game track: System Audio uses a non-default device — exclude-mode unavailable, game audio may appear on both tracks");
    }

    start_relayed(label, source.in_main_mix(), capture_paused, move |on_data| audio_capture::start_capture(device_id, flow, on_data))
}

/// Starts every configured, un-muted audio source; failures are logged and
/// skipped. `game_pid` adds a dedicated "Game" track and makes default-device
/// System Audio exclude that process so game sound lands on exactly one track.
pub(crate) fn start_configured_audio_sources(
    audio: &crate::config::AudioConfig,
    capture_paused: Arc<std::sync::atomic::AtomicBool>,
    game_pid: Option<u32>,
    force_own_audio: bool,
) -> (Vec<AudioStreamSpec>, Vec<RelayedAudioCapture>) {
    let separate = audio.separate_tracks;
    let game_session = game_pid.is_some();
    // "Game audio only": the dedicated Game track replaces System Audio in
    // the mix. Separate-tracks mode only demotes System Audio out of the mix;
    // single-track mode drops it entirely to avoid doubling the game.
    let game_only = audio.game_audio_only && game_session;
    // The Game capture runs in separate mode and in game-only single-track
    // mode; plain single-track mode keeps the game inside system loopback.
    // `force_own_audio` (an explicitly user-picked recording target, not an
    // auto-detected game) always wants it, ignoring both settings.
    let want_game_track = game_session && (separate || game_only || force_own_audio);
    log::info!(
        "audio setup: game_pid={game_pid:?} separate={separate} game_audio_only={} force_own_audio={force_own_audio} want_game_track={want_game_track}",
        audio.game_audio_only
    );
    // Harmless when it can't apply (non-default device) — see the fallback
    // in `start_audio_source`.
    let exclude_pid = if separate { game_pid } else { None };

    // Input-vs-game mix weighting (single-track mode, game sessions only).
    let weight_for = |is_game_side: bool| -> f32 {
        match (game_session && !separate, audio.mix_priority) {
            (true, crate::config::MixPriority::Input) => if is_game_side { 0.45 } else { 1.0 },
            (true, crate::config::MixPriority::Game) => if is_game_side { 1.0 } else { 0.45 },
            _ => 1.0,
        }
    };

    let sources: Vec<AudioSource> = audio.sources.iter()
        .filter(|s| s.is_enabled())
        .filter(|s| match s {
            // game_only drops System Audio in single-track mode; separate
            // mode keeps it as its own track.
            AudioSource::SystemOutput { .. } => !audio.system_muted && (separate || !game_only),
            AudioSource::Microphone { .. } => !audio.mic_muted,
            // Per-app tracks aren't covered by the quick mutes.
            AudioSource::Application { .. } => true,
        })
        .cloned()
        .collect();
    let mut audio_specs = Vec::new();
    let mut audio_handles = Vec::new();

    if want_game_track {
        let pid = game_pid.unwrap();
        match start_relayed("Game".to_string(), audio.game_track_main_mix, capture_paused.clone(), move |on_data| {
            audio_capture::start_process_capture(pid, on_data)
        }) {
            Ok((mut spec, handle)) => {
                log::info!("game audio track started for pid {pid}");
                if !separate {
                    spec.mix_only = true;
                    spec.weight = weight_for(true);
                }
                audio_specs.push(spec);
                audio_handles.push(handle);
            }
            Err(e) => log::warn!("failed to start the game audio track (pid {pid}): {e}"),
        }
    }

    for source in &sources {
        match start_audio_source(source, capture_paused.clone(), exclude_pid) {
            Ok((mut spec, handle)) => {
                if !separate {
                    spec.mix_only = true;
                    // Without the game split, "game side" covers everything
                    // that isn't the microphone.
                    let is_mic = matches!(source, AudioSource::Microphone { .. });
                    spec.weight = weight_for(!is_mic);
                } else if game_only && matches!(source, AudioSource::SystemOutput { .. }) {
                    // The Game track already fills System Audio's spot in the
                    // mix — keep this one out to avoid doubling the game.
                    spec.main_mix = false;
                }
                audio_specs.push(spec);
                audio_handles.push(handle);
            }
            Err(e) => log::warn!("failed to start audio source {source:?}: {e}"),
        }
    }
    (audio_specs, audio_handles)
}

/// One connected ffmpeg client's audio feeder — the relay counterpart of
/// `JobFeeder`. Each connection gets its own thread and bounded queue so a
/// slow or stalled connection never blocks audio delivery to the others.
struct AudioRelayConn {
    tx: Option<std::sync::mpsc::SyncSender<Arc<[u8]>>>,
    thread: Option<JoinHandle<()>>,
}

impl AudioRelayConn {
    fn spawn(mut stream: std::net::TcpStream) -> Self {
        // Audio chunks are tiny and frequent — a deeper queue than the video
        // feeder's absorbs hiccups without meaningful buffered latency.
        let (tx, rx) = std::sync::mpsc::sync_channel::<Arc<[u8]>>(16);
        let thread = std::thread::spawn(move || {
            while let Ok(bytes) = rx.recv() {
                if stream.write_all(&bytes).is_err() {
                    break;
                }
            }
            // Dropping `stream` closes the socket so ffmpeg sees EOF on this input.
        });
        Self { tx: Some(tx), thread: Some(thread) }
    }

    /// Hands a chunk to this connection's thread. Returns `false` once that
    /// thread has exited so the caller can drop the connection.
    fn send(&self, bytes: Arc<[u8]>) -> bool {
        match &self.tx {
            Some(tx) => !matches!(tx.try_send(bytes), Err(std::sync::mpsc::TrySendError::Disconnected(_))),
            None => false,
        }
    }

    fn disconnect(&mut self) {
        self.tx = None;
    }

    fn stop(mut self) {
        self.disconnect();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

pub(crate) struct RelayedAudioCapture {
    capture: AudioCaptureHandle,
    relay_streams: Arc<Mutex<Vec<AudioRelayConn>>>,
}

impl RelayedAudioCapture {
    pub(crate) fn stop(self) {
        self.capture.stop();
        // Disconnect every connection before joining any, so one connection's
        // catch-up time doesn't serialize behind another's.
        let mut conns: Vec<AudioRelayConn> = std::mem::take(&mut *self.relay_streams.lock().unwrap());
        for conn in conns.iter_mut() {
            conn.disconnect();
        }
        for conn in conns {
            conn.stop();
        }
    }
}

/// Opens the TCP relay ffmpeg reads this audio track from, then starts the
/// capture. The relay fans chunks out to all connected clients independently.
fn start_relayed(
    label: String,
    main_mix: bool,
    capture_paused: Arc<std::sync::atomic::AtomicBool>,
    start_fn: impl FnOnce(Box<dyn FnMut(&[u8]) + Send + 'static>) -> Result<(AudioCaptureHandle, audio_capture::AudioFormat), String>,
) -> Result<(AudioStreamSpec, RelayedAudioCapture), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let relay_streams: Arc<Mutex<Vec<AudioRelayConn>>> = Arc::new(Mutex::new(Vec::new()));
    let relay_accept = relay_streams.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            relay_accept.lock().unwrap().push(AudioRelayConn::spawn(stream));
        }
    });

    let relay_write = relay_streams.clone();
    let (capture, fmt) = start_fn(Box::new(move |bytes: &[u8]| {
        if capture_paused.load(std::sync::atomic::Ordering::Relaxed) {
            return; // paused capture: video frames are skipped too
        }
        // Bursts arriving before a connection exists (or has caught up) are
        // dropped, not queued; `retain` doubles as dead-connection cleanup.
        let shared: Arc<[u8]> = Arc::from(bytes);
        let mut streams = relay_write.lock().unwrap();
        streams.retain(|s| s.send(shared.clone()));
    }))?;

    Ok((
        AudioStreamSpec { port, sample_rate: fmt.sample_rate, channels: fmt.channels, sample_fmt: fmt.ffmpeg_sample_fmt(), label, main_mix, mix_only: false, weight: 1.0 },
        RelayedAudioCapture { capture, relay_streams },
    ))
}

use encoder::JobFeeder;

/// Spawns the writer thread owning the local/live ffmpeg children: pulls
/// frames from `mailbox`, lazily spawns outputs once dimensions are known,
/// and finishes remaining jobs gracefully when the mailbox closes.
#[allow(clippy::too_many_arguments)]
fn spawn_writer_thread(
    app: AppHandle,
    mailbox: Arc<FrameMailbox>,
    fps: u32,
    encoder: EncoderChoice,
    bitrate_kbps: u32,
    rate_control: crate::config::RateControl,
    quality: u32,
    audio_specs: Vec<AudioStreamSpec>,
    audio_codec: crate::config::AudioCodec,
    resolution: crate::config::RecordingResolution,
    container: crate::config::VideoContainer,
    initial_live: Option<encoder::LiveStreamParams>,
    initial_local_path: Option<PathBuf>,
    // Recorded window, when the target is a window — distinguishes
    // "minimized" from "content just isn't changing".
    window_hwnd: Option<u32>,
    // Privacy toggle: write an "Alt-tabbed" card while the window isn't foreground.
    alt_tab_privacy: bool,
    // What to write while the window is minimized (card / black / pause).
    minimized_behavior: crate::config::MinimizedBehavior,
    // Flipped while paused so the audio callbacks drop their bytes in step.
    capture_paused: Arc<std::sync::atomic::AtomicBool>,
    // The user's explicit pause toggle.
    manual_paused: Arc<std::sync::atomic::AtomicBool>,
    control_rx: std::sync::mpsc::Receiver<WriterControl>,
    session_id: String,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        use capture_session::PopResult;
        use std::sync::atomic::Ordering;
        let frame_interval = std::time::Duration::from_millis(1000 / fps.max(1) as u64);
        let mut local_job: Option<JobFeeder> = None;
        let mut live_job: Option<JobFeeder> = None;
        // Wanted but not yet spawned — taken the instant `dims` is known.
        let mut pending_local: Option<PathBuf> = initial_local_path;
        let mut pending_live: Option<encoder::LiveStreamParams> = initial_live;
        let mut dims: Option<(u32, u32)> = None;
        let mut last_frame: Option<Arc<[u8]>> = None; // most recent real frame
        let mut warned_tiny = false;
        let mut tiny_since: Option<std::time::Instant> = None;
        // Freeze mode: swallows the black frames delivered right after a
        // minimized window is restored (see the grace logic below).
        let mut was_minimized = false;
        let mut restore_grace: Option<std::time::Instant> = None;
        let mut cards: std::collections::HashMap<&'static str, Arc<[u8]>> = std::collections::HashMap::new();
        // Periodic snapshot for the gallery's in-progress card; `None`
        // forces one on the first eligible frame.
        let mut last_snapshot: Option<std::time::Instant> = None;
        const SNAPSHOT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3);
        loop {
            // Apply pending on/off toggles before this tick's frame handling;
            // starting or stopping one job never touches the other or the capture.
            while let Ok(msg) = control_rx.try_recv() {
                match msg {
                    WriterControl::EnableLocal(path) => pending_local = Some(path),
                    WriterControl::DisableLocal => {
                        // Off its own thread — `.stop()` can now block a
                        // while (see `EncodeJob::finish`), and blocking this
                        // loop would stall the other job too.
                        if let Some(j) = local_job.take() {
                            std::thread::spawn(move || j.stop());
                        }
                        pending_local = None;
                    }
                    WriterControl::EnableLive(params) => pending_live = Some(*params),
                    WriterControl::DisableLive => {
                        if let Some(j) = live_job.take() {
                            std::thread::spawn(move || j.stop());
                        }
                        pending_live = None;
                    }
                }
            }
            if let Some((w, h)) = dims {
                if let Some(path) = pending_local.take() {
                    match encoder::spawn_local(&app, w, h, fps, &encoder, bitrate_kbps, rate_control, quality, &audio_specs, &audio_codec, &resolution, &container, &path) {
                        Ok((new_job, rx)) => {
                            write_active_recording_marker(&app, &path, &container);
                            spawn_local_finish_watcher(app.clone(), rx, path, session_id.clone());
                            local_job = Some(JobFeeder::spawn(new_job, "local recording"));
                        }
                        Err(e) => {
                            log::error!("failed to start local recording: {e}");
                            crate::notify_error(&app, &format!("Recording failed to start: {e}"));
                        }
                    }
                }
                if let Some(params) = pending_live.take() {
                    match encoder::spawn_live(&app, w, h, fps, &encoder, &audio_specs, &params) {
                        Ok((new_job, rx)) => {
                            spawn_live_finish_watcher(app.clone(), rx, session_id.clone());
                            live_job = Some(JobFeeder::spawn(new_job, "live stream"));
                        }
                        Err(e) => {
                            log::error!("failed to start live stream: {e}");
                            crate::notify_error(&app, &format!("Streaming failed to start: {e}"));
                        }
                    }
                }
            }

            // Capture only delivers frames when content changes, so a timeout
            // is not proof of minimization — repeat the last real frame to
            // keep timeline/audio in sync; cards key off the window's real state.
            match mailbox.pop_timeout(frame_interval) {
                PopResult::Closed => break,
                PopResult::Frame(msg) => {
                    // Encoder dimensions lock in with the first frame — skip
                    // launcher-splash-sized frames hardware encoders would
                    // reject, until a plausible one arrives.
                    if dims.is_none() && (msg.width < MIN_CAPTURE_W || msg.height < MIN_CAPTURE_H) {
                        // Some real windows (e.g. a compact media player) never grow
                        // past this "splash screen" threshold — after a grace period,
                        // accept the size we have rather than waiting forever, as long
                        // as it clears the hardware encoders' actual minimum.
                        let past_grace = tiny_since.is_some_and(|t| t.elapsed() >= TINY_FRAME_GRACE);
                        let above_hard_floor = msg.width >= HARD_MIN_CAPTURE_W && msg.height >= HARD_MIN_CAPTURE_H;
                        if !(past_grace && above_hard_floor) {
                            tiny_since.get_or_insert_with(std::time::Instant::now);
                            if !warned_tiny {
                                warned_tiny = true;
                                log::warn!("capture frames are {}x{} (below {MIN_CAPTURE_W}x{MIN_CAPTURE_H}) — waiting for a real-sized frame before starting the encoder", msg.width, msg.height);
                            }
                            continue;
                        }
                        log::info!("capture frames stayed {}x{} after the grace period — starting the encoder at that size", msg.width, msg.height);
                    }
                    if dims.is_none() {
                        // The even size ffmpeg is told via `-s`; frames and
                        // placeholder cards are cropped/rendered to match.
                        dims = Some(encoder::even_dims(msg.width, msg.height));
                        let (w, h) = dims.unwrap();
                        if let Some(path) = pending_local.take() {
                            match encoder::spawn_local(&app, w, h, fps, &encoder, bitrate_kbps, rate_control, quality, &audio_specs, &audio_codec, &resolution, &container, &path) {
                                Ok((new_job, rx)) => {
                                    write_active_recording_marker(&app, &path, &container);
                                    spawn_local_finish_watcher(app.clone(), rx, path, session_id.clone());
                                    local_job = Some(JobFeeder::spawn(new_job, "local recording"));
                                }
                                Err(e) => {
                                    log::error!("failed to start local recording: {e}");
                                    crate::notify_error(&app, &format!("Recording failed to start: {e}"));
                                }
                            }
                        }
                        if let Some(params) = pending_live.take() {
                            match encoder::spawn_live(&app, w, h, fps, &encoder, &audio_specs, &params) {
                                Ok((new_job, rx)) => {
                                    spawn_live_finish_watcher(app.clone(), rx, session_id.clone());
                                    live_job = Some(JobFeeder::spawn(new_job, "live stream"));
                                }
                                Err(e) => {
                                    log::error!("failed to start live stream: {e}");
                                    crate::notify_error(&app, &format!("Streaming failed to start: {e}"));
                                }
                            }
                        }
                    }
                    // Freeze mode, right after restore: the first frames are
                    // black until the desktop recomposites — keep the frozen
                    // picture until a real frame arrives (bounded by the grace window).
                    let in_grace = minimized_behavior == crate::config::MinimizedBehavior::Freeze
                        && (was_minimized || restore_grace.map(|u| std::time::Instant::now() < u).unwrap_or(false));
                    if in_grace && placeholder::is_mostly_black(&msg.bgra) {
                        // swallow it — last_frame stays frozen
                    } else {
                        restore_grace = None;
                        // Scaled/cropped to the size the encoder started
                        // with — `msg.width/height` can differ from `dims`
                        // if the recorded window's been resized since (see
                        // `encoder::fit_frame_to_dims`).
                        let (dw, dh) = dims.unwrap();
                        last_frame = Some(Arc::from(encoder::fit_frame_to_dims(&msg.bgra, msg.width, msg.height, dw, dh).into_owned()));
                    }
                }
                PopResult::TimedOut => {}
            }

            let occlusion = placeholder::window_occlusion(window_hwnd, alt_tab_privacy);
            if minimized_behavior == crate::config::MinimizedBehavior::Freeze {
                if was_minimized && occlusion != placeholder::Occlusion::Minimized {
                    restore_grace = Some(std::time::Instant::now() + std::time::Duration::from_millis(1000));
                }
                was_minimized = occlusion == placeholder::Occlusion::Minimized;
            }
            let occlusion_paused = occlusion == placeholder::Occlusion::Minimized
                && minimized_behavior == crate::config::MinimizedBehavior::Pause;
            // Paused: skip video and signal the audio callbacks to drop
            // their bytes so all streams stay in lockstep.
            let paused = occlusion_paused || manual_paused.load(Ordering::Relaxed);
            capture_paused.store(paused, Ordering::Relaxed);
            if paused {
                continue;
            }

            let card_text = placeholder::card_for(occlusion, minimized_behavior);
            let bytes: Option<Arc<[u8]>> = match (card_text, dims) {
                (Some(text), Some((w, h))) => Some(
                    cards.entry(text).or_insert_with(|| Arc::from(placeholder::render(w, h, text))).clone(),
                ),
                _ => last_frame.clone(),
            };

            // Periodic snapshot for the gallery's in-progress card — only
            // while a local file exists, and off-thread so the JPEG encode
            // never delays frame feeding.
            if local_job.is_some() {
                if let (Some(b), Some((w, h))) = (&bytes, dims) {
                    if last_snapshot.map(|t| t.elapsed() >= SNAPSHOT_INTERVAL).unwrap_or(true) {
                        last_snapshot = Some(std::time::Instant::now());
                        spawn_live_thumbnail_snapshot(app.clone(), b.clone(), w, h);
                    }
                }
            }

            // Each job has its own feeder — one falling behind or dying never
            // blocks the other, or this loop's ability to notice a Stop.
            if let Some(bytes) = bytes {
                if let Some(j) = local_job.as_ref() {
                    if !j.send(bytes.clone()) {
                        local_job = None;
                    }
                }
                if let Some(j) = live_job.as_ref() {
                    if !j.send(bytes) {
                        live_job = None;
                    }
                }
            }
        }
        // Disconnect both first, then stop both concurrently so neither's
        // (now potentially slow) stop serializes behind the other's.
        if let Some(j) = local_job.as_mut() {
            j.disconnect();
        }
        if let Some(j) = live_job.as_mut() {
            j.disconnect();
        }
        let local_stop = local_job.take().map(|j| std::thread::spawn(move || j.stop()));
        let live_stop = live_job.take().map(|j| std::thread::spawn(move || j.stop()));
        if let Some(t) = local_stop {
            let _ = t.join();
        }
        if let Some(t) = live_stop {
            let _ = t.join();
        }
    })
}

/// Best-effort thumbnail for the gallery's in-progress card, encoded off-thread
/// from the raw frame already in hand (the output file isn't safe to read mid-mux).
fn spawn_live_thumbnail_snapshot(app: AppHandle, bgra: Arc<[u8]>, width: u32, height: u32) {
    std::thread::spawn(move || {
        let Some(jpeg) = bgra_to_jpeg(&bgra, width, height) else { return };
        use base64::Engine;
        let data_url = format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(jpeg));
        let _ = app.emit("recording-thumbnail", serde_json::json!({ "data_url": data_url }));
    });
}

/// BGRA raw bytes to a downscaled JPEG — same ~320px-wide convention as the
/// gallery's cached thumbnails.
fn bgra_to_jpeg(bgra: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    if width == 0 || height == 0 {
        return None;
    }
    let mut rgb = vec![0u8; (width as usize) * (height as usize) * 3];
    for (src, dst) in bgra.chunks_exact(4).zip(rgb.chunks_exact_mut(3)) {
        dst[0] = src[2];
        dst[1] = src[1];
        dst[2] = src[0];
    }
    let img = image::RgbImage::from_raw(width, height, rgb)?;
    let target_w = width.min(320).max(1);
    let target_h = (((height as u64) * (target_w as u64)) / (width.max(1) as u64)).max(1) as u32;
    let scaled = image::imageops::resize(&img, target_w, target_h, image::imageops::FilterType::Triangle);
    let mut out = Vec::new();
    scaled.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Jpeg).ok()?;
    Some(out)
}

/// Stops the session only if it's still the same one this job belonged to.
/// Returns whether it was — callers use that to tell a genuine crash apart
/// from ffmpeg exiting as part of a stop the app already initiated.
async fn stop_if_still_current(app: &AppHandle, session_id: &str) -> bool {
    let current = app.state::<Arc<RecordingManager>>().current_session();
    let still_current = current.map(|s| s.id) == Some(session_id.to_string());
    if still_current {
        log::warn!("ffmpeg exited unexpectedly while its session was still active — stopping the session");
        let app2 = app.clone();
        let _ = tauri::async_runtime::spawn_blocking(move || stop_recording(&app2)).await;
    }
    still_current
}

/// Live-specific version of `stop_if_still_current`: the session can stay
/// current (local recording keeps going) after `live` is intentionally
/// turned off, so this checks `session.live` itself, not just the session id.
async fn stop_if_live_still_expected(app: &AppHandle, session_id: &str) -> bool {
    let current = app.state::<Arc<RecordingManager>>().current_session();
    let live_still_expected = current.as_ref().is_some_and(|s| s.id == session_id && s.live);
    if live_still_expected {
        log::warn!("ffmpeg[live] exited unexpectedly while streaming was still active — stopping the session");
        let app2 = app.clone();
        let _ = tauri::async_runtime::spawn_blocking(move || stop_recording(&app2)).await;
    }
    live_still_expected
}

/// One completed ffmpeg `-progress` report: the windowed bitrate (`None` for
/// the very first report) and the file's cumulative size so far.
struct ProgressReport {
    kbps: Option<f64>,
    total_bytes: u64,
}

/// Parses ffmpeg `-progress` reports into a windowed bitrate, since ffmpeg's
/// own `bitrate=` field is a cumulative average too slow to reflect current health.
struct ProgressRateTracker {
    buf: String,
    cur_size: Option<u64>,
    cur_time_us: Option<i64>,
    prev_size: Option<u64>,
    prev_time_us: Option<i64>,
}

impl ProgressRateTracker {
    fn new() -> Self {
        Self { buf: String::new(), cur_size: None, cur_time_us: None, prev_size: None, prev_time_us: None }
    }

    /// Feeds a raw stdout chunk; returns every report completed by it.
    fn feed(&mut self, bytes: &[u8]) -> Vec<ProgressReport> {
        self.buf.push_str(&String::from_utf8_lossy(bytes));
        let mut out = Vec::new();
        while let Some(pos) = self.buf.find('\n') {
            let line: String = self.buf.drain(..=pos).collect();
            let line = line.trim();
            if let Some(v) = line.strip_prefix("total_size=") {
                self.cur_size = v.trim().parse().ok();
            } else if let Some(v) = line.strip_prefix("out_time_us=") {
                self.cur_time_us = v.trim().parse().ok();
            } else if line.starts_with("progress=") {
                // End of this report's key=value block.
                if let Some(size) = self.cur_size {
                    let kbps = if let (Some(t_us), Some(psize), Some(pt_us)) =
                        (self.cur_time_us, self.prev_size, self.prev_time_us)
                    {
                        let dt_secs = (t_us - pt_us) as f64 / 1_000_000.0;
                        (dt_secs > 0.05).then(|| (size.saturating_sub(psize)) as f64 * 8.0 / 1000.0 / dt_secs)
                    } else {
                        None
                    };
                    out.push(ProgressReport { kbps, total_bytes: size });
                }
                self.prev_size = self.cur_size;
                self.prev_time_us = self.cur_time_us;
            }
        }
        out
    }
}

fn active_recording_marker_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("active_recording.json"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ActiveRecordingMarker {
    path: PathBuf,
    container: crate::config::VideoContainer,
}

/// Written when a local recording starts, removed once its encoder is seen
/// stopping. A marker still present at the next startup means the app was
/// killed or crashed mid-write — `check_crash_recovery` surfaces the leftover.
fn write_active_recording_marker(app: &AppHandle, path: &std::path::Path, container: &crate::config::VideoContainer) {
    let Some(marker_path) = active_recording_marker_path(app) else { return };
    let marker = ActiveRecordingMarker { path: path.to_path_buf(), container: container.clone() };
    let Ok(json) = serde_json::to_vec(&marker) else { return };
    if let Some(parent) = marker_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&marker_path, json);
}

/// Only clears the marker if it still points at `path` — an old job's finish
/// watcher must never remove the marker a newer job wrote for its own file.
fn clear_active_recording_marker_if_matches(app: &AppHandle, path: &std::path::Path) {
    let Some(marker_path) = active_recording_marker_path(app) else { return };
    let Ok(bytes) = std::fs::read(&marker_path) else { return };
    let Ok(marker) = serde_json::from_slice::<ActiveRecordingMarker>(&bytes) else { return };
    if marker.path == path {
        let _ = std::fs::remove_file(&marker_path);
    }
}

/// Checked once at startup. A leftover marker means the previous run crashed
/// mid-write. Crash-safe containers are surfaced as-is; a plain MP4/MOV has
/// no index yet, so a best-effort ffmpeg repair is attempted instead.
pub fn check_crash_recovery(app: &AppHandle) {
    let Some(marker_path) = active_recording_marker_path(app) else { return };
    let Ok(bytes) = std::fs::read(&marker_path) else { return }; // no marker = last shutdown was clean
    let _ = std::fs::remove_file(&marker_path); // never retry, regardless of the outcome below
    let Ok(marker) = serde_json::from_slice::<ActiveRecordingMarker>(&bytes) else { return };
    if !marker.path.exists() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        recover_interrupted_recording(&app, marker).await;
    });
}

/// Holds the most recent `check_crash_recovery` outcome so the gallery can
/// still pull it if it wasn't mounted (listening) when the event fired.
#[derive(Default)]
pub struct CrashRecoveryState(Mutex<Option<serde_json::Value>>);

impl CrashRecoveryState {
    pub fn take(&self) -> Option<serde_json::Value> {
        self.0.lock().unwrap().take()
    }
}

/// `outcome`: "durable", "repaired", or "failed". `path` is absolute so the
/// gallery can show the recovered video directly in its modal.
fn report_crash_recovery(app: &AppHandle, name: &str, path: &std::path::Path, outcome: &str) {
    let payload = serde_json::json!({ "name": name, "path": path.to_string_lossy(), "outcome": outcome });
    if let Some(state) = app.try_state::<Arc<CrashRecoveryState>>() {
        *state.0.lock().unwrap() = Some(payload.clone());
    }
    let _ = app.emit("crash-recovery", payload);
}

async fn recover_interrupted_recording(app: &AppHandle, marker: ActiveRecordingMarker) {
    use crate::config::VideoContainer;
    let name = relative_video_name(app, &marker.path);

    if matches!(marker.container, VideoContainer::Mkv | VideoContainer::Mp4Fragmented | VideoContainer::MovFragmented) {
        log::info!("crash recovery: '{name}' is in a crash-safe container, nothing to repair");
        let _ = app.emit("video-saved", serde_json::json!({ "name": name }));
        report_crash_recovery(app, &name, &marker.path, "durable");
        return;
    }

    log::info!("crash recovery: attempting to repair '{name}' (plain {:?})", marker.container);
    let repaired_path = marker.path.with_extension(format!("recovered.{}", marker.container.extension()));
    let Ok(sidecar) = crate::integrity::ffmpeg_sidecar(app) else {
        report_crash_recovery(app, &name, &marker.path, "failed");
        return;
    };
    let result = sidecar
        .args([
            "-y", "-err_detect", "ignore_err",
            "-i", &marker.path.to_string_lossy(),
            "-map", "0", "-c", "copy",
            &repaired_path.to_string_lossy(),
        ])
        .output()
        .await;
    let repaired_ok = result.as_ref().map(|o| o.status.success()).unwrap_or(false)
        && std::fs::metadata(&repaired_path).map(|m| m.len() > 0).unwrap_or(false);

    if repaired_ok && std::fs::remove_file(&marker.path).is_ok() && std::fs::rename(&repaired_path, &marker.path).is_ok() {
        log::info!("crash recovery: repaired '{name}'");
        let _ = app.emit("video-saved", serde_json::json!({ "name": name }));
        report_crash_recovery(app, &name, &marker.path, "repaired");
        return;
    }
    let _ = std::fs::remove_file(&repaired_path);
    log::warn!("crash recovery: could not repair '{name}'");
    let _ = app.emit("video-saved", serde_json::json!({ "name": name }));
    report_crash_recovery(app, &name, &marker.path, "failed");
}

/// `EncodeJob::finish()` only closes ffmpeg's stdin; draining the event
/// channel to closed is the real "output file is complete" signal, so this
/// watcher — not the writer thread's join — decides when to show the file.
fn spawn_local_finish_watcher(app: AppHandle, mut rx: tauri::async_runtime::Receiver<tauri_plugin_shell::process::CommandEvent>, output_path: PathBuf, session_id: String) {
    use tauri_plugin_shell::process::CommandEvent;
    tauri::async_runtime::spawn(async move {
        log::info!("ffmpeg finish watcher started for {}", output_path.display());
        let mut exit_code = None;
        // Derives the recent measured output rate for the UI's live bitrate.
        let mut tracker = ProgressRateTracker::new();
        while let Some(event) = rx.recv().await {
            match event {
                CommandEvent::Stderr(bytes) => log::warn!("ffmpeg[{}]: {}", output_path.display(), String::from_utf8_lossy(&bytes).trim_end()),
                CommandEvent::Stdout(bytes) => {
                    for report in tracker.feed(&bytes) {
                        let _ = app.emit("recording-stats", serde_json::json!({
                            "bitrate_kbps": report.kbps,
                            "total_bytes": report.total_bytes,
                        }));
                    }
                }
                CommandEvent::Error(e) => log::error!("ffmpeg[{}] error: {e}", output_path.display()),
                CommandEvent::Terminated(payload) => {
                    exit_code = payload.code;
                    log::info!("ffmpeg[{}] terminated, exit code {:?}", output_path.display(), payload.code);
                }
                _ => {}
            }
        }
        log::info!("ffmpeg finish watcher: channel closed for {} (exit code {:?})", output_path.display(), exit_code);
        // The encoder was seen stopping — clean or not, crash recovery has
        // nothing left to do for this file.
        clear_active_recording_marker_if_matches(&app, &output_path);
        let file_name = relative_video_name(&app, &output_path);

        if exit_code != Some(0) {
            // Non-zero can also mean an intentional force-killed stop, not a
            // crash — only report it if the session was genuinely still active.
            if stop_if_still_current(&app, &session_id).await {
                crate::notify_error(&app, "Recording failed — the encoder exited with an error");
            }
            // Drop a zero-byte leftover and its orphaned metadata entry.
            if std::fs::metadata(&output_path).map(|m| m.len() == 0).unwrap_or(false) {
                let _ = std::fs::remove_file(&output_path);
            }
            if !output_path.exists() && !file_name.is_empty() {
                app.state::<Arc<crate::meta::MetaStore>>().remove(&file_name);
            }
            let _ = app.emit("video-saved", serde_json::json!({ "name": file_name }));
            return;
        }

        let _ = app.emit("video-saved", serde_json::json!({
            "path": output_path.to_string_lossy(),
            "name": file_name,
        }));
    });
}

/// Live-feed counterpart of `spawn_local_finish_watcher` — no output file to
/// clean up, just unexpected exits to surface. Also emits `live-stats`, the
/// measured bitrate the stream-health UI compares against the target.
fn spawn_live_finish_watcher(app: AppHandle, mut rx: tauri::async_runtime::Receiver<tauri_plugin_shell::process::CommandEvent>, session_id: String) {
    use tauri_plugin_shell::process::CommandEvent;
    tauri::async_runtime::spawn(async move {
        let mut exit_code = None;
        let mut tracker = ProgressRateTracker::new();
        while let Some(event) = rx.recv().await {
            match event {
                CommandEvent::Stderr(bytes) => log::warn!("ffmpeg[live]: {}", String::from_utf8_lossy(&bytes).trim_end()),
                CommandEvent::Stdout(bytes) => {
                    for report in tracker.feed(&bytes) {
                        let _ = app.emit("live-stats", serde_json::json!({
                            "bitrate_kbps": report.kbps,
                            "total_bytes": report.total_bytes,
                        }));
                    }
                }
                CommandEvent::Error(e) => log::error!("ffmpeg[live] error: {e}"),
                CommandEvent::Terminated(payload) => {
                    exit_code = payload.code;
                    log::info!("ffmpeg[live] terminated, exit code {:?}", payload.code);
                }
                _ => {}
            }
        }
        if exit_code != Some(0) {
            // Same reasoning — plus a plain "stop stream, keep recording"
            // toggle leaves the session current too. Only alarm if `live`
            // was still expected to be running.
            if stop_if_live_still_expected(&app, &session_id).await {
                crate::notify_error(&app, "Streaming stopped unexpectedly");
            }
        }
    });
}

/// Shared setup for every recording entry point: mutual-exclusion checks,
/// settings, encoder resolution, output path, and audio-source startup.
/// `folder_override` wins over the game's default folder if set. `force_own_audio`
/// makes `game_pid`'s dedicated track unconditional, ignoring the "Game audio
/// only" / separate-tracks settings that otherwise gate it — for an
/// explicitly user-picked recording target (recorder's Window mode), where
/// capturing that process's own audio is the whole point regardless of
/// general audio preferences, rather than the auto-detected-game case those
/// settings were designed around.
#[allow(clippy::type_complexity)]
async fn prepare(app: &AppHandle, game_name: Option<&str>, game_pid: Option<u32>, folder_override: Option<&str>, force_own_audio: bool) -> Result<(
    crate::config::VideoSettings,
    EncoderChoice,
    PathBuf,
    Arc<std::sync::atomic::AtomicBool>,
    tauri::async_runtime::JoinHandle<(Vec<AudioStreamSpec>, Vec<RelayedAudioCapture>)>,
    StartGuard,
), String> {
    let manager = app.state::<Arc<RecordingManager>>().inner().clone();
    if manager.is_recording() {
        return Err("A recording is already in progress".into());
    }
    // One-time (Windows remembers the answer) consent prompt so the OS can
    // stop drawing its capture border — see `win_util::request_borderless_capture_access`.
    static BORDERLESS_REQUESTED: std::sync::Once = std::sync::Once::new();
    BORDERLESS_REQUESTED.call_once(|| {
        let _ = tauri::async_runtime::spawn_blocking(crate::win_util::request_borderless_capture_access);
    });
    // Atomically reserves the start slot — see `RecordingManager::starting`'s
    // doc comment for why the `is_recording()` check above isn't enough on
    // its own. Released automatically (even on an early `?` return below or
    // in a caller) once `StartGuard` drops.
    if manager.starting.compare_exchange(
        false, true, std::sync::atomic::Ordering::AcqRel, std::sync::atomic::Ordering::Acquire,
    ).is_err() {
        return Err("A recording is already starting".into());
    }
    let start_guard = StartGuard(manager.clone());
    // A stop's teardown (closing capture, joining the writer thread, stopping
    // audio) runs detached so Stop itself returns instantly — but starting a
    // new session while that's still shutting down risks contending for the
    // same capture device or GPU encoder session. Wait it out first.
    // (The lock is taken in its own statement, not the `if let`'s scrutinee —
    // a temporary `MutexGuard` there would otherwise live until the end of
    // the block, across the `.await`, and `MutexGuard` isn't `Send`.)
    let pending_teardown = manager.pending_teardown.lock().unwrap().take();
    if let Some(handle) = pending_teardown {
        let _ = tauri::async_runtime::spawn_blocking(move || handle.join()).await;
    }
    // A running replay buffer stays running — each pipeline has its own
    // capture session, encoder process, and audio captures.

    let store = app.state::<Arc<ConfigStore>>();
    let settings = store.get();
    let mut video = settings.video.clone();
    let mut folder_id = folder_override.map(str::to_string);
    if let Some(name) = game_name {
        if let Some(ov) = app.state::<Arc<crate::games_db::GamesDb>>().overrides_for(name) {
            log::info!("applying per-game capture overrides for '{name}'");
            ov.apply_to(&mut video);
            folder_id = folder_id.or(ov.folder_id);
        }
    }
    let encoder = resolve_encoder(app, &video.encoder).await;
    // A detected game gets its own subdirectory; a chosen folder nests one
    // level inside it (`<Game>/<Folder>/...`), or directly under the root
    // when no game is detected.
    let mut dir = settings.resolved_recordings_dir();
    if let Some(name) = game_name {
        dir = dir.join(crate::drive::sanitize_filename(name));
        // Best-effort custom Explorer icon for the game's recordings folder;
        // cheap after the first time, so it just runs on every session start.
        let icons = app.state::<Arc<crate::icon_cache::IconCache>>();
        match crate::games_db::best_icon_bytes(app, &icons, name) {
            Some(bytes) => {
                let _ = std::fs::create_dir_all(&dir);
                crate::folder_icon::ensure_folder_icon(&dir, &bytes);
            }
            None => log::info!("folder_icon: no icon available yet for '{name}' — leaving its folder icon as default"),
        }
    }
    if let Some(folder) = folder_id.as_deref().and_then(|id| settings.folder_by_id(id)) {
        dir = dir.join(&folder.name);
    }
    let output_path = make_video_save_path(&dir, video.container.extension(), "").map_err(|e| e.to_string())?;

    // Shared pause flag: set by the writer loop while paused; audio
    // callbacks drop their bytes while it's set.
    let capture_paused = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let paused_for_audio = capture_paused.clone();
    let audio_config = video.audio.clone();
    // Spawned now but joined later, alongside the video capture start —
    // audio device negotiation and capture setup are independent costs, and
    // running them side by side roughly halves start-to-first-frame latency.
    let audio_task = tauri::async_runtime::spawn_blocking(move || {
        start_configured_audio_sources(&audio_config, paused_for_audio, game_pid, force_own_audio)
    });

    Ok((video, encoder, output_path, capture_paused, audio_task, start_guard))
}

#[allow(clippy::too_many_arguments)]
fn finish_starting(
    app: &AppHandle,
    target: RecordTarget,
    // Effective video settings (post per-game overrides) from `prepare` —
    // not re-read from config, which would lose the overrides.
    video: crate::config::VideoSettings,
    encoder: EncoderChoice,
    output_path: PathBuf,
    audio_specs: Vec<AudioStreamSpec>,
    audio_handles: Vec<RelayedAudioCapture>,
    mailbox: Arc<FrameMailbox>,
    capture_handle: CaptureHandle,
    capture_paused: Arc<std::sync::atomic::AtomicBool>,
    live: Option<crate::drive::youtube::LiveBroadcast>,
    // False for stream-only mode. `output_path` is still reserved by
    // `prepare` (cheap, no file created) so nothing below needs an
    // `Option<PathBuf>` — it just never becomes a real local job.
    local_enabled: bool,
) -> RecordingSession {
    let window_hwnd = match &target {
        RecordTarget::Window { hwnd, .. } => Some(*hwnd),
        _ => None,
    };
    let fps = video.fps;
    let alt_tab_privacy = video.replay_buffer.alt_tab_privacy;
    let minimized_behavior = video.minimized_behavior;
    let manual_paused = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let is_live = live.is_some();
    let (live_bid, live_title) = live.as_ref().map(|l| (l.broadcast_id.clone(), l.title.clone())).unzip();
    // Streaming is an additional output; the local file keeps its configured
    // quality. Only the live feed is capped against `YoutubeLiveSettings` —
    // exceeding YouTube's ingest limits makes a stream stutter or freeze.
    let live_params = live.map(|l| {
        let cap = app.state::<Arc<ConfigStore>>().get().video.youtube_live.clone();
        encoder::LiveStreamParams {
            rtmp_url: l.rtmp_url,
            bitrate_kbps: video.bitrate_kbps.min(cap.max_bitrate_kbps),
            resolution: tighter_resolution(video.resolution, cap.max_resolution),
            max_fps: fps.min(cap.max_fps),
            keyframe_interval_secs: cap.keyframe_interval_secs,
            audio_codec: cap.audio_codec,
            audio_sample_rate: cap.audio_sample_rate,
            cbr_buffer_secs: cap.cbr_buffer_secs,
        }
    });
    let session_id = uuid_v4();
    let (control_tx, control_rx) = std::sync::mpsc::channel();
    let initial_local_path = local_enabled.then(|| output_path.clone());
    let writer_thread = spawn_writer_thread(
        app.clone(), mailbox.clone(), fps, encoder.clone(), video.bitrate_kbps, video.rate_control, video.quality, audio_specs, video.audio_codec, video.resolution,
        video.container.clone(),
        live_params.clone(), initial_local_path, window_hwnd, alt_tab_privacy, minimized_behavior, capture_paused, manual_paused.clone(),
        control_rx, session_id.clone(),
    );

    let session = RecordingSession {
        id: session_id,
        target: target.clone(),
        output_path: output_path.clone(),
        started_at: chrono::Utc::now().timestamp(),
        fps,
        live: is_live,
        local: local_enabled,
        encoder: encoder.clone(),
        bitrate_kbps: video.bitrate_kbps,
        resolution: video.resolution,
        live_bitrate_kbps: live_params.as_ref().map(|lp| lp.bitrate_kbps),
        live_fps: live_params.as_ref().map(|lp| lp.max_fps),
        live_resolution: live_params.as_ref().map(|lp| lp.resolution),
        current_local_name: local_enabled.then(|| relative_video_name(app, &output_path)).filter(|n| !n.is_empty()),
        current_local_path: local_enabled.then(|| output_path.clone()),
    };

    let manager = app.state::<Arc<RecordingManager>>();
    *manager.active.lock().unwrap() = Some(ActiveRecording {
        session: session.clone(),
        capture_handle: Some(capture_handle),
        mailbox,
        writer_thread: Some(writer_thread),
        audio_handles,
        manual_paused,
        live_broadcast: Arc::new(Mutex::new(live_bid.clone())),
        local_active: Arc::new(std::sync::atomic::AtomicBool::new(local_enabled)),
        live_active: Arc::new(std::sync::atomic::AtomicBool::new(is_live)),
        local_files: Arc::new(Mutex::new(if local_enabled { vec![output_path.clone()] } else { Vec::new() })),
        control_tx,
        target,
        video,
        output_dir: output_path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| output_path.clone()),
    });

    // Per-file metadata for a Window-target session — skipped in stream-only
    // mode, where `output_path` was only reserved, never written to.
    if local_enabled {
        if let RecordTarget::Window { app: app_name, .. } = &session.target {
            let file_name = relative_video_name(app, &session.output_path);
            if !file_name.is_empty() {
                app.state::<Arc<crate::meta::MetaStore>>().set(
                    file_name,
                    crate::meta::VideoMeta {
                        // Not the window's raw title-bar text (e.g. "RAGE
                        // Multiplayer") — that's static per app, identical
                        // across every recording of it, and less useful
                        // than the gallery's own filename/timestamp fallback
                        // (see `displayTitle` in VideoGrid.jsx).
                        title: None,
                        app: Some(app_name.clone()),
                        created: Some(session.started_at),
                        kind: None,
                        stream_info: None,
                        duration_secs: None,
                        youtube_video_id: None,
                        tags: Vec::new(),
                        favorite: false,
                    },
                );
            }
        }
    }

    // When live-streaming: a virtual "YouTube Stream" entry tracks the
    // broadcast itself, alongside the local-file entry above.
    if let Some(bid) = &live_bid {
        let entry_name = format!("yt_{bid}");
        let mut stats: Vec<String> = Vec::new();
        if let Some(lp) = &live_params {
            if let Some(h) = lp.resolution.height() {
                stats.push(format!("{h}p"));
            }
            stats.push(format!("{} FPS", lp.max_fps));
            stats.push(format!("{} Mbps", lp.bitrate_kbps / 1000));
        }
        let (title_val, app_val) = match &session.target {
            RecordTarget::Window { title, app, .. } => (title.clone(), Some(app.clone())),
            RecordTarget::Monitor => ("Desktop".to_string(), None),
            RecordTarget::Area { .. } => ("Screen Area".to_string(), None),
        };
        app.state::<Arc<crate::meta::MetaStore>>().set(
            entry_name.clone(),
            crate::meta::VideoMeta {
                title: live_title.clone().or(Some(title_val)),
                app: app_val,
                created: Some(session.started_at),
                kind: Some("youtube_live".to_string()),
                stream_info: Some(stats.join(" · ")),
                duration_secs: None,
                // A broadcast id doubles as the video id once live, so the
                // gallery reads `youtube_video_id` uniformly for every kind.
                youtube_video_id: Some(bid.clone()),
                tags: Vec::new(),
                favorite: false,
            },
        );
        let _ = app.emit("video-saved", serde_json::json!({ "name": entry_name }));
    }

    // Icon caching only applies to an actually-captured window.
    if let RecordTarget::Window { hwnd, app: app_name, .. } = &session.target {
        let icon_cache = app.state::<Arc<crate::icon_cache::IconCache>>().inner().clone();
        #[cfg(windows)]
        let games = app.state::<Arc<crate::games_db::GamesDb>>().inner().clone();
        let (app_name, hwnd) = (app_name.clone(), *hwnd);
        std::thread::spawn(move || {
            icon_cache.cache_from_hwnd(&app_name, hwnd);
            // A plain window pick (not an auto-detected catalog game) has no
            // games-db entry at all, so it never gets its own overrides/toggle
            // and won't show a "Playing now" badge next time — auto-register
            // it as a custom game the first time it's recorded, same as if
            // the user had added it by hand via Settings > Games.
            #[cfg(windows)]
            if let Some(exe) = crate::capture::pid_for_hwnd(hwnd).and_then(crate::capture::exe_for_pid) {
                if !games.is_known_exe(&exe) {
                    games.add_custom(&exe, &app_name);
                }
            }
        });
    }

    hud::show(app, &session);
    start_tray_status_loop(app, session.id.clone());

    let _ = app.emit("recording-started", &session);
    notify_recording_toast(app, "Recording started", &session.target);
    crate::sound::play(&app.state::<Arc<ConfigStore>>().get().sound_effects.recording_started);
    session
}

/// A window target counts as a "session" recording of that app/game (its own
/// toast category and icon); monitor/area captures fall under the plain
/// "recording" category. Shared by started/stopped/discarded notifications.
fn notify_recording_toast(app: &AppHandle, title: &str, target: &RecordTarget) {
    match target {
        RecordTarget::Window { app: name, .. } => {
            crate::toast::show_for_game(app, "info", crate::toast::ToastCategory::Session, title, name, name);
        }
        _ => {
            crate::toast::show(app, "info", crate::toast::ToastCategory::Recording, title, "Recording the screen");
        }
    }
}

/// Swaps in the recording tray icon and keeps the tray menu's elapsed-time
/// row ticking once a second while this recording is still the active one.
fn start_tray_status_loop(app: &AppHandle, session_id: String) {
    crate::tray::set_tray_recording(app, true);
    crate::tray::refresh_tray_menu(app);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            let manager = app.state::<Arc<RecordingManager>>();
            let Some(current) = manager.current_session() else { break };
            if current.id != session_id {
                break;
            }
            crate::tray::refresh_tray_menu(&app);
        }
        crate::tray::set_tray_recording(&app, false);
        crate::tray::refresh_tray_menu(&app);
    });
}

/// Cleans up what `prepare()` already started when the capture step after it
/// fails — audio captures need an explicit `.stop()` (no `Drop` impl), and a
/// pre-created live broadcast would otherwise sit stuck as "upcoming" forever.
fn cleanup_failed_start(app: &AppHandle, audio_handles: Vec<RelayedAudioCapture>, live: Option<crate::drive::youtube::LiveBroadcast>) {
    for handle in audio_handles {
        handle.stop();
    }
    if let Some(l) = live {
        end_live_broadcast_detached(app, l.broadcast_id, false);
    }
}

pub async fn start_window_recording(
    app: &AppHandle,
    hwnd: u32,
    title: String,
    app_name: String,
) -> Result<RecordingSession, String> {
    start_window_recording_live(app, hwnd, title, app_name, None, None, true, false).await
}

/// Like `start_window_recording`, with an optional pre-created live
/// broadcast and recording-folder override. `local_enabled` is false only
/// for stream-only mode. `force_own_audio` — see `prepare`'s doc comment —
/// is true only for the recorder's explicit Window-mode pick, not the
/// auto-detected-game callers, which keep respecting the audio settings as
/// before.
pub async fn start_window_recording_live(
    app: &AppHandle,
    hwnd: u32,
    title: String,
    app_name: String,
    live: Option<crate::drive::youtube::LiveBroadcast>,
    folder_override: Option<String>,
    local_enabled: bool,
    force_own_audio: bool,
) -> Result<RecordingSession, String> {
    // `audio_pid_for_hwnd` (not the plain `pid_for_hwnd`): for a UWP/MSIX app
    // hosted by ApplicationFrameHost.exe, the window's owning process isn't
    // the one that actually renders audio — it resolves the real app's pid
    // instead so the "Game"/window audio track isn't silent for those apps.
    let window_pid = crate::capture::audio_pid_for_hwnd(hwnd);
    log::info!("start_window_recording_live: hwnd={hwnd} app_name={app_name:?} resolved_pid={window_pid:?} force_own_audio={force_own_audio}");
    let (video, encoder, output_path, capture_paused, audio_task, _start_guard) =
        prepare(app, Some(&app_name), window_pid, folder_override.as_deref(), force_own_audio).await?;
    let mailbox = FrameMailbox::new();
    let mailbox2 = mailbox.clone();
    let (fps, capture_cursor, exclude_overlay_windows, crop_titlebar) =
        (video.fps, video.capture_cursor, video.exclude_overlay_windows, video.crop_titlebar);
    let video_task = tauri::async_runtime::spawn_blocking(move || {
        capture_session::start_window_capture(hwnd, fps, capture_cursor, exclude_overlay_windows, crop_titlebar, mailbox2)
    });
    let (audio_result, video_result) = tokio::join!(audio_task, video_task);
    let (audio_specs, audio_handles) = audio_result.map_err(|e| e.to_string())?;
    let capture_handle = match video_result.map_err(|e| e.to_string()) {
        Ok(Ok(h)) => h,
        Ok(Err(e)) => { cleanup_failed_start(app, audio_handles, live); return Err(e); }
        Err(e) => { cleanup_failed_start(app, audio_handles, live); return Err(e); }
    };
    Ok(finish_starting(
        app,
        RecordTarget::Window { hwnd, title, app: app_name },
        video,
        encoder,
        output_path,
        audio_specs,
        audio_handles,
        mailbox,
        capture_handle,
        capture_paused,
        live,
        local_enabled,
    ))
}

/// Records the primary monitor (whole screen).
pub async fn start_monitor_recording(app: &AppHandle) -> Result<RecordingSession, String> {
    start_monitor_recording_live(app, None, None, true).await
}

/// Like `start_monitor_recording`, with an optional pre-created live
/// broadcast and recording-folder override. `local_enabled` false is
/// stream-only mode.
pub async fn start_monitor_recording_live(app: &AppHandle, live: Option<crate::drive::youtube::LiveBroadcast>, folder_override: Option<String>, local_enabled: bool) -> Result<RecordingSession, String> {
    let (video, encoder, output_path, capture_paused, audio_task, _start_guard) = prepare(app, None, None, folder_override.as_deref(), false).await?;
    let mailbox = FrameMailbox::new();
    let mailbox2 = mailbox.clone();
    let (fps, capture_cursor) = (video.fps, video.capture_cursor);
    let video_task = tauri::async_runtime::spawn_blocking(move || {
        capture_session::start_monitor_capture(None, fps, capture_cursor, None, mailbox2)
    });
    let (audio_result, video_result) = tokio::join!(audio_task, video_task);
    let (audio_specs, audio_handles) = audio_result.map_err(|e| e.to_string())?;
    let capture_handle = match video_result.map_err(|e| e.to_string()) {
        Ok(Ok(h)) => h,
        Ok(Err(e)) => { cleanup_failed_start(app, audio_handles, live); return Err(e); }
        Err(e) => { cleanup_failed_start(app, audio_handles, live); return Err(e); }
    };
    Ok(finish_starting(
        app,
        RecordTarget::Monitor,
        video,
        encoder,
        output_path,
        audio_specs,
        audio_handles,
        mailbox,
        capture_handle,
        capture_paused,
        live,
        local_enabled,
    ))
}

/// Records an arbitrary rectangle (absolute virtual-screen coordinates) by
/// capturing the monitor containing it and cropping every frame down to it.
pub async fn start_area_recording(app: &AppHandle, x: i32, y: i32, w: u32, h: u32) -> Result<RecordingSession, String> {
    start_area_recording_live(app, x, y, w, h, None).await
}

/// Like `start_area_recording`, with an optional pre-created live broadcast
/// (see `start_monitor_recording_live`'s doc comment).
pub async fn start_area_recording_live(app: &AppHandle, x: i32, y: i32, w: u32, h: u32, live: Option<crate::drive::youtube::LiveBroadcast>) -> Result<RecordingSession, String> {
    let (video, encoder, output_path, capture_paused, audio_task, _start_guard) = prepare(app, None, None, None, false).await?;
    let mailbox = FrameMailbox::new();
    let mailbox2 = mailbox.clone();
    let (fps, capture_cursor) = (video.fps, video.capture_cursor);
    let video_task = tauri::async_runtime::spawn_blocking(move || -> Result<CaptureHandle, String> {
        let (mon_index, mon_x, mon_y) = capture_session::monitor_index_and_origin_at(x, y)?;
        let crop = Some(((x - mon_x).max(0) as u32, (y - mon_y).max(0) as u32, w, h));
        capture_session::start_monitor_capture(Some(mon_index), fps, capture_cursor, crop, mailbox2)
    });
    let (audio_result, video_result) = tokio::join!(audio_task, video_task);
    let (audio_specs, audio_handles) = audio_result.map_err(|e| e.to_string())?;
    let capture_handle = match video_result.map_err(|e| e.to_string()) {
        Ok(Ok(h)) => h,
        Ok(Err(e)) => { cleanup_failed_start(app, audio_handles, live); return Err(e); }
        Err(e) => { cleanup_failed_start(app, audio_handles, live); return Err(e); }
    };
    Ok(finish_starting(
        app,
        RecordTarget::Area { x, y, w, h },
        video,
        encoder,
        output_path,
        audio_specs,
        audio_handles,
        mailbox,
        capture_handle,
        capture_paused,
        live,
        true,
    ))
}

/// Tears down a stopped recording's capture/writer/audio threads on a
/// detached thread, so a slow native teardown never blocks Stop's response.
/// The `JoinHandle` is stashed on the manager (not just fired-and-forgotten)
/// so a start request right on Stop's heels can wait for this to actually
/// finish instead of racing it for the same capture device/encoder session.
fn teardown_active(app: &AppHandle, active: ActiveRecording) {
    let manager = app.state::<Arc<RecordingManager>>().inner().clone();
    let session_id = active.session.id.clone();
    let handle = std::thread::spawn(move || {
        log::info!("teardown[{session_id}]: stopping capture");
        // `on_closed` doesn't fire on a manual stop, so the mailbox must be
        // closed explicitly or the writer thread parks in `pop()` forever.
        if let Some(handle) = active.capture_handle {
            match handle.stop() {
                Ok(()) => log::info!("teardown[{session_id}]: capture stopped"),
                Err(e) => log::warn!("teardown[{session_id}]: capture.stop() failed: {e:?}"),
            }
        }
        active.mailbox.close();
        log::info!("teardown[{session_id}]: joining writer thread");
        if let Some(t) = active.writer_thread {
            let _ = t.join();
        }
        log::info!("teardown[{session_id}]: writer thread joined, stopping {} audio handle(s)", active.audio_handles.len());
        for handle in active.audio_handles {
            handle.stop();
        }
        log::info!("teardown[{session_id}]: done");
    });
    *manager.pending_teardown.lock().unwrap() = Some(handle);
}

pub fn stop_recording(app: &AppHandle) -> Result<PathBuf, String> {
    let manager = app.state::<Arc<RecordingManager>>();
    let active = manager.active.lock().unwrap().take().ok_or("No recording in progress")?;
    hud::hide(app);
    crate::tray::set_tray_recording(app, false);
    crate::tray::refresh_tray_menu(app);

    let output_path = active.session.output_path.clone();
    // Report "stopped" the instant the slot is freed, not after teardown.
    // The file isn't done yet; the finish watcher emits `video-saved` later.
    let _ = app.emit("recording-stopped", &active.session);
    notify_recording_toast(app, "Recording stopped", &active.session.target);
    crate::sound::play(&app.state::<Arc<ConfigStore>>().get().sound_effects.recording_stopped);
    if let Some(bid) = active.live_broadcast.lock().unwrap().clone() {
        finalize_live_entry(app, &bid, active.session.started_at);
        end_live_broadcast_detached(app, bid, true);
    }
    teardown_active(app, active);
    Ok(output_path)
}

/// Stamps the session length onto the gallery's virtual YouTube-live entry
/// so the card can show a duration without a YouTube API round-trip.
fn finalize_live_entry(app: &AppHandle, broadcast_id: &str, started_at: i64) {
    let meta = app.state::<Arc<crate::meta::MetaStore>>();
    let key = format!("yt_{broadcast_id}");
    if let Some(mut m) = meta.get(&key) {
        m.duration_secs = Some((chrono::Utc::now().timestamp() - started_at).max(0) as u64);
        meta.set(key.clone(), m);
    }
    let _ = app.emit("video-saved", serde_json::json!({ "name": key }));
}

/// Best-effort close of the session's YouTube broadcast (the RTMP feed
/// dropping triggers auto-stop anyway; this just makes it immediate).
fn end_live_broadcast_detached(app: &AppHandle, broadcast_id: String, notify_link: bool) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let settings = app.state::<Arc<ConfigStore>>().get();
        let cid = settings.effective_google_client_id().to_string();
        let csec = settings.effective_google_client_secret().to_string();
        let drive = app.state::<Arc<crate::drive::DriveClient>>();
        drive.end_live_broadcast(&cid, &csec, &broadcast_id).await;
        if notify_link {
            crate::toast::show(&app, "info", crate::toast::ToastCategory::Stream, "Stream ended",
                &format!("https://youtube.com/watch?v={broadcast_id}"));
        }
    });
}

/// Like `stop_recording`, but discards: waits for teardown to finish so the
/// files it deletes are fully written. Callers wrap this in `spawn_blocking`.
pub fn cancel_recording(app: &AppHandle) -> Result<(), String> {
    let manager = app.state::<Arc<RecordingManager>>();
    let active = manager.active.lock().unwrap().take().ok_or("No recording in progress")?;
    hud::hide(app);
    crate::tray::set_tray_recording(app, false);
    crate::tray::refresh_tray_menu(app);

    let local_files = active.local_files.lock().unwrap().clone();
    let _ = app.emit("recording-stopped", &active.session);
    notify_recording_toast(app, "Recording discarded", &active.session.target);
    if let Some(bid) = active.live_broadcast.lock().unwrap().clone() {
        finalize_live_entry(app, &bid, active.session.started_at);
        end_live_broadcast_detached(app, bid, false);
    }

    // As in `teardown_active`: the mailbox must be closed explicitly.
    if let Some(handle) = active.capture_handle {
        let _ = handle.stop();
    }
    active.mailbox.close();
    if let Some(t) = active.writer_thread {
        let _ = t.join();
    }
    for handle in active.audio_handles {
        handle.stop();
    }
    // Each local off/on cycle produced a fresh file — discard all of them.
    let mut errors = Vec::new();
    for path in &local_files {
        if let Err(e) = std::fs::remove_file(path) {
            errors.push(format!("{}: {e}", path.display()));
        }
    }
    if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
}

/// Turns the local file on/off independently of any live stream. Off
/// finishes the current file; back on starts a brand new one. If this was
/// the only active output, turning it off is just Stop. Returns the new state.
pub async fn toggle_local_recording(app: &AppHandle) -> Result<bool, String> {
    let manager = app.state::<Arc<RecordingManager>>();

    let (local_on, live_on, output_dir, ext, target) = {
        let guard = manager.active.lock().unwrap();
        let active = guard.as_ref().ok_or("No recording in progress")?;
        (
            active.local_active.load(std::sync::atomic::Ordering::Relaxed),
            active.live_active.load(std::sync::atomic::Ordering::Relaxed),
            active.output_dir.clone(),
            active.video.container.extension().to_string(),
            active.target.clone(),
        )
    };

    if local_on {
        if !live_on {
            let app = app.clone();
            let result = tauri::async_runtime::spawn_blocking(move || stop_recording(&app))
                .await
                .map_err(|e| e.to_string())?;
            result?;
            return Ok(false);
        }
        let mut guard = manager.active.lock().unwrap();
        let active = guard.as_mut().ok_or("No recording in progress")?;
        active.local_active.store(false, std::sync::atomic::Ordering::Relaxed);
        active.session.current_local_name = None;
        active.session.current_local_path = None;
        let _ = active.control_tx.send(WriterControl::DisableLocal);
        drop(guard);
        notify_recording_toast(app, "Recording stopped", &target);
    } else {
        let new_path = make_video_save_path(&output_dir, &ext, "").map_err(|e| e.to_string())?;
        if let RecordTarget::Window { app: app_name, .. } = &target {
            let file_name = relative_video_name(app, &new_path);
            if !file_name.is_empty() {
                app.state::<Arc<crate::meta::MetaStore>>().set(
                    file_name,
                    crate::meta::VideoMeta {
                        title: None, // see the other `prepare()` call site's comment
                        app: Some(app_name.clone()),
                        created: Some(chrono::Utc::now().timestamp()),
                        kind: None,
                        stream_info: None,
                        duration_secs: None,
                        youtube_video_id: None,
                        tags: Vec::new(),
                        favorite: false,
                    },
                );
            }
        }
        let mut guard = manager.active.lock().unwrap();
        let active = guard.as_mut().ok_or("No recording in progress")?;
        active.local_files.lock().unwrap().push(new_path.clone());
        active.local_active.store(true, std::sync::atomic::Ordering::Relaxed);
        active.session.current_local_name = Some(relative_video_name(app, &new_path)).filter(|n| !n.is_empty());
        active.session.current_local_path = Some(new_path.clone());
        let _ = active.control_tx.send(WriterControl::EnableLocal(new_path));
        drop(guard);
        notify_recording_toast(app, "Recording started", &target);
    }
    if let Some(s) = manager.current_session() {
        let _ = app.emit("recording-started", &s);
    }
    Ok(!local_on)
}

/// Turns the live feed on/off independently of the local file. Turning it
/// back on starts a brand new broadcast. Returns the new state.
pub async fn toggle_live_streaming(app: &AppHandle) -> Result<bool, String> {
    let manager = app.state::<Arc<RecordingManager>>();

    let (local_on, live_on, target, video, fps, started_at) = {
        let guard = manager.active.lock().unwrap();
        let active = guard.as_ref().ok_or("No recording in progress")?;
        (
            active.local_active.load(std::sync::atomic::Ordering::Relaxed),
            active.live_active.load(std::sync::atomic::Ordering::Relaxed),
            active.target.clone(),
            active.video.clone(),
            active.session.fps,
            active.session.started_at,
        )
    };

    if live_on {
        if !local_on {
            let app = app.clone();
            let result = tauri::async_runtime::spawn_blocking(move || stop_recording(&app))
                .await
                .map_err(|e| e.to_string())?;
            result?;
            return Ok(false);
        }
        let bid = {
            let guard = manager.active.lock().unwrap();
            let active = guard.as_ref().ok_or("No recording in progress")?;
            let bid = active.live_broadcast.lock().unwrap().take();
            bid
        };
        if let Some(bid) = bid {
            finalize_live_entry(app, &bid, started_at);
            end_live_broadcast_detached(app, bid, true);
        }
        let mut guard = manager.active.lock().unwrap();
        let active = guard.as_mut().ok_or("No recording in progress")?;
        active.live_active.store(false, std::sync::atomic::Ordering::Relaxed);
        active.session.live_bitrate_kbps = None;
        active.session.live_fps = None;
        active.session.live_resolution = None;
        let _ = active.control_tx.send(WriterControl::DisableLive);
        drop(guard);
    } else {
        let game = match &target {
            RecordTarget::Window { app: app_name, .. } => Some(app_name.clone()),
            _ => None,
        };
        let broadcast = try_start_live_broadcast(app, game.as_deref())
            .await
            .ok_or("Could not start the live broadcast")?;

        let cap = app.state::<Arc<ConfigStore>>().get().video.youtube_live.clone();
        let params = encoder::LiveStreamParams {
            rtmp_url: broadcast.rtmp_url.clone(),
            bitrate_kbps: video.bitrate_kbps.min(cap.max_bitrate_kbps),
            resolution: tighter_resolution(video.resolution, cap.max_resolution),
            max_fps: fps.min(cap.max_fps),
            keyframe_interval_secs: cap.keyframe_interval_secs,
            audio_codec: cap.audio_codec,
            audio_sample_rate: cap.audio_sample_rate,
            cbr_buffer_secs: cap.cbr_buffer_secs,
        };

        let entry_name = format!("yt_{}", broadcast.broadcast_id);
        let mut stats: Vec<String> = Vec::new();
        if let Some(h) = params.resolution.height() {
            stats.push(format!("{h}p"));
        }
        stats.push(format!("{} FPS", params.max_fps));
        stats.push(format!("{} Mbps", params.bitrate_kbps / 1000));
        let (title_val, app_val) = match &target {
            RecordTarget::Window { title, app: app_name, .. } => (title.clone(), Some(app_name.clone())),
            RecordTarget::Monitor => ("Desktop".to_string(), None),
            RecordTarget::Area { .. } => ("Screen Area".to_string(), None),
        };
        app.state::<Arc<crate::meta::MetaStore>>().set(
            entry_name.clone(),
            crate::meta::VideoMeta {
                title: Some(broadcast.title.clone()).filter(|t| !t.is_empty()).or(Some(title_val)),
                app: app_val,
                created: Some(chrono::Utc::now().timestamp()),
                kind: Some("youtube_live".to_string()),
                stream_info: Some(stats.join(" · ")),
                duration_secs: None,
                youtube_video_id: Some(broadcast.broadcast_id.clone()),
                tags: Vec::new(),
                favorite: false,
            },
        );
        let _ = app.emit("video-saved", serde_json::json!({ "name": entry_name }));

        let mut guard = manager.active.lock().unwrap();
        let active = guard.as_mut().ok_or("No recording in progress")?;
        *active.live_broadcast.lock().unwrap() = Some(broadcast.broadcast_id.clone());
        active.live_active.store(true, std::sync::atomic::Ordering::Relaxed);
        active.session.live_bitrate_kbps = Some(params.bitrate_kbps);
        active.session.live_fps = Some(params.max_fps);
        active.session.live_resolution = Some(params.resolution);
        let _ = active.control_tx.send(WriterControl::EnableLive(Box::new(params)));
        drop(guard);
    }
    if let Some(s) = manager.current_session() {
        let _ = app.emit("recording-started", &s);
    }
    Ok(!live_on)
}

pub(crate) fn uuid_v4() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let bytes: [u8; 16] = std::array::from_fn(|_| rng.gen());
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    )
}
