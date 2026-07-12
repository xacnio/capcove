//! Replay buffer: continuously encodes into rotating fixed-length segment
//! files, with old segments pruned in the background. "Save replay"
//! concatenates the closed segments in the window with no re-encode.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime};

use chrono::Local;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::config::{ConfigStore, EncoderChoice, ReplayBufferStorage, ReplayBufferTarget};
use crate::recording::capture_session::{self, CaptureHandle, FrameMailbox};
use crate::recording::encoder::AudioStreamSpec;
use crate::recording::{encoder, resolve_encoder, start_configured_audio_sources, RelayedAudioCapture};

const SEGMENT_SECONDS: u32 = 15;
/// Sanity ceiling on a single MP4 box's declared size, in `MemRing::push_stream`'s
/// streaming parser — a real `mdat` (one ~2s GOP) never gets remotely close to
/// this even at the app's highest bitrate. Without this, a corrupted/bogus
/// size read from ffmpeg's stdout (a stray byte sequence misread as a box
/// header, a torn pipe read) would make the parser wait forever for enough
/// bytes to complete a box that will never arrive, growing `parse_buf`
/// unboundedly for the rest of the session instead of resyncing.
const MAX_BOX_SIZE: usize = 256 * 1024 * 1024;
/// Extra margin (on top of buffer_minutes) before a segment is eligible for
/// cleanup — guards against deleting a segment that's still partially within
/// the requested window due to the coarse 15s granularity.
const CLEANUP_MARGIN_SECONDS: u64 = SEGMENT_SECONDS as u64 * 2;

struct ActiveReplayBuffer {
    capture_handle: Option<CaptureHandle>,
    mailbox: Arc<FrameMailbox>,
    writer_thread: Option<JoinHandle<()>>,
    cleanup_stop: Arc<AtomicBool>,
    cleanup_thread: Option<JoinHandle<()>>,
    audio_handles: Vec<RelayedAudioCapture>,
    segment_dir: PathBuf,
    buffer_minutes: u32,
    started_at: SystemTime,
    /// The target this buffer instance is actually capturing (post-override).
    target: ReplayBufferTarget,
    /// Present in memory storage mode — the in-RAM ring holding the encoded
    /// stream instead of disk segments.
    mem_ring: Option<Arc<Mutex<MemRing>>>,
    /// Resolved (never `Auto`) encoder and effective bitrate/resolution/fps
    /// this buffer started with, post per-game overrides.
    encoder: EncoderChoice,
    bitrate_kbps: u32,
    resolution: crate::config::RecordingResolution,
    fps: u32,
}

/// In-RAM ring of the encoded **fragmented-MP4** stream (memory storage mode).
/// The `ftyp`+`moov` init segment is kept once; each `moof`+`mdat` fragment
/// (one per keyframe/GOP) is stored with the time it arrived and evicted once
/// older than the buffer window, keeping RAM at roughly bitrate × window. A
/// snapshot is `init` + every in-window fragment — a self-contained fMP4 that
/// remuxes cleanly from its first (keyframe) fragment. This is what lets AV1,
/// HEVC and FLAC work in memory mode where the old MPEG-TS ring couldn't.
pub struct MemRing {
    init: Vec<u8>,
    // `Arc`, not a bare `Vec<u8>`: lets a save snapshot the in-window fragments
    // by cloning cheap pointer/refcount pairs (see `snapshot_refs`) instead of
    // copying every buffered byte a second time while writing them out.
    fragments: std::collections::VecDeque<(std::time::Instant, Arc<Vec<u8>>)>,
    total_bytes: usize,
    max_age: Duration,
    // Streaming top-level-box splitter state (ffmpeg stdout arrives in
    // arbitrary chunks, not box-aligned).
    parse_buf: Vec<u8>,
    pending_moof: Option<Vec<u8>>,
    init_done: bool,
}

impl MemRing {
    fn new(max_age: Duration) -> Self {
        Self {
            init: Vec::new(),
            fragments: std::collections::VecDeque::new(),
            total_bytes: 0,
            max_age,
            parse_buf: Vec::new(),
            pending_moof: None,
            init_done: false,
        }
    }

    /// Feeds raw ffmpeg-stdout bytes through a minimal MP4 box splitter:
    /// `ftyp`/`moov` accumulate into the init segment; each `moof`+`mdat` pair
    /// becomes one age-stamped fragment. Incomplete trailing boxes stay
    /// buffered until the rest arrives.
    fn push_stream(&mut self, chunk: &[u8]) {
        self.parse_buf.extend_from_slice(chunk);
        let mut off = 0;
        loop {
            // Parse just the header first, ending the borrow of `parse_buf`
            // before we mutate `self` below.
            let (typ, size) = {
                let buf = &self.parse_buf[off..];
                if buf.len() < 8 {
                    break;
                }
                let mut size = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
                let mut hdr = 8usize;
                if size == 1 {
                    if buf.len() < 16 {
                        break;
                    }
                    size = u64::from_be_bytes(buf[8..16].try_into().unwrap()) as usize;
                    hdr = 16;
                }
                // size 0 ("to end of stream") never appears in ffmpeg's
                // fragmented output; a size below the header, or above
                // `MAX_BOX_SIZE`, is corruption — either way, drop what we
                // have and resync on the next boxes instead of buffering
                // forever waiting for a box that will never complete.
                if size < hdr || size > MAX_BOX_SIZE {
                    self.parse_buf.clear();
                    self.pending_moof = None;
                    return;
                }
                if buf.len() < size {
                    break;
                }
                ([buf[4], buf[5], buf[6], buf[7]], size)
            };
            let box_bytes = self.parse_buf[off..off + size].to_vec();
            off += size;
            match &typ {
                b"ftyp" | b"moov" => {
                    if !self.init_done {
                        self.init.extend_from_slice(&box_bytes);
                    }
                    if &typ == b"moov" {
                        self.init_done = true;
                    }
                }
                b"moof" => self.pending_moof = Some(box_bytes),
                b"mdat" => {
                    if let Some(mut frag) = self.pending_moof.take() {
                        frag.extend_from_slice(&box_bytes);
                        self.total_bytes += frag.len();
                        self.fragments.push_back((std::time::Instant::now(), Arc::new(frag)));
                    }
                }
                // styp/sidx/mfra/free etc. — not needed to reconstruct the ring.
                _ => {}
            }
        }
        if off > 0 {
            self.parse_buf.drain(..off);
        }
        while let Some((t, f)) = self.fragments.front() {
            if t.elapsed() > self.max_age {
                self.total_bytes -= f.len();
                self.fragments.pop_front();
            } else {
                break;
            }
        }
    }

    /// A cheap snapshot of everything within `window`: the (small) init
    /// segment is copied once, and each in-window fragment is a bumped `Arc`
    /// refcount rather than a byte copy. Safe to call while holding the
    /// ring's lock only briefly — the caller then writes each `Arc<Vec<u8>>`
    /// to disk (e.g. via `write_all`) *after* releasing the lock, so a save
    /// never holds a second full copy of the buffered window in RAM on top
    /// of the ring's own steady-state footprint, and never blocks the
    /// encoder's stdout reader for the duration of the disk write either.
    fn snapshot_refs(&self, window: Duration) -> (Vec<u8>, Vec<Arc<Vec<u8>>>) {
        let frags = self.fragments.iter()
            .filter(|(t, _)| t.elapsed() <= window)
            .map(|(_, f)| f.clone())
            .collect();
        (self.init.clone(), frags)
    }

    /// How many fragments fall within `window` — lets the save path tell
    /// "enough footage buffered" from "the buffer only just started".
    fn fragment_count(&self, window: Duration) -> usize {
        self.fragments.iter().filter(|(t, _)| t.elapsed() <= window).count()
    }
}

#[derive(Default)]
pub struct ReplayBufferManager {
    active: Mutex<Option<ActiveReplayBuffer>>,
    /// Set for the whole span between a start being accepted and `active`
    /// actually being populated — `start_replay_buffer_with_target` does a
    /// lot of `.await`ing (encoder resolution, audio device negotiation,
    /// capture startup) before `active` is set, so checking only
    /// `active.is_some()` at the top left a race window where two concurrent
    /// start calls could both pass that check and both fully start, the
    /// second silently overwriting the first's still-running buffer (leaking
    /// its capture/writer/audio/cleanup threads forever). Same fix as
    /// `recording::RecordingManager::starting` — see its doc comment.
    starting: std::sync::atomic::AtomicBool,
}

/// Releases `ReplayBufferManager::starting` on drop — including on an early
/// `?` return anywhere in `start_replay_buffer_with_target`.
struct BufferStartGuard(Arc<ReplayBufferManager>);

impl Drop for BufferStartGuard {
    fn drop(&mut self) {
        self.0.starting.store(false, std::sync::atomic::Ordering::Release);
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ReplayBufferStatus {
    pub running: bool,
    pub buffered_seconds: u64,
    /// The cap `buffered_seconds` grows up to (buffer length in seconds) — the
    /// UI ticks the counter locally between status polls and needs this so it
    /// stops at the same ceiling the backend does instead of overshooting.
    pub max_seconds: Option<u64>,
    /// Buffer start as a Unix timestamp (seconds) — lets the UI derive the live
    /// counter as `now - started_at` (capped at `max_seconds`) and tick it each
    /// second itself, instead of only stepping when the status poll lands.
    pub started_at: Option<u64>,
    /// Game/app name when the buffer targets a specific window (game
    /// detection) — feeds the gallery titlebar's status block.
    pub app: Option<String>,
    /// The buffer's actual (post-override) encoder/bitrate/resolution/fps —
    /// `None` when not running.
    pub encoder: Option<EncoderChoice>,
    pub bitrate_kbps: Option<u32>,
    pub resolution: Option<crate::config::RecordingResolution>,
    pub fps: Option<u32>,
}

impl ReplayBufferManager {
    pub fn is_running(&self) -> bool {
        self.active.lock().unwrap().is_some()
    }

    pub fn current_target(&self) -> Option<ReplayBufferTarget> {
        self.active.lock().unwrap().as_ref().map(|a| a.target.clone())
    }

    pub fn status(&self) -> ReplayBufferStatus {
        match self.active.lock().unwrap().as_ref() {
            Some(a) => {
                let elapsed = a.started_at.elapsed().unwrap_or_default().as_secs();
                let app = match &a.target {
                    ReplayBufferTarget::SpecificWindow { app, .. } => Some(app.clone()),
                    ReplayBufferTarget::PrimaryMonitor => None,
                };
                let max_seconds = a.buffer_minutes as u64 * 60;
                let started_at = a.started_at.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).ok();
                ReplayBufferStatus {
                    running: true,
                    buffered_seconds: elapsed.min(max_seconds),
                    max_seconds: Some(max_seconds),
                    started_at,
                    app,
                    encoder: Some(a.encoder.clone()),
                    bitrate_kbps: Some(a.bitrate_kbps),
                    resolution: Some(a.resolution),
                    fps: Some(a.fps),
                }
            }
            None => ReplayBufferStatus { running: false, buffered_seconds: 0, max_seconds: None, started_at: None, app: None, encoder: None, bitrate_kbps: None, resolution: None, fps: None },
        }
    }
}

fn segment_dir_for(app: &AppHandle) -> PathBuf {
    app.path().temp_dir().unwrap_or_else(|_| std::env::temp_dir()).join("dev.xacnio.capcove").join("replay_segments")
}

fn spawn_segment_writer(
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
    segment_dir: PathBuf,
    // Buffered window, when targeting a window — distinguishes "minimized"
    // from "content just isn't changing".
    window_hwnd: Option<u32>,
    // Privacy toggle: write an "Alt-tabbed" card while the window isn't foreground.
    alt_tab_privacy: bool,
    // What to write while the window is minimized (card / black / pause).
    minimized_behavior: crate::config::MinimizedBehavior,
    // Flipped while paused so the audio callbacks drop their bytes in step.
    capture_paused: Arc<AtomicBool>,
    // Memory storage mode: the ring collecting the encoder's TS stdout.
    // None = disk segments.
    mem_ring: Option<Arc<Mutex<MemRing>>>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        use crate::recording::capture_session::PopResult;
        use crate::recording::placeholder;
        let frame_interval = std::time::Duration::from_millis(1000 / fps.max(1) as u64);
        // A `JobFeeder`, not the raw `EncodeJob` — a blocking write here would
        // stall this loop's real-time pacing too (see `JobFeeder`'s doc comment),
        // silently shortening the encoded duration and speeding up playback.
        let mut job: Option<encoder::JobFeeder> = None;
        let mut dims: Option<(u32, u32)> = None;
        let mut last_frame: Option<Arc<[u8]>> = None; // most recent real frame
        let mut warned_tiny = false;
        let mut tiny_since: Option<std::time::Instant> = None;
        // Freeze mode: swallows post-restore black frames.
        let mut was_minimized = false;
        let mut restore_grace: Option<std::time::Instant> = None;
        let mut cards: std::collections::HashMap<&'static str, Arc<[u8]>> = std::collections::HashMap::new();
        loop {
            // Capture only delivers frames on content change, so a timeout
            // means "repeat the last frame"; cards key off the window's real state.
            match mailbox.pop_timeout(frame_interval) {
                PopResult::Closed => break,
                PopResult::Frame(msg) => {
                    // Don't lock encoder dimensions onto a launcher-splash-sized frame —
                    // but after a grace period, accept a legitimately small stable
                    // window rather than waiting forever (see `spawn_writer_thread`).
                    if job.is_none()
                        && (msg.width < crate::recording::MIN_CAPTURE_W || msg.height < crate::recording::MIN_CAPTURE_H)
                    {
                        let past_grace = tiny_since.is_some_and(|t| t.elapsed() >= crate::recording::TINY_FRAME_GRACE);
                        let above_hard_floor = msg.width >= crate::recording::HARD_MIN_CAPTURE_W && msg.height >= crate::recording::HARD_MIN_CAPTURE_H;
                        if !(past_grace && above_hard_floor) {
                            tiny_since.get_or_insert_with(std::time::Instant::now);
                            if !warned_tiny {
                                warned_tiny = true;
                                log::warn!("buffer capture frames are {}x{} — waiting for a real-sized frame before starting the encoder", msg.width, msg.height);
                            }
                            continue;
                        }
                        log::info!("buffer capture frames stayed {}x{} after the grace period — starting the encoder at that size", msg.width, msg.height);
                    }
                    if job.is_none() {
                        let spawned = match &mem_ring {
                            Some(_) => encoder::spawn_fmp4_stream(&app, msg.width, msg.height, fps, &encoder, bitrate_kbps, rate_control, quality, &audio_specs, &audio_codec, &resolution),
                            None => encoder::spawn_segmented(&app, msg.width, msg.height, fps, &encoder, bitrate_kbps, rate_control, quality, &audio_specs, &audio_codec, &resolution, SEGMENT_SECONDS, &segment_dir),
                        };
                        match spawned {
                            Ok((new_job, rx)) => {
                                spawn_buffer_ffmpeg_watcher(app.clone(), rx, mem_ring.clone());
                                job = Some(encoder::JobFeeder::spawn(new_job, "replay buffer"));
                                // Even size ffmpeg was told via `-s` — see `encoder::fit_frame_to_dims`.
                                dims = Some(encoder::even_dims(msg.width, msg.height));
                            }
                            Err(e) => {
                                log::error!("failed to start replay buffer encoder: {e}");
                                break;
                            }
                        }
                    }
                    // Freeze mode: hold the frozen picture through the black
                    // frames delivered right after a restore.
                    let in_grace = minimized_behavior == crate::config::MinimizedBehavior::Freeze
                        && (was_minimized || restore_grace.map(|u| std::time::Instant::now() < u).unwrap_or(false));
                    if in_grace && placeholder::is_mostly_black(&msg.bgra) {
                        // swallow it — last_frame stays frozen
                    } else {
                        restore_grace = None;
                        // Scaled/cropped to the size the encoder started with
                        // — `msg.width/height` can differ from `dims` if the
                        // recorded window's been resized since (see
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
            if occlusion == placeholder::Occlusion::Minimized
                && minimized_behavior == crate::config::MinimizedBehavior::Pause
            {
                // Paused: skip video and signal the audio callbacks to drop
                // their bytes so all tracks stay in lockstep.
                capture_paused.store(true, Ordering::Relaxed);
                continue;
            }
            capture_paused.store(false, Ordering::Relaxed);
            let card_text = placeholder::card_for(occlusion, minimized_behavior);
            let bytes: Option<Arc<[u8]>> = match (card_text, dims) {
                (Some(text), Some((w, h))) => Some(
                    cards.entry(text).or_insert_with(|| Arc::from(placeholder::render(w, h, text))).clone(),
                ),
                _ => last_frame.clone(),
            };

            if let (Some(j), Some(bytes)) = (job.as_ref(), bytes) {
                if !j.send(bytes) {
                    log::error!("replay buffer encoder feeder exited (write error or ffmpeg quit)");
                    break;
                }
            }
        }
        if let Some(j) = job.take() {
            j.stop();
        }
    })
}

/// Drains the buffer encoder's event channel: feeds the memory ring in
/// memory mode, and — should ffmpeg die unexpectedly — stops the buffer so
/// it isn't left "running" while encoding nothing.
fn spawn_buffer_ffmpeg_watcher(
    app: AppHandle,
    mut rx: tauri::async_runtime::Receiver<tauri_plugin_shell::process::CommandEvent>,
    ring: Option<Arc<Mutex<MemRing>>>,
) {
    use tauri_plugin_shell::process::CommandEvent;
    tauri::async_runtime::spawn(async move {
        let mut exit_code = None;
        while let Some(ev) = rx.recv().await {
            match ev {
                CommandEvent::Stdout(chunk) => {
                    if let Some(r) = &ring {
                        r.lock().unwrap().push_stream(&chunk);
                    }
                }
                CommandEvent::Stderr(bytes) => log::debug!("buffer ffmpeg: {}", String::from_utf8_lossy(&bytes).trim_end()),
                CommandEvent::Terminated(p) => exit_code = p.code,
                _ => {}
            }
        }
        if exit_code != Some(0) {
            let manager = app.state::<Arc<ReplayBufferManager>>();
            if manager.is_running() {
                log::warn!("replay buffer encoder exited with an error (code {exit_code:?}) — stopping the buffer");
                crate::notify_error(&app, "Replay buffer stopped unexpectedly (encoder error)");
                let app2 = app.clone();
                let _ = tauri::async_runtime::spawn_blocking(move || stop_replay_buffer(&app2)).await;
            }
        }
    });
}

fn spawn_cleanup_thread(segment_dir: PathBuf, buffer_minutes: u32, stop: Arc<AtomicBool>) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let max_age = Duration::from_secs(buffer_minutes as u64 * 60 + CLEANUP_MARGIN_SECONDS);
        while !stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_secs(30));
            let Ok(entries) = std::fs::read_dir(&segment_dir) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(meta) = entry.metadata() else { continue };
                let Ok(modified) = meta.modified() else { continue };
                if modified.elapsed().unwrap_or_default() > max_age {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
    })
}

pub async fn start_replay_buffer(app: &AppHandle) -> Result<(), String> {
    start_replay_buffer_with_target(app, None).await
}

/// Like `start_replay_buffer`, but `override_target` (when given) wins over
/// the configured `replay_buffer.target` — used by game detection to point
/// the buffer at the detected game window without touching saved settings.
pub async fn start_replay_buffer_with_target(app: &AppHandle, override_target: Option<ReplayBufferTarget>) -> Result<(), String> {
    let manager = app.state::<Arc<ReplayBufferManager>>().inner().clone();
    if manager.is_running() {
        return Ok(()); // already running — idempotent
    }
    // Atomically reserves the start slot — see `ReplayBufferManager::starting`'s
    // doc comment for why `is_running()` alone leaves a race window across
    // this function's `.await`s. Same idempotent "no-op" semantics as the
    // already-running check above; released automatically (even on an early
    // `?` return) once `BufferStartGuard` drops.
    if manager.starting.compare_exchange(
        false, true, std::sync::atomic::Ordering::AcqRel, std::sync::atomic::Ordering::Acquire,
    ).is_err() {
        return Ok(());
    }
    let _start_guard = BufferStartGuard(manager.clone());
    // A recording in progress is not an obstacle — the buffer runs its own
    // capture session, encoder, and audio captures alongside it.

    let store = app.state::<Arc<ConfigStore>>();
    let settings = store.get();
    let mut video = settings.video.clone();
    let mut rb = video.replay_buffer.clone();
    if let Some(target) = override_target {
        rb.target = target;
    }
    // Game-targeted buffer: apply that game's per-game capture overrides,
    // remembering its buffer-only override (applied last, below).
    let mut per_game_buffer_video = None;
    if let ReplayBufferTarget::SpecificWindow { app: game_app, .. } = &rb.target {
        if let Some(ov) = app.state::<Arc<crate::games_db::GamesDb>>().overrides_for(game_app) {
            log::info!("applying per-game capture overrides for '{game_app}'");
            ov.apply_to(&mut video);
            per_game_buffer_video = Some(ov.replay_buffer_video.clone());
        }
    }
    // Buffer-specific overrides win over the full-recording ones above, for
    // the buffer's own encode only: first the global "always run the buffer
    // at X" setting, then (most specific) this exact game's own buffer-only
    // override.
    if rb.use_custom_video {
        rb.video_override.apply_to(&mut video);
    }
    if let Some(ov) = per_game_buffer_video {
        ov.apply_to(&mut video);
    }
    let encoder = resolve_encoder(app, &video.encoder).await;

    // Memory mode streams fragmented MP4 into the RAM ring, which carries every
    // codec (incl. AV1) and audio format we offer — no per-codec fallback.
    let storage = rb.storage;
    let mem_ring = (storage == ReplayBufferStorage::Memory).then(|| {
        Arc::new(Mutex::new(MemRing::new(Duration::from_secs(
            rb.buffer_minutes as u64 * 60 + CLEANUP_MARGIN_SECONDS,
        ))))
    });

    let segment_dir = segment_dir_for(app);
    let _ = std::fs::remove_dir_all(&segment_dir);
    std::fs::create_dir_all(&segment_dir).map_err(|e| e.to_string())?;
    // Persisted alongside the segments (survives the pending-clip/crash-
    // recovery rename, and a restart) so a save that has no live buffer state
    // to consult — pending clip, crash recovery — can still put the file in
    // the right game subfolder instead of always falling back to the root.
    if let ReplayBufferTarget::SpecificWindow { app: game_app, .. } = &rb.target {
        write_buffer_game_name(&segment_dir, game_app);
    }

    let mailbox = FrameMailbox::new();
    let capture_handle = match &rb.target {
        ReplayBufferTarget::PrimaryMonitor => capture_session::start_monitor_capture(None, video.fps, video.capture_cursor, None, mailbox.clone())?,
        ReplayBufferTarget::SpecificWindow { hwnd, .. } => capture_session::start_window_capture(*hwnd, video.fps, video.capture_cursor, video.exclude_overlay_windows, video.crop_titlebar, mailbox.clone())?,
    };
    let window_hwnd = match &rb.target {
        ReplayBufferTarget::SpecificWindow { hwnd, .. } => Some(*hwnd),
        ReplayBufferTarget::PrimaryMonitor => None,
    };

    // Audio tracks, same setup as full recordings, including the smart
    // game-track split. The pause flag keeps them in step with the video.
    let capture_paused = Arc::new(AtomicBool::new(false));
    let paused_for_audio = capture_paused.clone();
    let audio_config = video.audio.clone();
    let game_pid = window_hwnd.and_then(crate::capture::audio_pid_for_hwnd);
    let (audio_specs, audio_handles) = tauri::async_runtime::spawn_blocking(move || {
        start_configured_audio_sources(&audio_config, paused_for_audio, game_pid, false)
    })
    .await
    .map_err(|e| e.to_string())?;

    // Segments carry no track titles in MPEG-TS (dropped on mux — see
    // `audio_track_titles`), so the labels are saved alongside the segments
    // and reapplied at save time regardless of storage mode or which save
    // path runs (including crash/pending recovery, which have no live state).
    write_audio_track_titles(&segment_dir, &encoder::audio_track_titles(&audio_specs));

    let writer_thread = spawn_segment_writer(app.clone(), mailbox.clone(), video.fps, encoder.clone(), video.bitrate_kbps, video.rate_control, video.quality, audio_specs, video.audio_codec, video.resolution, segment_dir.clone(), window_hwnd, rb.alt_tab_privacy, video.minimized_behavior, capture_paused, mem_ring.clone());

    // Disk mode rotates segment files that need age-based cleanup; the
    // memory ring evicts old chunks itself as data arrives.
    let cleanup_stop = Arc::new(AtomicBool::new(false));
    let cleanup_thread = (storage == ReplayBufferStorage::Disk)
        .then(|| spawn_cleanup_thread(segment_dir.clone(), rb.buffer_minutes, cleanup_stop.clone()));

    *manager.active.lock().unwrap() = Some(ActiveReplayBuffer {
        capture_handle: Some(capture_handle),
        mailbox,
        writer_thread: Some(writer_thread),
        cleanup_stop,
        cleanup_thread,
        audio_handles,
        segment_dir,
        buffer_minutes: rb.buffer_minutes,
        started_at: SystemTime::now(),
        target: rb.target.clone(),
        mem_ring,
        encoder: encoder.clone(),
        bitrate_kbps: video.bitrate_kbps,
        resolution: video.resolution,
        fps: video.fps,
    });

    log::info!(
        "replay buffer started ({}min window, {} storage)",
        rb.buffer_minutes,
        if storage == ReplayBufferStorage::Memory { "memory" } else { "disk" }
    );
    notify_buffer_toast(app, "Instant Replay started", &rb.target);
    super::hud::set_buffer(app, true);
    #[cfg(windows)]
    crate::tray::set_tray_buffering(app, true);
    crate::sound::play(&app.state::<Arc<ConfigStore>>().get().sound_effects.buffer_started);
    Ok(())
}

/// Same shape as `notify_recording_toast` in mod.rs — a game-targeted
/// buffer gets its icon/name, a monitor one gets a plain "screen" body.
fn notify_buffer_toast(app: &AppHandle, title: &str, target: &ReplayBufferTarget) {
    match target {
        ReplayBufferTarget::SpecificWindow { app: name, .. } => {
            crate::toast::show_for_game(app, "info", crate::toast::ToastCategory::Buffer, title, name, name);
        }
        ReplayBufferTarget::PrimaryMonitor => {
            crate::toast::show(app, "info", crate::toast::ToastCategory::Buffer, title, "Buffering the screen");
        }
    }
}

pub fn stop_replay_buffer(app: &AppHandle) -> Result<(), String> {
    let manager = app.state::<Arc<ReplayBufferManager>>();
    let Some(active) = manager.active.lock().unwrap().take() else { return Ok(()) };
    notify_buffer_toast(app, "Instant Replay stopped", &active.target);
    super::hud::set_buffer(app, false);
    #[cfg(windows)]
    crate::tray::set_tray_buffering(app, false);
    crate::sound::play(&app.state::<Arc<ConfigStore>>().get().sound_effects.buffer_stopped);

    // Stopping the capture never triggers the closed-window callback, so the
    // mailbox must be closed explicitly or the writer thread parks forever.
    if let Some(handle) = active.capture_handle {
        let _ = handle.stop();
    }
    active.mailbox.close();
    if let Some(t) = active.writer_thread {
        let _ = t.join();
    }
    active.cleanup_stop.store(true, Ordering::Relaxed);
    if let Some(t) = active.cleanup_thread {
        let _ = t.join();
    }
    for handle in active.audio_handles {
        handle.stop();
    }
    let _ = std::fs::remove_dir_all(&active.segment_dir);
    Ok(())
}

/// Like `stop_replay_buffer`, but for Clips mode's "game just closed" path
/// when `confirm_save_on_close` is on: stops the (now-invalid — the window
/// it was capturing is gone) capture exactly the same way, but stages the
/// segment directory aside instead of deleting it outright, and reports it
/// so the gallery can ask the user whether to keep it as a clip. Reuses the
/// same disk-only staging trick as crash recovery — memory mode's buffer
/// lives only in RAM tied to the capture that just ended, so there's
/// nothing to stage for it, same limitation `stage_replay_buffer_crash_recovery` has.
pub fn stop_replay_buffer_for_pending_save(app: &AppHandle) {
    let manager = app.state::<Arc<ReplayBufferManager>>();
    let Some(active) = manager.active.lock().unwrap().take() else { return };
    let game = match &active.target {
        ReplayBufferTarget::SpecificWindow { app: name, .. } => Some(name.clone()),
        ReplayBufferTarget::PrimaryMonitor => None,
    };
    // No "Instant Replay stopped" toast here — the save/discard modal that's
    // about to show already says so, and the teardown below can take a
    // couple of seconds (waiting on the writer thread's ffmpeg finalize),
    // so firing it immediately would read as "it stopped" followed by a
    // confusing few-second gap before anything actually asks you anything.
    // It's only shown in the fallback paths below, where nothing else will.
    super::hud::set_buffer(app, false);
    #[cfg(windows)]
    crate::tray::set_tray_buffering(app, false);
    crate::sound::play(&app.state::<Arc<ConfigStore>>().get().sound_effects.buffer_stopped);

    if let Some(handle) = active.capture_handle {
        let _ = handle.stop();
    }
    active.mailbox.close();
    if let Some(t) = active.writer_thread {
        let _ = t.join();
    }
    active.cleanup_stop.store(true, Ordering::Relaxed);
    if let Some(t) = active.cleanup_thread {
        let _ = t.join();
    }
    for handle in active.audio_handles {
        handle.stop();
    }

    if active.mem_ring.is_some() {
        let _ = std::fs::remove_dir_all(&active.segment_dir);
        notify_buffer_toast(app, "Instant Replay stopped", &active.target);
        crate::notify(app, "Capcove", "Auto clipping stopped (game closed)");
        return;
    }
    let staging = replay_pending_staging_dir(app);
    let _ = std::fs::remove_dir_all(&staging); // clear any previous, never-resolved prompt
    if let Some(parent) = staging.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // The joins above only guarantee our own Rust-side threads are done —
    // the ffmpeg child process they were feeding can take a moment longer to
    // actually exit and release its file handles, which makes an immediate
    // rename() fail with "access denied" on Windows. Retry briefly instead
    // of giving up on what's almost always just that transient race.
    let mut staged = false;
    for attempt in 0..10 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(200));
        }
        if std::fs::rename(&active.segment_dir, &staging).is_ok() {
            staged = true;
            break;
        }
    }
    if !staged {
        log::warn!("pending clip: couldn't stage segments aside after retrying (still locked, or a different filesystem?) — discarding instead");
        let _ = std::fs::remove_dir_all(&active.segment_dir);
        notify_buffer_toast(app, "Instant Replay stopped", &active.target);
        crate::notify(app, "Capcove", "Auto clipping stopped (game closed)");
        return;
    }
    log::info!("pending clip: staged replay buffer segments for a save/discard prompt (game: {game:?})");
    report_pending_clip(app, game);
}

fn replay_pending_staging_dir(app: &AppHandle) -> PathBuf {
    app.path().temp_dir().unwrap_or_else(|_| std::env::temp_dir()).join("dev.xacnio.capcove").join("replay_segments_pending")
}

#[derive(Default)]
pub struct PendingClipState(Mutex<Option<serde_json::Value>>);

impl PendingClipState {
    pub fn take(&self) -> Option<serde_json::Value> {
        self.0.lock().unwrap().take()
    }
}

fn report_pending_clip(app: &AppHandle, game: Option<String>) {
    let payload = serde_json::json!({ "game": game });
    if let Some(state) = app.try_state::<Arc<PendingClipState>>() {
        *state.0.lock().unwrap() = Some(payload.clone());
    }
    let _ = app.emit("replay-buffer-pending-clip", payload);
    // Answering "keep this clip?" only matters right after the game closes —
    // surface the gallery now instead of leaving the prompt to be found
    // whenever the user next happens to open it.
    crate::tray::show_main(app);
}

/// The user chose to keep the clip that was pending after the game closed —
/// same disk-mode-only concat approach as `recover_replay_buffer_crash`.
pub async fn confirm_pending_clip(app: &AppHandle) -> Result<PathBuf, String> {
    let staging = replay_pending_staging_dir(app);
    let mut entries = list_segment_files(&staging)?;
    if entries.is_empty() {
        let _ = std::fs::remove_dir_all(&staging);
        return Err("Not enough buffered footage to save".into());
    }
    // `EncodeJob::finish` only closes ffmpeg's stdin (EOF) and returns — it
    // doesn't block until ffmpeg has actually finished flushing/finalizing
    // that segment's container, so the newest file can still have an
    // incorrect duration/timestamp if it's read too soon after being
    // (early, off the segment muxer's normal 15s boundary) finalized, which
    // plays back at the wrong speed once concatenated with the other,
    // properly-timed segments. Same reasoning `recover_replay_buffer_crash`
    // already applies to a crash's still-open segment — drop it here too,
    // unless it's the only footage there is.
    if entries.len() > 1 {
        entries.pop();
    }

    let store = app.state::<Arc<ConfigStore>>();
    let settings = store.get();
    let game = read_buffer_game_name(&staging);
    let dir = resolve_clip_dir(app, &settings, game.as_deref());
    let mut container = settings.video.container.clone();
    if settings.video.replay_buffer.use_custom_video {
        if let Some(c) = &settings.video.replay_buffer.video_override.container {
            container = c.clone();
        }
    }
    // MOV can't hold AV1 — fall back to the MP4 equivalent so the concat
    // doesn't fail with an empty file.
    let adjusted = container.compatible_with_av1(segments_are_av1(&entries));
    if adjusted != container {
        log::warn!("MOV can't carry AV1; saving replay clip as {} instead", adjusted.extension());
        container = adjusted;
    }
    let output_path = crate::recording::make_video_save_path(&dir, container.extension(), "Clip_").map_err(|e| e.to_string())?;

    let list_path = staging.join("concat_list.txt");
    let list_body = entries.iter()
        .map(|p| format!("file '{}'", p.to_string_lossy().replace('\'', "'\\''")))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&list_path, list_body).map_err(|e| e.to_string())?;

    let mut args: Vec<String> = vec![
        "-y".into(), "-f".into(), "concat".into(), "-safe".into(), "0".into(),
        "-i".into(), list_path.to_string_lossy().into_owned(),
        // -map 0 keeps every stream — default selection would
        // silently drop all but one audio track.
        "-map".into(), "0".into(), "-c".into(), "copy".into(),
    ];
    push_segment_audio_bsf(&mut args, &entries);
    push_audio_title_metadata(&mut args, &read_audio_track_titles(&staging));
    if let Some(flags) = container.movflags() {
        args.push("-movflags".into());
        args.push(flags.into());
    }
    args.push(output_path.to_string_lossy().into_owned());

    let sidecar = crate::integrity::ffmpeg_sidecar(app).map_err(|e| e.to_string())?;
    let output = sidecar.args(args).output().await.map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&staging);
    if !output.status.success() {
        return Err(format!("ffmpeg concat failed: {}", String::from_utf8_lossy(&output.stderr)));
    }

    let file_name = output_path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let _ = filetime::set_file_mtime(&output_path, filetime::FileTime::now());
    app.state::<Arc<crate::meta::MetaStore>>().set(file_name.to_string(), crate::meta::VideoMeta {
        created: Some(chrono::Utc::now().timestamp()),
        kind: Some("clip".to_string()),
        stream_info: None,
        app: game,
        ..Default::default()
    });
    let _ = app.emit("video-saved", serde_json::json!({
        "path": output_path.to_string_lossy(),
        "name": file_name,
    }));
    crate::toast::show(app, "info", crate::toast::ToastCategory::Clip, "Clip saved", file_name);
    crate::sound::play(&settings.sound_effects.clip_saved);
    log::info!("pending clip: saved as {file_name}");
    Ok(output_path)
}

/// The user chose not to keep the clip that was pending after the game closed.
pub fn discard_pending_clip(app: &AppHandle) -> Result<(), String> {
    let staging = replay_pending_staging_dir(app);
    std::fs::remove_dir_all(&staging).map_err(|e| e.to_string()).or_else(|e| {
        if staging.exists() { Err(e) } else { Ok(()) }
    })
}

/// Appends `-bsf:a aac_adtstoasc` to a concat command when the segments are
/// MPEG-TS (`.ts`) — TS carries AAC as ADTS, which must be stripped to raw
/// access units for an MP4/MKV target. Fragmented-MP4 (`.mp4`) segments store
/// raw AAC already and must NOT get the filter (it would error). A session's
/// segments share one container, so the first file decides.
fn push_segment_audio_bsf(args: &mut Vec<String>, segments: &[PathBuf]) {
    let is_ts = segments
        .first()
        .and_then(|p| p.extension())
        .and_then(|x| x.to_str())
        == Some("ts");
    if is_ts {
        args.push("-bsf:a".into());
        args.push("aac_adtstoasc".into());
    }
}

const AUDIO_TRACK_TITLES_FILE: &str = "audio_tracks.json";

/// Persists the buffer's audio track titles alongside its segments (see the
/// call site in `start_replay_buffer_with_target` for why). Best-effort: a
/// write failure just means the eventual save falls back to untitled tracks,
/// not a failure worth surfacing.
fn write_audio_track_titles(segment_dir: &std::path::Path, titles: &[String]) {
    if titles.is_empty() {
        return;
    }
    if let Ok(json) = serde_json::to_vec(titles) {
        let _ = std::fs::write(segment_dir.join(AUDIO_TRACK_TITLES_FILE), json);
    }
}

/// Reads back titles written by `write_audio_track_titles`. Missing/corrupt
/// (e.g. a pre-upgrade leftover buffer dir) just means no relabeling — the
/// concat still succeeds, tracks are just untitled like before this existed.
fn read_audio_track_titles(dir: &std::path::Path) -> Vec<String> {
    std::fs::read(dir.join(AUDIO_TRACK_TITLES_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Appends fresh `-metadata:s:a:N title=...` for each track onto a concat
/// command. As an output-side option this attaches regardless of what the
/// (possibly title-less, see `write_audio_track_titles`) input segments carry.
fn push_audio_title_metadata(args: &mut Vec<String>, titles: &[String]) {
    for (i, title) in titles.iter().enumerate() {
        args.push(format!("-metadata:s:a:{i}"));
        args.push(format!("title={title}"));
    }
}

const GAME_NAME_FILE: &str = "game.txt";

/// Persists the buffer's target game name alongside its segments — same
/// rationale and lifetime as `write_audio_track_titles` (survives the
/// pending-clip/crash-recovery rename and a restart, since those saves have
/// no live `ReplayBufferManager` state to ask). Desktop-targeted buffers
/// write nothing; `read_buffer_game_name` then correctly reports "no game".
fn write_buffer_game_name(segment_dir: &std::path::Path, game_app: &str) {
    let _ = std::fs::write(segment_dir.join(GAME_NAME_FILE), game_app);
}

/// Reads back the name written by `write_buffer_game_name`. Missing (desktop
/// buffer, or a pre-upgrade leftover dir) just means the clip saves to the
/// recordings root, same as before this existed.
fn read_buffer_game_name(dir: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(dir.join(GAME_NAME_FILE))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Resolves the directory a replay-buffer clip should save into: the
/// recordings root, or — when the buffer targeted a known game — that game's
/// subfolder (optionally nested one level deeper in the game's configured
/// folder override), exactly like a full recording session
/// (`recording::prepare`). Every save path (live, pending-clip, crash
/// recovery) funnels through here so clips land next to full recordings of
/// the same game instead of always falling back to the recordings root.
fn resolve_clip_dir(app: &AppHandle, settings: &crate::config::Settings, game: Option<&str>) -> PathBuf {
    let mut dir = settings.resolved_recordings_dir();
    let Some(name) = game else { return dir };
    dir = dir.join(crate::drive::sanitize_filename(name));
    let icons = app.state::<Arc<crate::icon_cache::IconCache>>();
    if let Some(bytes) = crate::games_db::best_icon_bytes(app, &icons, name) {
        let _ = std::fs::create_dir_all(&dir);
        crate::folder_icon::ensure_folder_icon(&dir, &bytes);
    }
    if let Some(ov) = app.state::<Arc<crate::games_db::GamesDb>>().overrides_for(name) {
        if let Some(folder) = ov.folder_id.as_deref().and_then(|id| settings.folder_by_id(id)) {
            dir = dir.join(&folder.name);
        }
    }
    dir
}

/// Whether disk segments are AV1. `segment_container_for` uses fragmented MP4
/// (`.mp4`) only for AV1 and MPEG-TS (`.ts`) for H.264/HEVC, so the extension
/// is a reliable codec tell — including in crash recovery, where the active
/// buffer state (and its resolved encoder) no longer exists.
fn segments_are_av1(segments: &[PathBuf]) -> bool {
    segments
        .first()
        .and_then(|p| p.extension())
        .and_then(|x| x.to_str())
        == Some("mp4")
}

/// Disk-mode segment files currently on disk, oldest first — includes the
/// newest one, which is still open for writing; the caller must drop it.
fn list_segment_files(segment_dir: &std::path::Path) -> Result<Vec<PathBuf>, String> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(segment_dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        // A session's segments are all one container (.ts for H.264/HEVC,
        // .mp4 for AV1), but crash recovery reads whatever's on disk.
        .filter(|p| matches!(p.extension().and_then(|x| x.to_str()), Some("ts") | Some("mp4")))
        .collect();
    entries.sort();
    Ok(entries)
}

/// Writes the buffered window out as a clip. Disk mode concatenates the
/// closed segment files (the still-open newest one is excluded). Memory
/// mode remuxes the in-RAM fMP4 ring from a temp file. Neither re-encodes.
pub fn save_replay(app: &AppHandle) -> Result<PathBuf, String> {
    let manager = app.state::<Arc<ReplayBufferManager>>();
    let (segment_dir, buffer_minutes, mem_ring, is_av1) = {
        let guard = manager.active.lock().unwrap();
        let active = guard.as_ref().ok_or("Replay buffer is not running")?;
        // Both storage modes now carry AV1 (disk via fMP4 segments, memory via
        // the fMP4 ring), so this keys purely off the encoder.
        let is_av1 = active.encoder.is_av1();
        (active.segment_dir.clone(), active.buffer_minutes, active.mem_ring.clone(), is_av1)
    };

    let store = app.state::<Arc<ConfigStore>>();
    let settings = store.get();
    // Per-game container override, when the buffer targets a known game.
    let mut container = settings.video.container.clone();
    let mut per_game_buffer_container = None;
    let game = match manager.current_target() {
        Some(ReplayBufferTarget::SpecificWindow { app: game_app, .. }) => Some(game_app),
        _ => None,
    };
    if let Some(game_app) = &game {
        if let Some(ov) = app.state::<Arc<crate::games_db::GamesDb>>().overrides_for(game_app) {
            if let Some(c) = &ov.container {
                container = c.clone();
            }
            per_game_buffer_container = ov.replay_buffer_video.container;
        }
    }
    let dir = resolve_clip_dir(app, &settings, game.as_deref());
    // Buffer-specific overrides win last, same order as at buffer start:
    // global buffer override, then (most specific) this game's own.
    if settings.video.replay_buffer.use_custom_video {
        if let Some(c) = &settings.video.replay_buffer.video_override.container {
            container = c.clone();
        }
    }
    if let Some(c) = per_game_buffer_container {
        container = c;
    }
    // MOV can't hold AV1 — fall back to the MP4 equivalent so the save doesn't
    // fail with an empty file.
    let adjusted = container.compatible_with_av1(is_av1);
    if adjusted != container {
        log::warn!("MOV can't carry AV1; saving replay clip as {} instead", adjusted.extension());
        container = adjusted;
    }
    let output_path = crate::recording::make_video_save_path(&dir, container.extension(), "Clip_").map_err(|e| e.to_string())?;
    let sidecar = crate::integrity::ffmpeg_sidecar(app).map_err(|e| e.to_string())?;

    if let Some(ring) = mem_ring {
        // Memory mode
        let window = Duration::from_secs(buffer_minutes as u64 * 60);
        // `init`/`frags` here are a cheap clone (small buffer + bumped Arc
        // refcounts, see `snapshot_refs`'s doc comment) — the lock is held
        // only for that, not for the disk write below.
        let (init, frags) = {
            let r = ring.lock().unwrap();
            // Each fragment is one ~2s GOP; need at least a couple before
            // there's anything worth saving (and one keyframe to start from).
            if r.fragment_count(window) < 2 {
                return Err("Not enough buffered footage yet".into());
            }
            r.snapshot_refs(window)
        };
        std::fs::create_dir_all(&segment_dir).map_err(|e| e.to_string())?;
        // Already a valid fragmented MP4 (init segment + keyframe fragments),
        // so ffmpeg reads it directly and the stream copy needs no bitstream
        // filter. Non-zero fragment start times are normalized to 0 by
        // ffmpeg's default (non-`-copyts`) timestamp handling.
        //
        // Written straight from the ring's own (Arc-shared) fragment buffers
        // instead of first concatenating them into one big `Vec<u8>` — avoids
        // ever holding a second full copy of the buffered window in RAM on
        // top of the ring's own steady-state footprint (which, at high
        // bitrate and a long window, is already the dominant cost of memory
        // mode).
        let temp_mp4 = segment_dir.join("mem_buffer.mp4");
        {
            use std::io::Write;
            let mut file = std::fs::File::create(&temp_mp4).map_err(|e| e.to_string())?;
            file.write_all(&init).map_err(|e| e.to_string())?;
            for frag in &frags {
                file.write_all(frag).map_err(|e| e.to_string())?;
            }
        }
        drop(frags);

        let mut mem_args: Vec<String> = vec![
            "-y".into(),
            "-i".into(), temp_mp4.to_string_lossy().into_owned(),
            // -map 0 keeps every stream — default selection would
            // silently drop all but one audio track.
            "-map".into(), "0".into(), "-c".into(), "copy".into(),
        ];
        push_audio_title_metadata(&mut mem_args, &read_audio_track_titles(&segment_dir));
        if let Some(flags) = container.movflags() {
            mem_args.push("-movflags".into());
            mem_args.push(flags.into());
        }
        mem_args.push(output_path.to_string_lossy().into_owned());
        let output = tauri::async_runtime::block_on(sidecar.args(mem_args).output())
            .map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(&temp_mp4);
        if !output.status.success() {
            return Err(format!("ffmpeg remux failed: {}", String::from_utf8_lossy(&output.stderr)));
        }
    } else {
        // Disk mode
        let mut entries = list_segment_files(&segment_dir)?;

        if entries.len() < 2 {
            // Not enough closed segments yet — normal right after the buffer
            // just started. Wait for the next rotation and retry instead of
            // failing outright, so every caller gets a "loading" state for free.
            crate::toast::show(app, "info", crate::toast::ToastCategory::Clip, "Saving clip…", "Not enough buffered footage yet — saving automatically once the next segment is ready");
            let deadline = std::time::Instant::now() + Duration::from_secs(SEGMENT_SECONDS as u64 * 4);
            loop {
                std::thread::sleep(Duration::from_secs(1));
                entries = list_segment_files(&segment_dir)?;
                if entries.len() >= 2 {
                    break;
                }
                if std::time::Instant::now() >= deadline {
                    return Err("Not enough buffered footage yet".into());
                }
            }
        }
        // Drop the newest segment — it's the one ffmpeg currently has open.
        entries.pop();

        let max_segments = ((buffer_minutes as u64 * 60) / SEGMENT_SECONDS as u64).max(1) as usize;
        let skip = entries.len().saturating_sub(max_segments);
        let chosen: Vec<PathBuf> = entries[skip..].to_vec();
        if chosen.is_empty() {
            return Err("Not enough buffered footage yet".into());
        }

        let list_path = segment_dir.join("concat_list.txt");
        let list_body = chosen
            .iter()
            .map(|p| format!("file '{}'", p.to_string_lossy().replace('\'', "'\\''")))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&list_path, list_body).map_err(|e| e.to_string())?;

        let mut disk_args: Vec<String> = vec![
            "-y".into(), "-f".into(), "concat".into(), "-safe".into(), "0".into(),
            "-i".into(), list_path.to_string_lossy().into_owned(),
            // -map 0 keeps every stream — default selection would
            // silently drop all but one audio track.
            "-map".into(), "0".into(), "-c".into(), "copy".into(),
        ];
        push_segment_audio_bsf(&mut disk_args, &chosen);
        push_audio_title_metadata(&mut disk_args, &read_audio_track_titles(&segment_dir));
        if let Some(flags) = container.movflags() {
            disk_args.push("-movflags".into());
            disk_args.push(flags.into());
        }
        disk_args.push(output_path.to_string_lossy().into_owned());
        let output = tauri::async_runtime::block_on(sidecar.args(disk_args).output())
            .map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(&list_path);
        if !output.status.success() {
            return Err(format!("ffmpeg concat failed: {}", String::from_utf8_lossy(&output.stderr)));
        }
    }

    let file_name = output_path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    // Same event regular recordings emit — the gallery reloads its grid on it.
    let _ = app.emit("video-saved", serde_json::json!({
        "path": output_path.to_string_lossy(),
        "name": file_name,
    }));
    match &game {
        Some(game_app) => {
            crate::toast::show_for_game(app, "info", crate::toast::ToastCategory::Clip, "Clip saved", game_app, game_app);
        }
        None => {
            crate::toast::show(app, "info", crate::toast::ToastCategory::Clip, "Clip saved", file_name);
        }
    }
    crate::sound::play(&settings.sound_effects.clip_saved);

    // Stamp mtime to "now" — concat -c copy can preserve the first
    // segment's original mtime, and the gallery sorts by modified time.
    let _ = filetime::set_file_mtime(&output_path, filetime::FileTime::now());

    // Every replay save is a "clip" — the gallery's Videos/Clips filter runs
    // on this. Stamp the game's app when the buffer is game-targeted (not
    // its raw window title — see `recording::mod`'s `prepare()` for why).
    let video_meta = crate::meta::VideoMeta {
        created: Some(chrono::Utc::now().timestamp()),
        kind: Some("clip".to_string()),
        stream_info: None,
        app: game,
        ..Default::default()
    };
    app.state::<Arc<crate::meta::MetaStore>>().set(file_name.to_string(), video_meta);

    let local_now = Local::now();
    log::info!("replay saved at {local_now}");
    Ok(output_path)
}

// Disk-mode crash recovery: stages leftover segments aside so the user can
// choose to recover or discard, rather than silently auto-saving footage
// they never asked to keep. Memory mode has nothing to recover from RAM.

fn replay_crash_staging_dir(app: &AppHandle) -> PathBuf {
    app.path().temp_dir().unwrap_or_else(|_| std::env::temp_dir()).join("dev.xacnio.capcove").join("replay_segments_crash")
}

#[derive(Default)]
pub struct ReplayCrashRecoveryState(Mutex<Option<serde_json::Value>>);

impl ReplayCrashRecoveryState {
    pub fn take(&self) -> Option<serde_json::Value> {
        self.0.lock().unwrap().take()
    }
}

fn report_replay_crash_recovery(app: &AppHandle, segment_count: usize) {
    let payload = serde_json::json!({ "segment_count": segment_count });
    if let Some(state) = app.try_state::<Arc<ReplayCrashRecoveryState>>() {
        *state.0.lock().unwrap() = Some(payload.clone());
    }
    let _ = app.emit("replay-crash-recovery", payload);
}

/// Checked once at startup, before auto-start wipes the segment directory.
/// A directory still present means the app crashed while the buffer was
/// active; moves the leftovers to staging and reports how many are waiting.
pub fn stage_replay_buffer_crash_recovery(app: &AppHandle) {
    let segment_dir = segment_dir_for(app);
    let Ok(entries) = list_segment_files(&segment_dir) else { return };
    if entries.len() < 2 {
        let _ = std::fs::remove_dir_all(&segment_dir);
        return;
    }
    let staging = replay_crash_staging_dir(app);
    let _ = std::fs::remove_dir_all(&staging); // clear out any previous, never-resolved staging
    if let Some(parent) = staging.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::rename(&segment_dir, &staging).is_err() {
        log::warn!("crash recovery: could not stage leftover replay segments (different filesystem?) — leaving them in place");
        return;
    }
    // The newest is excluded — see `recover_replay_buffer_crash`.
    log::info!("crash recovery: staged {} leftover replay segment(s) for recovery", entries.len());
    report_replay_crash_recovery(app, entries.len().saturating_sub(1));
}

/// Concatenates the staged segments into a normal clip, same `-map 0 -c
/// copy` approach as `save_replay`'s disk-mode branch. Drops the last
/// segment — it was likely still being written when the crash happened.
pub async fn recover_replay_buffer_crash(app: &AppHandle) -> Result<PathBuf, String> {
    let staging = replay_crash_staging_dir(app);
    let mut entries = list_segment_files(&staging)?;
    if entries.len() < 2 {
        let _ = std::fs::remove_dir_all(&staging);
        return Err("Not enough buffered footage to recover".into());
    }
    entries.pop();

    let store = app.state::<Arc<ConfigStore>>();
    let settings = store.get();
    let game = read_buffer_game_name(&staging);
    let dir = resolve_clip_dir(app, &settings, game.as_deref());
    let mut container = settings.video.container.clone();
    if settings.video.replay_buffer.use_custom_video {
        if let Some(c) = &settings.video.replay_buffer.video_override.container {
            container = c.clone();
        }
    }
    // MOV can't hold AV1 — fall back to the MP4 equivalent so the concat
    // doesn't fail with an empty file.
    let adjusted = container.compatible_with_av1(segments_are_av1(&entries));
    if adjusted != container {
        log::warn!("MOV can't carry AV1; saving replay clip as {} instead", adjusted.extension());
        container = adjusted;
    }
    let output_path = crate::recording::make_video_save_path(&dir, container.extension(), "Clip_").map_err(|e| e.to_string())?;

    let list_path = staging.join("concat_list.txt");
    let list_body = entries.iter()
        .map(|p| format!("file '{}'", p.to_string_lossy().replace('\'', "'\\''")))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&list_path, list_body).map_err(|e| e.to_string())?;

    let mut args: Vec<String> = vec![
        "-y".into(), "-f".into(), "concat".into(), "-safe".into(), "0".into(),
        "-i".into(), list_path.to_string_lossy().into_owned(),
        // -map 0 keeps every stream — default selection would
        // silently drop all but one audio track.
        "-map".into(), "0".into(), "-c".into(), "copy".into(),
    ];
    push_segment_audio_bsf(&mut args, &entries);
    push_audio_title_metadata(&mut args, &read_audio_track_titles(&staging));
    if let Some(flags) = container.movflags() {
        args.push("-movflags".into());
        args.push(flags.into());
    }
    args.push(output_path.to_string_lossy().into_owned());

    let sidecar = crate::integrity::ffmpeg_sidecar(app).map_err(|e| e.to_string())?;
    let output = sidecar.args(args).output().await.map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&staging);
    if !output.status.success() {
        return Err(format!("ffmpeg concat failed: {}", String::from_utf8_lossy(&output.stderr)));
    }

    let file_name = output_path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let _ = filetime::set_file_mtime(&output_path, filetime::FileTime::now());
    app.state::<Arc<crate::meta::MetaStore>>().set(file_name.to_string(), crate::meta::VideoMeta {
        created: Some(chrono::Utc::now().timestamp()),
        kind: Some("clip".to_string()),
        stream_info: None,
        app: game,
        ..Default::default()
    });
    let _ = app.emit("video-saved", serde_json::json!({
        "path": output_path.to_string_lossy(),
        "name": file_name,
    }));
    log::info!("crash recovery: recovered replay buffer footage as {file_name}");
    Ok(output_path)
}

/// The user chose not to keep the leftover buffered footage.
pub fn discard_replay_buffer_crash(app: &AppHandle) -> Result<(), String> {
    let staging = replay_crash_staging_dir(app);
    std::fs::remove_dir_all(&staging).map_err(|e| e.to_string()).or_else(|e| {
        if staging.exists() { Err(e) } else { Ok(()) }
    })
}
