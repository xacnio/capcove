//! Short sound-effect playback for recording/replay-buffer state changes.
//! Presets are synthesized tones (no bundled assets); a user can also point
//! an event at their own audio file or a Windows system sound.

use std::time::Duration;

use rodio::{Decoder, OutputStream, Sink, Source};
use serde::Serialize;

use crate::config::{SoundEffectSetting, SoundPreset, SoundSource};

const SAMPLE_RATE: u32 = 48000;

/// One overtone of a `RichTone`: `ratio` is a multiple of the base
/// frequency, `amp` its starting weight, `decay` its own exponential rate.
#[derive(Clone, Copy)]
struct Partial {
    ratio: f32,
    amp: f32,
    decay: f32,
}

/// A short tone built from a fundamental plus a few decaying overtones,
/// rather than one flat sine — reads as "bell"/"ping"/"glass", not a beep.
struct RichTone {
    base_freq: f32,
    partials: Vec<Partial>,
    n: u32,
    total: u32,
    attack: u32,
    release: u32,
    amp: f32,
}

impl RichTone {
    fn new(base_freq: f32, partials: &[Partial], ms: u64, amp: f32) -> Self {
        let total = ((SAMPLE_RATE as u64 * ms) / 1000).max(1) as u32;
        // Always fade the last ~12ms to silence — cutting the stream off
        // mid-amplitude reads as a click/pop.
        let release = (((SAMPLE_RATE as u64 * 12) / 1000).min(total as u64 / 2)).max(1) as u32;
        RichTone { base_freq, partials: partials.to_vec(), n: 0, total, attack: (total / 20).max(1), release, amp }
    }
}

impl Iterator for RichTone {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.n >= self.total {
            return None;
        }
        let t = self.n as f32 / SAMPLE_RATE as f32;
        let raw: f32 = self.partials.iter().map(|p| {
            (2.0 * std::f32::consts::PI * self.base_freq * p.ratio * t).sin() * p.amp * (-p.decay * t).exp()
        }).sum();
        let attack_env = if self.n < self.attack { self.n as f32 / self.attack as f32 } else { 1.0 };
        let release_env = if self.n + self.release >= self.total {
            (self.total - self.n) as f32 / self.release as f32
        } else {
            1.0
        };
        self.n += 1;
        Some(raw * attack_env * release_env * self.amp)
    }
}

impl Source for RichTone {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        1
    }
    fn sample_rate(&self) -> u32 {
        SAMPLE_RATE
    }
    fn total_duration(&self) -> Option<Duration> {
        Some(Duration::from_secs_f32(self.total as f32 / SAMPLE_RATE as f32))
    }
}

fn rest(sink: &Sink, ms: u64) {
    sink.append(rodio::source::Zero::<f32>::new(1, SAMPLE_RATE).take_duration(Duration::from_millis(ms)));
}

// Note frequencies (Hz), named for readability below.
const C5: f32 = 523.25;
const E5: f32 = 659.25;
const G5: f32 = 783.99;
const A5: f32 = 880.0;
const B5: f32 = 987.77;
const E6: f32 = 1318.51;

// A clean tone with just a touch of octave brightness — a soft, modern
// notification "ping" rather than a bare lab-tone sine.
fn soft_ping(sink: &Sink, freq: f32, ms: u64) {
    sink.append(RichTone::new(freq, &[
        Partial { ratio: 1.0, amp: 0.9, decay: 3.5 },
        Partial { ratio: 2.0, amp: 0.15, decay: 6.0 },
    ], ms, 0.3));
}

// Fundamental plus a fast-decaying high partial — reads as a woody knock.
fn marimba(sink: &Sink, freq: f32, ms: u64) {
    sink.append(RichTone::new(freq, &[
        Partial { ratio: 1.0, amp: 0.85, decay: 4.5 },
        Partial { ratio: 3.97, amp: 0.35, decay: 9.0 },
    ], ms, 0.3));
}

// Two very closely detuned partials beat against each other as they decay
// — the shimmer that makes something sound "glassy" rather than a plain tone.
fn glass(sink: &Sink, freq: f32, ms: u64) {
    sink.append(RichTone::new(freq, &[
        Partial { ratio: 1.0, amp: 0.6, decay: 2.2 },
        Partial { ratio: 1.006, amp: 0.6, decay: 2.2 },
        Partial { ratio: 2.0, amp: 0.2, decay: 3.0 },
    ], ms, 0.28));
}

// Inharmonic partials at bell-like ratios (1x / 2.4x / 3x), each decaying at
// its own rate — the classic trick behind every synthesized "bell" sound.
fn soft_bell(sink: &Sink, freq: f32, ms: u64) {
    sink.append(RichTone::new(freq, &[
        Partial { ratio: 1.0, amp: 0.7, decay: 2.0 },
        Partial { ratio: 2.4, amp: 0.35, decay: 3.2 },
        Partial { ratio: 3.0, amp: 0.2, decay: 4.0 },
    ], ms, 0.3));
}

fn tone(sink: &Sink, freq: f32, ms: u64) {
    sink.append(RichTone::new(freq, &[
        Partial { ratio: 1.0, amp: 1.0, decay: 1.5 },
        Partial { ratio: 2.0, amp: 0.25, decay: 3.0 },
    ], ms, 0.28));
}

fn play_preset(sink: &Sink, preset: SoundPreset) {
    match preset {
        SoundPreset::SoftPing => soft_ping(sink, E6, 260),
        SoundPreset::Marimba => marimba(sink, A5, 280),
        SoundPreset::Glass => glass(sink, C5 * 2.0, 500),
        SoundPreset::SoftBell => soft_bell(sink, A5, 450),
        SoundPreset::TwoToneUp => { soft_ping(sink, 440.0, 120); soft_ping(sink, 660.0, 220); }
        SoundPreset::TwoToneDown => { soft_ping(sink, 660.0, 120); soft_ping(sink, 440.0, 220); }
        SoundPreset::Coin => { tone(sink, B5, 60); tone(sink, E6, 220); }
        SoundPreset::Success => { tone(sink, C5, 80); tone(sink, E5, 80); tone(sink, G5, 200); }
        SoundPreset::Alert => { tone(sink, A5, 80); rest(sink, 60); tone(sink, A5, 80); }
    }
}

fn play_file(sink: &Sink, path: &str) {
    let Ok(file) = std::fs::File::open(path) else {
        log::warn!("sound: couldn't open sound file {path}");
        return;
    };
    match Decoder::new(std::io::BufReader::new(file)) {
        Ok(source) => sink.append(source),
        Err(e) => log::warn!("sound: couldn't decode {path}: {e}"),
    }
}

/// Opens a fresh output stream per sound (closed on return) rather than one
/// persistent stream — that would hold the audio device open for the app's
/// whole life, risking contention with the replay buffer's own capture.
fn play_source(source: SoundSource) {
    let Ok((_stream, handle)) = OutputStream::try_default() else { return };
    let Ok(sink) = Sink::try_new(&handle) else { return };
    match source {
        SoundSource::Preset { preset } => play_preset(&sink, preset),
        SoundSource::Custom { path } => play_file(&sink, &path),
    }
    sink.sleep_until_end();
}

/// Fire-and-forget: no-ops if `setting.enabled` is false, otherwise plays on
/// a background thread so the caller (a recording/buffer state transition)
/// never blocks on audio device setup or playback.
pub fn play(setting: &SoundEffectSetting) {
    if !setting.enabled {
        return;
    }
    let source = setting.source.clone();
    std::thread::spawn(move || play_source(source));
}

/// Settings page's "preview" button — plays regardless of `enabled`, since
/// auditioning a sound before turning it on is the whole point.
#[tauri::command]
pub fn preview_sound_effect(source: SoundSource) {
    std::thread::spawn(move || play_source(source));
}

/// Lets the user pick their own audio file for a sound-effect slot.
#[tauri::command]
pub async fn pick_sound_file(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("Audio", &["wav", "mp3", "ogg", "flac"])
            .blocking_pick_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(picked.map(|p| p.to_string()))
}

#[derive(Serialize)]
pub struct SystemSound {
    pub name: String,
    pub path: String,
}

/// Windows' own `%SystemRoot%\Media\*.wav` notification sounds.
#[tauri::command]
pub fn list_windows_sounds() -> Vec<SystemSound> {
    #[cfg(windows)]
    {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());
        let dir = std::path::Path::new(&root).join("Media");
        let mut out: Vec<SystemSound> = std::fs::read_dir(&dir)
            .map(|entries| {
                entries
                    .flatten()
                    .filter_map(|e| {
                        let path = e.path();
                        if path.extension().and_then(|x| x.to_str())?.eq_ignore_ascii_case("wav") {
                            let name = path.file_stem()?.to_str()?.to_string();
                            Some(SystemSound { name, path: path.to_string_lossy().into_owned() })
                        } else {
                            None
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}
