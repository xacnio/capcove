use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// Shortcut system
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ShortcutCapture {
    /// Opens the window-picker overlay; the pick starts a recording of that window.
    #[default]
    RecordWindow,
    /// Opens the area-picker overlay; the dragged rectangle starts a recording.
    RecordArea,
    /// Starts recording the primary monitor immediately — no picker overlay.
    RecordMonitor,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ShortcutAction {
    /// Saves the last `replay_buffer.buffer_minutes` of the running replay
    /// buffer to a file. Slots using this action ignore `capture` entirely —
    /// wired in `tray::register_hotkeys` before the normal capture dispatch.
    SaveReplay,
    /// Opens the on-screen radial shortcut wheel with all capture actions.
    /// Like `SaveReplay`, ignores `capture`.
    OpenWheel,
}

// ---------------------------------------------------------------------------
// Video recording settings
// ---------------------------------------------------------------------------

/// Hardware/software encoder to use for recording. `Auto` resolves to the
/// best available option at recording-start time (NVENC > AMF > software),
/// not at settings-save time, so it always reflects current hardware state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum EncoderChoice {
    #[default]
    Auto,
    NvencH264,
    NvencHevc,
    NvencAv1,
    AmfH264,
    AmfHevc,
    AmfAv1,
    QsvH264,
    QsvHevc,
    QsvAv1,
    X264Software,
    X265Software,
    SvtAv1,
    AomAv1,
}

impl EncoderChoice {
    /// True for the AV1 encoders — they can't ride in MPEG-TS (no mapping),
    /// which the memory replay buffer needs to know.
    pub fn is_av1(&self) -> bool {
        matches!(self, Self::NvencAv1 | Self::AmfAv1 | Self::QsvAv1 | Self::SvtAv1 | Self::AomAv1)
    }
}

/// Rate-control mode for the local recording/replay-buffer encode (the live
/// stream is unaffected — it always forces its own strict CBR via
/// `live_cbr_args`, since RTMP/YouTube need a steady feed regardless of this
/// setting).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RateControl {
    /// Spends the full configured bitrate on every frame, including
    /// static/simple scenes that don't need it — wasted bits for local files,
    /// where (unlike streaming) nothing needs a steady feed.
    #[default]
    Cbr,
    /// Treats the bitrate as an average/cap: less on simple content, more on
    /// complex motion, so recordings usually end up smaller (or better
    /// quality at the same size) — at the cost of the exact byte size no
    /// longer being reliably predictable up front.
    Vbr,
    /// Constant QP/CRF: `VideoSettings::quality` alone drives every frame's
    /// quality, with no bitrate target at all — the file lands wherever that
    /// quality actually costs for the content. Closest to what a video
    /// editor thinks of as "quality", but completely unpredictable in size.
    Cqp,
    /// Bitrate as a cap, `VideoSettings::quality` as the actual driver within
    /// it (NVENC's native "VBR + target quality"/ICQ; approximated on other
    /// vendors as a capped-CRF-style encode) — a middle ground between VBR
    /// and CQP: quality-led, but with a ceiling so a spike can't balloon.
    VbrCq,
    /// True lossless — enormous files (can run into hundreds of Mbps).
    /// `quality` is ignored; there's nothing to tune once nothing is thrown away.
    Lossless,
}

/// Output resolution cap. `Native` records at capture size; the P* options
/// downscale (never upscale) to that height, keeping aspect ratio.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RecordingResolution {
    #[default]
    Native,
    P2160,
    P1440,
    P1080,
    P720,
    P480,
}

impl RecordingResolution {
    /// Target height, or None for native.
    pub fn height(&self) -> Option<u32> {
        match self {
            Self::Native => None,
            Self::P2160 => Some(2160),
            Self::P1440 => Some(1440),
            Self::P1080 => Some(1080),
            Self::P720 => Some(720),
            Self::P480 => Some(480),
        }
    }
}

/// Audio codec for recorded tracks. AAC is the safe default everywhere;
/// FLAC is lossless (bigger files, MKV recommended).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AudioCodec {
    #[default]
    Aac,
    Opus,
    Mp3,
    Flac,
}

impl AudioCodec {
    /// (ffmpeg encoder name, bitrate arg or None for lossless).
    pub fn ffmpeg_args(&self) -> (&'static str, Option<&'static str>) {
        match self {
            Self::Aac => ("aac", Some("192k")),
            Self::Opus => ("libopus", Some("160k")),
            Self::Mp3 => ("libmp3lame", Some("192k")),
            Self::Flac => ("flac", None),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum VideoContainer {
    Mp4,
    Mkv,
    Mov,
    /// Same `.mp4` file as `Mp4`, muxed as fragmented ISO-BMFF — a crash mid-
    /// recording leaves it playable up to the last complete chunk, same as
    /// `Mkv`. Exists because MKV isn't accepted everywhere MP4 is. Default:
    /// gives every recording MKV's crash-safety while staying a plain `.mp4`.
    #[default]
    Mp4Fragmented,
    /// Same idea as `Mp4Fragmented`, `.mov` extension instead.
    MovFragmented,
}

impl VideoContainer {
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Mp4 | Self::Mp4Fragmented => "mp4",
            Self::Mkv => "mkv",
            Self::Mov | Self::MovFragmented => "mov",
        }
    }

    /// AV1 can't be muxed into QuickTime (MOV) — ffmpeg's `mov` muxer has no
    /// AV1 sample entry and fails with "incorrect codec parameters", producing
    /// an empty file. When the codec is AV1, swap a MOV-family choice for the
    /// matching MP4-family one (same fragmented-ness); MP4 and MKV both carry
    /// AV1, so they're left alone.
    pub fn compatible_with_av1(&self, is_av1: bool) -> Self {
        if !is_av1 {
            return self.clone();
        }
        match self {
            Self::Mov => Self::Mp4,
            Self::MovFragmented => Self::Mp4Fragmented,
            other => other.clone(),
        }
    }

    /// The `-movflags` value ffmpeg needs for this container, if any. MKV
    /// needs none; plain MP4/MOV need `+faststart` (only safe on a clean
    /// finish); fragmented variants need no finalization step at all.
    pub fn movflags(&self) -> Option<&'static str> {
        match self {
            Self::Mp4 | Self::Mov => Some("+faststart"),
            Self::Mp4Fragmented | Self::MovFragmented => Some("+frag_keyframe+empty_moov+default_base_moof"),
            Self::Mkv => None,
        }
    }
}

/// One audio source to record; each selected source becomes its own output track (never mixed down).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum AudioSource {
    SystemOutput {
        device_id: String,
        label: String,
        /// Kept in the list but skipped at capture start when false —
        /// lets extra devices be turned off without losing the selection.
        #[serde(default = "default_true")]
        enabled: bool,
        /// Part of the first ("Mix") audio track — what YouTube (live and
        /// uploads) and most players play. Sources with this off still get
        /// their own separate track, they just stay out of the mix.
        #[serde(default = "default_true")]
        main_mix: bool,
    },
    Microphone {
        device_id: String,
        label: String,
        #[serde(default = "default_true")]
        enabled: bool,
        #[serde(default = "default_true")]
        main_mix: bool,
    },
    /// One application's audio in isolation, identified by exe stem (pids don't survive restarts)
    /// and resolved to a live audio-session pid at recording start; skipped if not running.
    Application {
        exe: String,
        label: String,
        #[serde(default = "default_true")]
        enabled: bool,
        #[serde(default = "default_true")]
        main_mix: bool,
    },
}

impl AudioSource {
    pub fn is_enabled(&self) -> bool {
        match self {
            AudioSource::SystemOutput { enabled, .. }
            | AudioSource::Microphone { enabled, .. }
            | AudioSource::Application { enabled, .. } => *enabled,
        }
    }

    pub fn in_main_mix(&self) -> bool {
        match self {
            AudioSource::SystemOutput { main_mix, .. }
            | AudioSource::Microphone { main_mix, .. }
            | AudioSource::Application { main_mix, .. } => *main_mix,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AudioConfig {
    pub sources: Vec<AudioSource>,
    /// Quick mutes (flippable from the shortcut wheel): skip the matching
    /// source kind at recording start without touching the device selection.
    pub system_muted: bool,
    pub mic_muted: bool,
    /// Master multi-track switch: ON gives each source (plus a dedicated "Game" track) its own
    /// output track with track 1 as the curated "Mix"; OFF mixes everything into one track.
    #[serde(default = "default_true", alias = "game_track")]
    pub separate_tracks: bool,
    /// Whether the dedicated Game track participates in the first ("Mix")
    /// audio track — same knob every regular source has via
    /// `AudioSource::main_mix`. Separate-tracks mode only.
    #[serde(default = "default_true")]
    pub game_track_main_mix: bool,
    /// Single-track mode only, game sessions only: which side of the mix
    /// gets the volume edge — the microphone (game/system ducked) or the
    /// game (mic ducked). `Balanced` applies no weighting.
    #[serde(default)]
    pub mix_priority: MixPriority,
    /// Game sessions only: skip System Audio and take the game's sound directly from its process.
    /// Needed for virtual-device setups where System Audio shares the game's endpoint and would
    /// otherwise duplicate it onto two tracks.
    #[serde(default)]
    pub game_audio_only: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MixPriority {
    #[default]
    Balanced,
    Input,
    Game,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            // Record the default system output out of the box (empty
            // device_id = OS default; empty label = "System Audio").
            sources: vec![AudioSource::SystemOutput {
                device_id: String::new(),
                label: String::new(),
                enabled: true,
                main_mix: true,
            }],
            system_muted: false,
            mic_muted: false,
            separate_tracks: true,
            game_track_main_mix: true,
            mix_priority: MixPriority::default(),
            game_audio_only: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ReplayBufferTarget {
    #[default]
    PrimaryMonitor,
    SpecificWindow { hwnd: u32, title: String, app: String },
}

/// What happens automatically when a fullscreen game is detected in the foreground.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum GameDetectMode {
    /// Start the replay buffer targeting the game so clip hotkeys work;
    /// stop it when the game closes.
    Clips,
    /// Start a full recording of the game window; stop (and save) when the
    /// game closes.
    FullSession,
    /// Do nothing automatically. Default — recording a window auto-registers
    /// it as a custom game (see `recording::finish_starting`), and an opt-out
    /// default keeps that from silently turning every recorded app into an
    /// auto-triggered target the moment it's added.
    #[default]
    Off,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ReplayBufferSettings {
    /// Always-on desktop buffer from app launch ("also record desktop") —
    /// game-detected buffering works regardless of this.
    pub enabled: bool,
    pub buffer_minutes: u32,
    pub target: ReplayBufferTarget,
    /// Automatic behavior on fullscreen-game detection. Independent of
    /// `enabled`, which means "buffer always on from app launch".
    pub game_detect_mode: GameDetectMode,
    /// Opt-in privacy: while a window-targeted capture's window is alt-tabbed away, write an
    /// "Alt-tabbed" card into the stream instead of the window's actual (still-rendering) content.
    pub alt_tab_privacy: bool,
    /// Where buffered footage lives while the buffer runs: rotating segment files on disk
    /// (default), or an in-RAM ring of the encoded stream (no disk writes until "Save Clip").
    pub storage: ReplayBufferStorage,
    /// Full Session mode extra: also stream the session to YouTube as a
    /// private live broadcast over RTMP (needs a connected Google account,
    /// an H.264 encoder, and AAC audio).
    pub full_session_youtube_live: bool,
    /// Clips mode only: when the detected game closes, ask before discarding
    /// the buffer instead of silently dropping it — see
    /// `replay_buffer::stop_replay_buffer_for_pending_save`. Full Session
    /// always just stops (it's already a real saved file, nothing to lose).
    #[serde(default = "default_true")]
    pub confirm_save_on_close: bool,
    /// When on, `video_override` replaces the matching fields of the global
    /// video settings (after any per-game overrides) for the buffer's own
    /// encode only — full recordings are untouched. Lets the buffer run
    /// lighter (or heavier) than full recordings, e.g. to keep it from
    /// falling behind under real game load.
    #[serde(default)]
    pub use_custom_video: bool,
    #[serde(default)]
    pub video_override: ReplayBufferVideoOverride,
}

impl Default for ReplayBufferSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            buffer_minutes: 5,
            target: ReplayBufferTarget::default(),
            game_detect_mode: GameDetectMode::default(),
            alt_tab_privacy: false,
            storage: ReplayBufferStorage::default(),
            full_session_youtube_live: false,
            confirm_save_on_close: true,
            use_custom_video: false,
            video_override: ReplayBufferVideoOverride::default(),
        }
    }
}

/// The replay buffer's own video-quality override — every field optional;
/// `None` means "use the global video setting" (after any per-game
/// override). Same shape as `GameOverrides`' video fields, minus the
/// fields that don't apply to an encode (mode/folder/youtube).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ReplayBufferVideoOverride {
    pub fps: Option<u32>,
    pub bitrate_kbps: Option<u32>,
    pub encoder: Option<EncoderChoice>,
    #[serde(default)]
    pub rate_control: Option<RateControl>,
    #[serde(default)]
    pub quality: Option<u32>,
    pub container: Option<VideoContainer>,
    pub audio_codec: Option<AudioCodec>,
    pub resolution: Option<RecordingResolution>,
}

impl ReplayBufferVideoOverride {
    /// Overlays the set fields onto a copy of the (possibly already
    /// per-game-overridden) video settings the buffer is about to start with.
    pub fn apply_to(&self, video: &mut VideoSettings) {
        if let Some(v) = self.fps {
            video.fps = v;
        }
        if let Some(v) = self.bitrate_kbps {
            video.bitrate_kbps = v;
        }
        if let Some(v) = &self.encoder {
            video.encoder = v.clone();
        }
        if let Some(v) = self.rate_control {
            video.rate_control = v;
        }
        if let Some(v) = self.quality {
            video.quality = v;
        }
        if let Some(v) = &self.container {
            video.container = v.clone();
        }
        if let Some(v) = self.audio_codec {
            video.audio_codec = v;
        }
        if let Some(v) = self.resolution {
            video.resolution = v;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplayBufferStorage {
    #[default]
    Disk,
    Memory,
}

/// A short synthesized tone (see `sound.rs`) — no bundled audio assets
/// needed, and every install hears the exact same, licensing-free sound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SoundPreset {
    SoftPing,
    Marimba,
    Glass,
    #[default]
    SoftBell,
    TwoToneUp,
    TwoToneDown,
    Coin,
    Success,
    Alert,
}

/// Either one of the built-in presets, or a user-picked audio file (wav/mp3/ogg/flac).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SoundSource {
    Preset { preset: SoundPreset },
    Custom { path: String },
}

impl Default for SoundSource {
    fn default() -> Self {
        SoundSource::Preset { preset: SoundPreset::default() }
    }
}

/// One event's sound-effect config — independently toggleable, since not
/// every user wants a sound for every state change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SoundEffectSetting {
    pub enabled: bool,
    pub source: SoundSource,
}

impl Default for SoundEffectSetting {
    fn default() -> Self {
        Self { enabled: true, source: SoundSource::default() }
    }
}

/// Sound effects for recording/replay-buffer state changes (Alt+F2 wheel and
/// every other trigger of the same action) — see `sound::play`. On by
/// default, each with a preset tone picked to match its event; still
/// independently toggleable per event from Settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SoundEffectsSettings {
    pub recording_started: SoundEffectSetting,
    pub recording_stopped: SoundEffectSetting,
    pub buffer_started: SoundEffectSetting,
    pub buffer_stopped: SoundEffectSetting,
    pub clip_saved: SoundEffectSetting,
}

impl Default for SoundEffectsSettings {
    fn default() -> Self {
        Self {
            // Rising two-note tone for "started", the mirrored falling one for "stopped".
            recording_started: SoundEffectSetting { source: SoundSource::Preset { preset: SoundPreset::TwoToneUp }, ..Default::default() },
            recording_stopped: SoundEffectSetting { source: SoundSource::Preset { preset: SoundPreset::TwoToneDown }, ..Default::default() },
            // Light, quick ping for the (unobtrusive, background) buffer turning
            // on; a duller percussive knock for it turning back off.
            buffer_started: SoundEffectSetting { source: SoundSource::Preset { preset: SoundPreset::SoftPing }, ..Default::default() },
            buffer_stopped: SoundEffectSetting { source: SoundSource::Preset { preset: SoundPreset::Marimba }, ..Default::default() },
            // Rising major-triad arpeggio — the universal "task completed" sound.
            clip_saved: SoundEffectSetting { source: SoundSource::Preset { preset: SoundPreset::Success }, ..Default::default() },
        }
    }
}

/// Shared by every YouTube live-stream start — the title template and the
/// created broadcast's visibility are account-wide preferences.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct YoutubeLiveSettings {
    /// `{game}`/`{date}`/`{time}`/`{datetime}` tokens substituted at stream-start (see
    /// `recording::render_title_template`); `{game}` falls back to "Capcove" if none started it.
    pub title_template: String,
    /// YouTube `privacyStatus` for created broadcasts: "private" (default —
    /// only you can watch it), "unlisted" (anyone with the link), "public".
    pub privacy: String,
    /// Resolution/bitrate ceiling for the live RTMP feed only — kept separate from the local
    /// recording since YouTube's ingest enforces a hard per-resolution bitrate ceiling.
    pub max_resolution: RecordingResolution,
    pub max_bitrate_kbps: u32,
    /// Framerate ceiling for the live feed — YouTube's ingest tops out at 60fps regardless of
    /// the local recording's fps (up to 144).
    pub max_fps: u32,
    /// Seconds between forced keyframes on the live feed only (YouTube recommends 2s); has no
    /// equivalent for the local recording.
    pub keyframe_interval_secs: u32,
    /// Audio codec for the live feed only — YouTube's ingest only lists AAC
    /// or MP3 (never Opus/FLAC, which the local recording can use).
    pub audio_codec: AudioCodec,
    pub audio_sample_rate: u32,
    /// VBV buffer size for the live feed's CBR encode, in seconds of target
    /// bitrate — bigger smooths quality but adds encode-side latency.
    pub cbr_buffer_secs: f32,
}

impl Default for YoutubeLiveSettings {
    fn default() -> Self {
        Self {
            title_template: "{game} — {date} {time}".to_string(),
            privacy: "private".to_string(),
            max_resolution: RecordingResolution::P1080,
            max_bitrate_kbps: 8000,
            max_fps: 60,
            keyframe_interval_secs: 2,
            audio_codec: AudioCodec::Aac,
            audio_sample_rate: 48000,
            cbr_buffer_secs: 1.0,
        }
    }
}

/// Per-game overrides (settings → Games → expanded row): every field is
/// optional; `None` means "use the global setting". Applied wherever a
/// capture starts for a detected/known game.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct GameOverrides {
    pub game_detect_mode: Option<GameDetectMode>,
    pub fps: Option<u32>,
    pub bitrate_kbps: Option<u32>,
    pub encoder: Option<EncoderChoice>,
    pub container: Option<VideoContainer>,
    pub audio_codec: Option<AudioCodec>,
    pub resolution: Option<RecordingResolution>,
    /// Overrides `full_session_youtube_live` for this game (Full Session
    /// mode's YouTube streaming) — checked by the detection loop directly.
    pub youtube_live: Option<bool>,
    /// This game's default recording folder (`RecordingFolder::id`); `None` (or a stale id)
    /// falls back to the recordings root.
    pub folder_id: Option<String>,
    /// This game's own override for the replay buffer's encode specifically
    /// — wins over both the fields above and the global replay-buffer
    /// override (`ReplayBufferSettings.video_override`) when the buffer is
    /// targeting this game, since it's the most specific of the three.
    #[serde(default)]
    pub replay_buffer_video: ReplayBufferVideoOverride,
}

impl GameOverrides {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Overlays the set fields onto a copy of the global video settings.
    pub fn apply_to(&self, video: &mut VideoSettings) {
        if let Some(v) = self.fps {
            video.fps = v;
        }
        if let Some(v) = self.bitrate_kbps {
            video.bitrate_kbps = v;
        }
        if let Some(v) = &self.encoder {
            video.encoder = v.clone();
        }
        if let Some(v) = &self.container {
            video.container = v.clone();
        }
        if let Some(v) = self.audio_codec {
            video.audio_codec = v;
        }
        if let Some(v) = self.resolution {
            video.resolution = v;
        }
    }
}

/// A named recording destination — a real subdirectory under the recordings
/// root, not just a database tag. Referenced elsewhere by `id` (stable
/// across a rename); the physical subdirectory renames alongside `name`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecordingFolder {
    pub id: String,
    pub name: String,
    /// `None` = a global folder, selectable for any game. `Some(game)` scopes
    /// it to that game and nests it under the game's subdirectory instead of
    /// sitting at the recordings root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game: Option<String>,
    /// Auto-delete recordings in this folder once older than this many days, independent of
    /// the global storage-limit cleanup (`Settings::auto_delete_oldest`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_delete_days: Option<u32>,
    /// Never sync this folder's recordings to Google Drive.
    #[serde(default)]
    pub never_upload_to_drive: bool,
    /// Exempt every recording in this folder from all auto-delete paths (global storage-limit
    /// cleanup and this folder's own `auto_delete_days`); wins if both are set.
    #[serde(default)]
    pub always_keep: bool,
}

/// What the encoder writes while the recorded window is minimized (WGC
/// produces no frames at all for a minimized window).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MinimizedBehavior {
    /// Keep encoding the last captured frame (frozen picture, audio runs on).
    Freeze,
    /// Branded "window minimized" card (black + logo).
    #[default]
    Branded,
    /// Plain black frames.
    Black,
    /// Pause capture entirely (video and audio) — the file doesn't grow
    /// while minimized.
    Pause,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum HudCorner {
    TopLeft,
    #[default]
    TopRight,
    BottomLeft,
    BottomRight,
}

/// Per-category on/off for in-app toast notifications (see `toast.rs`). Errors/uncategorized
/// events have no toggle here and always show (see `toast::ToastCategory::General`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct ToastCategorySettings {
    pub recording: bool,
    pub session: bool,
    pub stream: bool,
    pub buffer: bool,
    pub clip: bool,
}

impl Default for ToastCategorySettings {
    fn default() -> Self {
        Self { recording: true, session: true, stream: true, buffer: true, clip: true }
    }
}

/// Per-badge on/off and icon choice for the on-screen HUD (see `recording::hud`); each of the
/// 3 badges can be hidden individually and given its own glyph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HudBadgeSettings {
    pub recording_enabled: bool,
    pub buffer_enabled: bool,
    pub mic_enabled: bool,
    pub recording_icon: String,
    pub buffer_icon: String,
    pub mic_icon: String,
}

impl Default for HudBadgeSettings {
    fn default() -> Self {
        Self {
            recording_enabled: true,
            buffer_enabled: true,
            mic_enabled: true,
            recording_icon: "dot".into(),
            buffer_icon: "history".into(),
            mic_icon: "mic".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VideoSettings {
    pub fps: u32,
    pub bitrate_kbps: u32,
    pub encoder: EncoderChoice,
    /// See `RateControl`'s doc comment.
    #[serde(default)]
    pub rate_control: RateControl,
    /// QP/CRF value for `RateControl::Cqp`/`VbrCq` (ignored otherwise) — 0
    /// (best, largest) to 51 (worst, smallest); 23 is a typical "visually
    /// lossless-ish" default across encoders.
    #[serde(default = "default_quality")]
    pub quality: u32,
    pub container: VideoContainer,
    /// Codec for every recorded audio track (AAC unless changed).
    #[serde(default)]
    pub audio_codec: AudioCodec,
    /// Output resolution cap (downscale only).
    #[serde(default)]
    pub resolution: RecordingResolution,
    /// Local recordings folder. Falls back to Videos\Capcove when empty.
    pub recordings_dir: String,
    pub capture_cursor: bool,
    /// Window-target recordings only: excludes "secondary" windows layered on/owned by the
    /// captured window. Only hides overlays rendered as a separate top-level window; overlays
    /// baked directly into the game's own frames can't be separated out this way.
    #[serde(default)]
    pub exclude_overlay_windows: bool,
    /// Window-target recordings only: crop the OS title bar off the top of the capture (see
    /// `capture_session::start_window_capture`). On by default.
    #[serde(default = "default_true")]
    pub crop_titlebar: bool,
    pub audio: AudioConfig,
    pub replay_buffer: ReplayBufferSettings,
    /// Which screen corner the on-screen "recording in progress" indicator
    /// anchors to.
    #[serde(default = "default_hud_corner")]
    pub hud_corner: HudCorner,
    /// Per-badge visibility/icon for the on-screen HUD — see
    /// `HudBadgeSettings`.
    #[serde(default)]
    pub hud_badges: HudBadgeSettings,
    /// Which screen corner in-app toast notifications (recording started/
    /// stopped, errors, etc.) slide in from — independent of `hud_corner`
    /// since the REC dot and toasts don't need to share a spot.
    #[serde(default = "default_toast_corner")]
    pub toast_corner: HudCorner,
    /// Which toast categories are enabled — see `ToastCategorySettings`.
    #[serde(default)]
    pub toast_categories: ToastCategorySettings,
    /// What gets written while the recorded window is minimized.
    #[serde(default)]
    pub minimized_behavior: MinimizedBehavior,
    /// Title template + visibility shared by every YouTube live stream.
    #[serde(default)]
    pub youtube_live: YoutubeLiveSettings,
}

impl Default for VideoSettings {
    fn default() -> Self {
        Self {
            fps: 30,
            bitrate_kbps: 12000,
            encoder: EncoderChoice::Auto,
            rate_control: RateControl::default(),
            quality: default_quality(),
            container: VideoContainer::Mp4Fragmented,
            recordings_dir: String::new(),
            capture_cursor: true,
            exclude_overlay_windows: false,
            crop_titlebar: true,
            replay_buffer: ReplayBufferSettings::default(),
            audio: AudioConfig::default(),
            audio_codec: AudioCodec::default(),
            resolution: RecordingResolution::default(),
            hud_corner: default_hud_corner(),
            hud_badges: HudBadgeSettings::default(),
            toast_corner: default_toast_corner(),
            toast_categories: ToastCategorySettings::default(),
            minimized_behavior: MinimizedBehavior::default(),
            youtube_live: YoutubeLiveSettings::default(),
        }
    }
}

fn default_true() -> bool { true }
fn default_hud_corner() -> HudCorner { HudCorner::BottomRight }
fn default_toast_corner() -> HudCorner { HudCorner::TopLeft }
fn default_quality() -> u32 { 23 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShortcutSlot {
    pub id: String,
    pub combo: String,
    pub capture: ShortcutCapture,
    /// Ordered action list — currently only meaningful for `SaveReplay` slots,
    /// which ignore `capture` entirely (see `ShortcutAction`).
    pub actions: Vec<ShortcutAction>,
    pub show_in_menu: bool,
    pub label: String,
    /// When true, the window-picker overlay spans all monitors; when false,
    /// only the monitor under the cursor. Not used by `RecordArea` (area
    /// selection is always on the monitor under the cursor).
    #[serde(default = "default_true")]
    pub multi_monitor: bool,
    /// Icon shown for this shortcut; `None` falls back to one matching the capture type.
    #[serde(default)]
    pub icon: Option<String>,
}

/// Defaults are pressable one-handed (Ctrl+Alt+digit) and chosen to avoid combos commonly
/// bound by games, in-game overlays, and OS capture shortcuts.
pub fn default_shortcuts() -> Vec<ShortcutSlot> {
    vec![
        ShortcutSlot {
            id: "save_replay".into(),
            combo: "F8".into(),
            capture: ShortcutCapture::RecordMonitor, // ignored for action slots
            actions: vec![ShortcutAction::SaveReplay],
            show_in_menu: true,
            label: String::new(),
            multi_monitor: true,
            icon: None,
        },
        ShortcutSlot {
            id: "shortcut_wheel".into(),
            combo: "Alt+F2".into(),
            capture: ShortcutCapture::RecordMonitor, // ignored for action slots
            actions: vec![ShortcutAction::OpenWheel],
            show_in_menu: false,
            label: String::new(),
            multi_monitor: true,
            icon: None,
        },
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SyncMode {
    Full,
    #[default]
    LocalFirst,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub hotkeys_enabled: bool,
    pub shortcuts: Vec<ShortcutSlot>,
    pub autostart: bool,
    pub sync_enabled: bool,
    /// Name of the folder to create in Google Drive
    pub drive_folder_name: String,
    pub google_client_id: String,
    pub google_client_secret: String,
    /// Whether the sync queue is paused (persists across restarts)
    pub sync_paused: bool,
    /// Whether to open the gallery window automatically on startup
    pub start_with_gallery: bool,
    pub sync_mode: SyncMode,
    pub language: String,
    pub run_as_admin: bool,
    /// Whether the user has completed the first-run onboarding wizard
    #[serde(default)]
    pub onboarded: bool,
    /// Whether to automatically check for app updates on startup
    #[serde(default = "default_true")]
    pub auto_update: bool,
    /// Version string of the Terms/Privacy the user has last accepted (see
    /// `src/lib/legal.js`'s `LEGAL_VERSION`). Mismatch prompts re-acceptance.
    #[serde(default)]
    pub accepted_legal_version: String,
    /// Last app version the user has seen "What's New" for.
    #[serde(default)]
    pub last_seen_version: String,
    /// Update version the user has already been notified about, so the
    /// "update available" modal doesn't reappear on every gallery open.
    #[serde(default)]
    pub last_notified_update_version: String,
    #[serde(default)]
    pub video: VideoSettings,
    /// Whether the wheel/toast/HUD/in-game-gallery overlays set
    /// `WDA_EXCLUDEFROMCAPTURE` — hidden from any screen-capture/streaming
    /// software, not just our own recorder. On by default.
    #[serde(default = "default_true")]
    pub hide_overlays_from_capture: bool,
    /// Reusable YouTube liveStream (stream key) id — created once on the
    /// first full-session live stream and bound to every later broadcast,
    /// instead of minting a new key per session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub youtube_stream_id: Option<String>,
    /// Local storage cap in MB for the recordings folder — `None` means no
    /// limit. Only has any effect while `auto_delete_oldest` is also on;
    /// see `video_thumb::enforce_storage_limit`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_limit_mb: Option<u64>,
    /// When over `storage_limit_mb`, automatically delete the oldest local
    /// recordings (never Drive/YouTube backups themselves — just the local
    /// copy, same as a manual delete) until back under the limit.
    #[serde(default)]
    pub auto_delete_oldest: bool,
    /// Auto-cleanup only ever removes full recordings, never clips, when on.
    #[serde(default)]
    pub only_delete_long_recordings: bool,
    /// Local deletions (manual or auto-cleanup) go to the Recycle Bin
    /// instead of being permanently removed outright.
    #[serde(default = "default_true")]
    pub use_recycle_bin: bool,
    /// Auto-cleanup (the global storage-limit pass, and every folder's own
    /// `auto_delete_days`) skips anything marked favorite when this is on.
    #[serde(default)]
    pub keep_favorites: bool,
    /// User-defined recording destinations — see `RecordingFolder`.
    #[serde(default)]
    pub recording_folders: Vec<RecordingFolder>,
    /// Unix timestamp of the newest auto-deleted-file entry the startup
    /// summary modal has already shown — see
    /// `video_thumb::check_storage_startup_summary`. `0` means never shown.
    #[serde(default)]
    pub deletion_summary_acked_at: i64,
    #[serde(default)]
    pub sound_effects: SoundEffectsSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkeys_enabled: true,
            shortcuts: default_shortcuts(),
            autostart: false,
            sync_enabled: true,
            drive_folder_name: "Capcove".into(),
            google_client_id: String::new(),
            google_client_secret: String::new(),
            sync_paused: false,
            start_with_gallery: true,
            sync_mode: SyncMode::Manual,
            language: "en".into(),
            run_as_admin: false,
            onboarded: false,
            auto_update: true,
            accepted_legal_version: String::new(),
            last_seen_version: String::new(),
            last_notified_update_version: String::new(),
            video: VideoSettings::default(),
            hide_overlays_from_capture: true,
            youtube_stream_id: None,
            storage_limit_mb: None,
            auto_delete_oldest: false,
            only_delete_long_recordings: false,
            use_recycle_bin: true,
            keep_favorites: false,
            recording_folders: Vec::new(),
            deletion_summary_acked_at: 0,
            sound_effects: SoundEffectsSettings::default(),
        }
    }
}

// Credentials are XOR-obfuscated at build time by build.rs (read from .env).
// The key and ciphertext live in separate locations in the binary so that a
// simple `strings` scan cannot recover the plaintext.
mod embedded_creds {
    include!(concat!(env!("OUT_DIR"), "/credentials.rs"));
}

fn xor_decrypt(enc: &[u8], key: &[u8]) -> String {
    enc.iter()
        .zip(key.iter().cycle())
        .map(|(b, k)| (b ^ k) as char)
        .collect()
}

/// Returns true when the app was built with embedded OAuth credentials.
/// When false, users must supply their own Google client ID / secret.
pub fn has_builtin_credentials() -> bool {
    !default_client_id().is_empty()
}

fn default_client_id() -> &'static str {
    static V: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    V.get_or_init(|| xor_decrypt(embedded_creds::_ENC_CLIENT_ID, embedded_creds::_CRED_KEY))
}

fn default_client_secret() -> &'static str {
    static V: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    V.get_or_init(|| xor_decrypt(embedded_creds::_ENC_CLIENT_SECRET, embedded_creds::_CRED_KEY))
}

impl Settings {
    pub fn effective_google_client_id(&self) -> &str {
        let id = self.google_client_id.trim();
        if id.is_empty() { default_client_id() } else { id }
    }

    pub fn effective_google_client_secret(&self) -> &str {
        let s = self.google_client_secret.trim();
        if s.is_empty() { default_client_secret() } else { s }
    }

    pub fn resolved_recordings_dir(&self) -> PathBuf {
        if !self.video.recordings_dir.trim().is_empty() {
            return PathBuf::from(self.video.recordings_dir.trim());
        }
        let videos = dirs_videos().unwrap_or_else(|| PathBuf::from("."));
        videos.join("Capcove")
    }

    pub fn folder_by_id(&self, id: &str) -> Option<&RecordingFolder> {
        self.recording_folders.iter().find(|f| f.id == id)
    }

    /// A recording folder's rules, looked up by subfolder-name path segment (e.g. `"Highlights"`
    /// out of `"Highlights/foo.mp4"`), since only the name is persisted on disk, not the id.
    /// `None` for the recordings root or an unmatched subfolder.
    /// Finds a folder by name; a game-specific folder match wins over a same-named global
    /// one, since the path alone can't disambiguate the two (see `video_thumb::folder_name_of`).
    pub fn folder_by_name_scoped(&self, name: &str, game: Option<&str>) -> Option<&RecordingFolder> {
        if let Some(g) = game {
            if let Some(f) = self.recording_folders.iter().find(|f| f.name == name && f.game.as_deref() == Some(g)) {
                return Some(f);
            }
        }
        self.recording_folders.iter().find(|f| f.name == name && f.game.is_none())
    }
}

fn dirs_videos() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("Videos"))
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Videos"))
    }
}

pub struct ConfigStore {
    path: PathBuf,
    pub settings: Mutex<Settings>,
}

/// Reverts the brief Cmd-default experiment back to Ctrl — only touches
/// combos that exactly match that short-lived default, so any shortcut the
/// user customized themselves is left alone.
#[cfg(target_os = "macos")]
fn migrate_legacy_ctrl_shortcuts(settings: &mut Settings) {
    for slot in &mut settings.shortcuts {
        if let Some(n) = slot.combo.strip_prefix("Cmd+Shift+") {
            if n.len() == 1 && n.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                slot.combo = format!("Ctrl+Shift+{n}");
            }
        }
    }
}
#[cfg(not(target_os = "macos"))]
fn migrate_legacy_ctrl_shortcuts(_settings: &mut Settings) {}

/// Swaps a retired default shortcut set for the current defaults — only when the saved
/// shortcuts exactly match one of the untouched legacy sets, so user customizations are left alone.
fn migrate_legacy_default_shortcuts(settings: &mut Settings) {
    let s = &settings.shortcuts;
    let is_ctrl_shift_set = s.len() == 3
        && s.iter().all(|slot| slot.actions.is_empty() && slot.label.is_empty())
        && matches!(s[0].capture, ShortcutCapture::RecordWindow) && s[0].combo == "Ctrl+Shift+1"
        && matches!(s[1].capture, ShortcutCapture::RecordArea) && s[1].combo == "Ctrl+Shift+2"
        && matches!(s[2].capture, ShortcutCapture::RecordMonitor) && s[2].combo == "Ctrl+Shift+3";
    let is_retired_action_set = |c0: &str, c1: &str, c2: &str| {
        s.len() == 3
            && s.iter().all(|slot| slot.label.is_empty())
            && s[0].actions == vec![ShortcutAction::SaveReplay] && s[0].combo == c0
            && s[1].actions.is_empty() && matches!(s[1].capture, ShortcutCapture::RecordMonitor) && s[1].combo == c1
            && s[2].actions == vec![ShortcutAction::OpenWheel] && s[2].combo == c2
    };
    if is_ctrl_shift_set
        || is_retired_action_set("F8", "Alt+F7", "F9")
        || is_retired_action_set("Ctrl+F12", "Ctrl+F11", "Ctrl+F10")
        || is_retired_action_set("Alt+Q", "Alt+W", "Alt+E")
        || is_retired_action_set("Ctrl+Alt+1", "Ctrl+Alt+2", "Ctrl+Alt+3")
        || is_retired_action_set("Ctrl+Alt+1", "Ctrl+Alt+2", "Alt+F2")
    {
        settings.shortcuts = default_shortcuts();
    }
}

impl ConfigStore {
    pub fn load(config_dir: PathBuf) -> Self {
        let path = config_dir.join("settings.json");
        let mut settings: Settings = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        migrate_legacy_ctrl_shortcuts(&mut settings);
        migrate_legacy_default_shortcuts(&mut settings);
        Self {
            path,
            settings: Mutex::new(settings),
        }
    }

    pub fn get(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }

    pub fn save(&self, new: Settings) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(&new)?;
        std::fs::write(&self.path, json)?;
        *self.settings.lock().unwrap() = new;
        Ok(())
    }
}
