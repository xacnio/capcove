//! WASAPI audio capture, each source on its own dedicated thread/COM
//! apartment, streaming raw PCM via a callback: system-output loopback and
//! microphone (`start_capture`), and per-process loopback (`start_process_capture`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::Serialize;
use windows::core::PCWSTR;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::{
    eCapture, eConsole, eRender, IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator,
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK,
    DEVICE_STATE_ACTIVE, MMDeviceEnumerator,
};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ};

#[derive(Debug, Clone, Serialize)]
pub struct AudioDeviceInfo {
    pub id: String,
    pub label: String,
    /// The current OS default endpoint for this flow — lets the UI show the
    /// concrete device behind the "Default device" choice.
    pub is_default: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFlow {
    Render,  // system output, opened with loopback
    Capture, // microphone
}

fn device_friendly_name(device: &windows::Win32::Media::Audio::IMMDevice) -> String {
    unsafe {
        let Ok(store) = device.OpenPropertyStore(STGM_READ) else { return String::new() };
        let Ok(pv) = store.GetValue(&PKEY_Device_FriendlyName) else { return String::new() };
        let raw = pv.as_raw();
        const VT_LPWSTR: u16 = 31;
        if raw.Anonymous.Anonymous.vt == VT_LPWSTR {
            let ptr = raw.Anonymous.Anonymous.Anonymous.pwszVal;
            if ptr.is_null() {
                String::new()
            } else {
                let len = (0..).take_while(|&i| *ptr.add(i) != 0).count();
                String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len))
            }
        } else {
            String::new()
        }
    }
}

/// Lists active endpoints for the given flow. `Render` devices are what
/// `list_devices(Render)` + loopback-mode capture turns into "system audio"
/// sources; `Capture` devices are microphones.
pub fn list_devices(flow: AudioFlow) -> Result<Vec<AudioDeviceInfo>, String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
        let data_flow = match flow {
            AudioFlow::Render => eRender,
            AudioFlow::Capture => eCapture,
        };
        let collection = enumerator
            .EnumAudioEndpoints(data_flow, DEVICE_STATE_ACTIVE)
            .map_err(|e| e.to_string())?;
        let default_id = enumerator
            .GetDefaultAudioEndpoint(data_flow, eConsole)
            .ok()
            .and_then(|d| d.GetId().ok())
            .and_then(|s| s.to_string().ok());
        let count = collection.GetCount().map_err(|e| e.to_string())?;
        let mut out = Vec::with_capacity(count as usize);
        for i in 0..count {
            let device = collection.Item(i).map_err(|e| e.to_string())?;
            let id = device.GetId().map_err(|e| e.to_string())?.to_string().unwrap_or_default();
            let label = device_friendly_name(&device);
            let is_default = default_id.as_deref() == Some(id.as_str());
            out.push(AudioDeviceInfo { id, label: if label.is_empty() { format!("Device {}", i + 1) } else { label }, is_default });
        }
        Ok(out)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AudioFormat {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
}

impl AudioFormat {
    /// FFmpeg `-f` value for this format. Only 16-bit PCM and 32-bit IEEE
    /// float are handled — the only two formats WASAPI shared-mode mix
    /// formats realistically come back as.
    pub fn ffmpeg_sample_fmt(&self) -> &'static str {
        if self.bits_per_sample >= 32 { "f32le" } else { "s16le" }
    }
}

pub struct AudioCaptureHandle {
    stop_flag: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl AudioCaptureHandle {
    pub fn stop(mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Starts capturing `device_id` (empty = system default for the given flow).
/// `on_data` is called on the capture thread with raw PCM bytes as they
/// arrive — keep it fast, since it's writing to a TCP socket in practice.
pub fn start_capture(
    device_id: String,
    flow: AudioFlow,
    mut on_data: impl FnMut(&[u8]) + Send + 'static,
) -> Result<(AudioCaptureHandle, AudioFormat), String> {
    let (tx, rx) = std::sync::mpsc::channel::<Result<AudioFormat, String>>();
    let stop_flag = Arc::new(AtomicBool::new(false));
    let stop_flag2 = stop_flag.clone();

    let thread = std::thread::spawn(move || unsafe {
        let setup = (|| -> Result<(IAudioClient, IAudioCaptureClient, AudioFormat), String> {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
            let device = if device_id.is_empty() {
                let data_flow = match flow { AudioFlow::Render => eRender, AudioFlow::Capture => eCapture };
                enumerator.GetDefaultAudioEndpoint(data_flow, eConsole).map_err(|e| e.to_string())?
            } else {
                let wide: Vec<u16> = device_id.encode_utf16().chain(std::iter::once(0)).collect();
                enumerator.GetDevice(PCWSTR(wide.as_ptr())).map_err(|e| e.to_string())?
            };
            let client: IAudioClient = device.Activate(CLSCTX_ALL, None).map_err(|e| e.to_string())?;
            let mix_format = client.GetMixFormat().map_err(|e| e.to_string())?;
            let wfx = *mix_format;
            let fmt = AudioFormat {
                sample_rate: wfx.nSamplesPerSec,
                channels: wfx.nChannels,
                bits_per_sample: wfx.wBitsPerSample,
            };
            let stream_flags: u32 = if flow == AudioFlow::Render { AUDCLNT_STREAMFLAGS_LOOPBACK } else { 0 };
            // 200ms shared-mode buffer — generous enough that our ~10ms poll loop
            // never risks an overrun even under scheduler jitter.
            let result = client.Initialize(AUDCLNT_SHAREMODE_SHARED, stream_flags, 200_0000, 0, mix_format, None);
            windows::Win32::System::Com::CoTaskMemFree(Some(mix_format as *mut core::ffi::c_void));
            result.map_err(|e| e.to_string())?;
            let capture_client: IAudioCaptureClient = client.GetService().map_err(|e| e.to_string())?;
            Ok((client, capture_client, fmt))
        })();

        let (client, capture_client, fmt) = match setup {
            Ok(v) => v,
            Err(e) => {
                let _ = tx.send(Err(e));
                return;
            }
        };

        if let Err(e) = client.Start() {
            let _ = tx.send(Err(e.to_string()));
            return;
        }
        let _ = tx.send(Ok(fmt));

        // Device loopback/capture already delivers a continuous stream (silent
        // buffers included), so no synthesized silence needed here.
        pump_capture(&capture_client, fmt, &stop_flag2, false, &mut on_data);
        let _ = client.Stop();
    });

    match rx.recv() {
        Ok(Ok(fmt)) => Ok((AudioCaptureHandle { stop_flag, thread: Some(thread) }, fmt)),
        Ok(Err(e)) => Err(e),
        Err(_) => Err("audio capture thread exited before starting".into()),
    }
}

/// The shared 10ms poll loop draining WASAPI packets into `on_data` until
/// `stop` is set — used by both device capture and process loopback.
///
/// `synth_silence`: fill quiet gaps with generated silence, paced to wall
/// clock. Required for process loopback — unlike device loopback (which
/// delivers flagged-silent buffers continuously), process loopback delivers
/// NO buffers at all while the target app is silent, so `on_data` would
/// simply stop being called. That starves ffmpeg's audio input, and since
/// ffmpeg needs audio to advance its mux, the *whole* recording stalls the
/// moment the game goes quiet — no more frames written, clip cut short. Off
/// for device capture, which already produces a continuous stream.
unsafe fn pump_capture(
    capture_client: &IAudioCaptureClient,
    fmt: AudioFormat,
    stop: &AtomicBool,
    synth_silence: bool,
    on_data: &mut (impl FnMut(&[u8]) + Send + 'static),
) {
    let block_align = (fmt.channels as usize) * (fmt.bits_per_sample as usize / 8);
    let sample_rate = fmt.sample_rate as u64;
    // Wall-clock anchor for silence pacing: `emitted_frames` tracks how many
    // audio frames have gone to `on_data` (real + synthesized), and silence
    // is only ever added to catch it up to elapsed real time — so when real
    // audio is flowing, no silence is added at all (no doubling / no speedup).
    let start = std::time::Instant::now();
    let mut emitted_frames: u64 = 0;
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(std::time::Duration::from_millis(10));
        let mut got_any = false;
        loop {
            let packet_size = capture_client.GetNextPacketSize().unwrap_or(0);
            if packet_size == 0 {
                break;
            }
            let mut data_ptr: *mut u8 = std::ptr::null_mut();
            let mut frames: u32 = 0;
            let mut flags_out: u32 = 0;
            if capture_client.GetBuffer(&mut data_ptr, &mut frames, &mut flags_out, None, None).is_err() {
                break;
            }
            if frames > 0 {
                got_any = true;
                let byte_len = frames as usize * block_align;
                let silent = (flags_out & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0;
                if silent || data_ptr.is_null() {
                    on_data(&vec![0u8; byte_len]);
                } else {
                    on_data(std::slice::from_raw_parts(data_ptr, byte_len));
                }
                emitted_frames += frames as u64;
            }
            let _ = capture_client.ReleaseBuffer(frames);
        }
        // Only top up with silence on ticks that delivered no real audio, so
        // active playback is never padded (which would speed it up).
        if synth_silence && !got_any {
            let target_frames = (start.elapsed().as_secs_f64() * sample_rate as f64) as u64;
            if target_frames > emitted_frames {
                // Safety cap per tick — a fresh loop tick is ~10ms, so this
                // only bites if the thread was starved; wall clock catches up.
                let gap = (target_frames - emitted_frames).min(sample_rate / 5);
                on_data(&vec![0u8; gap as usize * block_align]);
                emitted_frames += gap;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Per-process loopback (Windows 10 2004+)
// ---------------------------------------------------------------------------

use windows::core::{implement, Interface};
use windows::Win32::Media::Audio::{
    ActivateAudioInterfaceAsync, AudioSessionStateActive, IActivateAudioInterfaceAsyncOperation,
    IActivateAudioInterfaceCompletionHandler, IActivateAudioInterfaceCompletionHandler_Impl,
    IAudioSessionControl2, IAudioSessionManager2, AUDIOCLIENT_ACTIVATION_PARAMS,
    AUDIOCLIENT_ACTIVATION_PARAMS_0, AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
    AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS, PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
    PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE, VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
    WAVEFORMATEX,
};

/// Signals the waiting capture thread once Windows finishes the async
/// activation of the process-loopback audio client.
#[implement(IActivateAudioInterfaceCompletionHandler)]
struct ActivationDone(std::sync::mpsc::Sender<()>);

impl IActivateAudioInterfaceCompletionHandler_Impl for ActivationDone_Impl {
    fn ActivateCompleted(&self, _op: Option<&IActivateAudioInterfaceAsyncOperation>) -> windows::core::Result<()> {
        let _ = self.0.send(());
        Ok(())
    }
}

/// The fixed format process loopback is initialized with — the virtual
/// loopback device has no mix format of its own, the caller dictates one.
const PROCESS_LOOPBACK_FORMAT: AudioFormat = AudioFormat { sample_rate: 48_000, channels: 2, bits_per_sample: 32 };

/// Captures one process tree's audio in isolation (what it *renders*, no
/// matter which output device it plays to). `on_data` receives interleaved
/// 48kHz stereo f32 PCM.
pub fn start_process_capture(
    pid: u32,
    on_data: impl FnMut(&[u8]) + Send + 'static,
) -> Result<(AudioCaptureHandle, AudioFormat), String> {
    start_process_capture_mode(pid, false, on_data)
}

/// The inverse: everything on the default render endpoint except one
/// process tree, so the System Audio track doesn't double-carry a game that
/// already has its own dedicated track.
pub fn start_process_exclude_capture(
    pid: u32,
    on_data: impl FnMut(&[u8]) + Send + 'static,
) -> Result<(AudioCaptureHandle, AudioFormat), String> {
    start_process_capture_mode(pid, true, on_data)
}

/// The WASAPI device id of the current default render endpoint — used to
/// decide whether the configured System Audio device can be swapped for
/// exclude-mode process loopback (see `start_process_exclude_capture`).
pub fn default_render_device_id() -> Option<String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
        let id = device.GetId().ok()?;
        let s = id.to_string().ok()?;
        Some(s)
    }
}

/// Triggers the OS microphone-consent prompt by briefly starting a real
/// capture stream on the default device — the exact same `Activate` →
/// `Initialize` → `Start` sequence `start_capture` uses for a real recording,
/// immediately torn back down. `ActivateAudioInterfaceAsync` (the API
/// generally recommended for triggering this consent flow, e.g. via the
/// `Windows.Media.Capture.MediaCapture` UWP surface) did not reliably surface
/// the prompt for a plain WASAPI activation in testing — actually starting
/// the stream does, matching what real recordings already observably do.
pub fn request_microphone_consent() -> Result<(), String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
        let device = enumerator.GetDefaultAudioEndpoint(eCapture, eConsole).map_err(|e| e.to_string())?;
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None).map_err(|e| e.to_string())?;
        let mix_format = client.GetMixFormat().map_err(|e| e.to_string())?;
        let init_result = client.Initialize(AUDCLNT_SHAREMODE_SHARED, 0, 200_0000, 0, mix_format, None);
        windows::Win32::System::Com::CoTaskMemFree(Some(mix_format as *mut core::ffi::c_void));
        init_result.map_err(|e| e.to_string())?;
        client.Start().map_err(|e| e.to_string())?;
        let _ = client.Stop();
        Ok(())
    }
}

fn start_process_capture_mode(
    pid: u32,
    exclude: bool,
    mut on_data: impl FnMut(&[u8]) + Send + 'static,
) -> Result<(AudioCaptureHandle, AudioFormat), String> {
    let (tx, rx) = std::sync::mpsc::channel::<Result<AudioFormat, String>>();
    let stop_flag = Arc::new(AtomicBool::new(false));
    let stop_flag2 = stop_flag.clone();

    let thread = std::thread::spawn(move || unsafe {
        let setup = (|| -> Result<(IAudioClient, IAudioCaptureClient), String> {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

            let params = AUDIOCLIENT_ACTIVATION_PARAMS {
                ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
                Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
                    ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                        TargetProcessId: pid,
                        ProcessLoopbackMode: if exclude {
                            PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE
                        } else {
                            PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE
                        },
                    },
                },
            };
            // The activation params travel as a VT_BLOB PROPVARIANT; the
            // wrapper type has no blob constructor, so build a
            // layout-identical repr(C) mirror and cast.
            #[repr(C)]
            struct PropVariantBlob {
                vt: u16,
                reserved1: u16,
                reserved2: u16,
                reserved3: u16,
                cb_size: u32,
                p_blob_data: *mut u8,
            }
            let prop = PropVariantBlob {
                vt: 65, // VT_BLOB
                reserved1: 0,
                reserved2: 0,
                reserved3: 0,
                cb_size: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
                p_blob_data: &params as *const _ as *mut u8,
            };

            let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
            let handler: IActivateAudioInterfaceCompletionHandler = ActivationDone(done_tx).into();
            let op = ActivateAudioInterfaceAsync(
                VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
                &IAudioClient::IID,
                Some(&prop as *const _ as *const windows::core::PROPVARIANT),
                &handler,
            )
            .map_err(|e| e.to_string())?;
            done_rx
                .recv_timeout(std::time::Duration::from_secs(3))
                .map_err(|_| "process loopback activation timed out".to_string())?;

            let mut activate_hr = windows::core::HRESULT(0);
            let mut activated: Option<windows::core::IUnknown> = None;
            op.GetActivateResult(&mut activate_hr, &mut activated).map_err(|e| e.to_string())?;
            activate_hr.ok().map_err(|e| format!("process loopback activation failed: {e}"))?;
            let client: IAudioClient = activated
                .ok_or("process loopback returned no interface")?
                .cast()
                .map_err(|e| e.to_string())?;

            let fmt = PROCESS_LOOPBACK_FORMAT;
            let wf = WAVEFORMATEX {
                wFormatTag: 3, // WAVE_FORMAT_IEEE_FLOAT
                nChannels: fmt.channels,
                nSamplesPerSec: fmt.sample_rate,
                nAvgBytesPerSec: fmt.sample_rate * (fmt.channels as u32) * 4,
                nBlockAlign: fmt.channels * 4,
                wBitsPerSample: fmt.bits_per_sample,
                cbSize: 0,
            };
            client
                .Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK, 200_0000, 0, &wf, None)
                .map_err(|e| e.to_string())?;
            let capture_client: IAudioCaptureClient = client.GetService().map_err(|e| e.to_string())?;
            Ok((client, capture_client))
        })();

        let (client, capture_client) = match setup {
            Ok(v) => v,
            Err(e) => {
                let _ = tx.send(Err(e));
                return;
            }
        };
        if let Err(e) = client.Start() {
            let _ = tx.send(Err(e.to_string()));
            return;
        }
        let _ = tx.send(Ok(PROCESS_LOOPBACK_FORMAT));

        // Process loopback delivers nothing while the target app is silent, so
        // synthesize silence to keep the audio timeline (and thus ffmpeg's mux)
        // moving — see `pump_capture`'s `synth_silence` doc.
        pump_capture(&capture_client, PROCESS_LOOPBACK_FORMAT, &stop_flag2, true, &mut on_data);
        let _ = client.Stop();
    });

    match rx.recv() {
        Ok(Ok(fmt)) => Ok((AudioCaptureHandle { stop_flag, thread: Some(thread) }, fmt)),
        Ok(Err(e)) => Err(e),
        Err(_) => Err("process loopback thread exited before starting".into()),
    }
}

// ---------------------------------------------------------------------------
// Audio session (per-app) enumeration
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct AudioAppInfo {
    pub pid: u32,
    pub exe: String,
}

/// Every app seen actually playing audio since launch, deduped by exe (first
/// pid wins) — a live session check alone is too broad for idle helpers and
/// too narrow for a momentarily-paused app. Vec keeps insertion order stable.
static SEEN_AUDIO_APPS: std::sync::Mutex<Vec<AudioAppInfo>> = std::sync::Mutex::new(Vec::new());

fn accumulate_active_sessions() -> Result<(), String> {
    let current = audio_apps(true)?;
    let mut seen = SEEN_AUDIO_APPS.lock().unwrap();
    for a in current {
        if let Some(existing) = seen.iter_mut().find(|s| s.exe.eq_ignore_ascii_case(&a.exe)) {
            existing.pid = a.pid; // refresh — the app may have restarted
        } else {
            seen.push(a);
        }
    }
    Ok(())
}

/// Background poll keeping `SEEN_AUDIO_APPS` fresh even while the settings
/// page is closed. Session enumeration is a handful of COM calls every few
/// seconds; negligible cost.
pub fn spawn_session_watcher() {
    std::thread::spawn(|| {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        loop {
            let _ = accumulate_active_sessions();
            std::thread::sleep(std::time::Duration::from_secs(5));
        }
    });
}

pub fn list_audio_apps() -> Result<Vec<AudioAppInfo>, String> {
    // Fold in whatever is active right now too, so a just-started app shows
    // immediately instead of waiting for the watcher's next tick.
    accumulate_active_sessions()?;
    Ok(SEEN_AUDIO_APPS.lock().unwrap().clone())
}

/// `active_only`: the settings picker wants just the audibly-playing set;
/// pid resolution at recording start must NOT filter, since the chosen app
/// may be momentarily silent and start playing mid-recording.
fn audio_apps(active_only: bool) -> Result<Vec<AudioAppInfo>, String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
        // Every ACTIVE render endpoint, not just the default one — apps
        // routed through a virtual audio device hold their sessions on that
        // endpoint and would be invisible otherwise.
        let devices = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE).map_err(|e| e.to_string())?;
        let device_count = devices.GetCount().map_err(|e| e.to_string())?;

        let own_pid = windows::Win32::System::Threading::GetCurrentProcessId();
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for d in 0..device_count {
            let Ok(device) = devices.Item(d) else { continue };
            let Ok(manager) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else { continue };
            let Ok(sessions) = manager.GetSessionEnumerator() else { continue };
            let Ok(count) = sessions.GetCount() else { continue };
            for i in 0..count {
                let Ok(session) = sessions.GetSession(i) else { continue };
                // Only sessions actually rendering right now — WASAPI keeps
                // inactive session objects around forever, and including
                // those fills the list with silent helper processes.
                if active_only && session.GetState().map(|s| s != AudioSessionStateActive).unwrap_or(true) {
                    continue;
                }
                let Ok(session2) = session.cast::<IAudioSessionControl2>() else { continue };
                let pid = session2.GetProcessId().unwrap_or(0);
                if pid == 0 || pid == own_pid {
                    continue; // system sounds / ourselves
                }
                let Some(exe) = crate::capture::exe_for_pid(pid) else { continue };
                if seen.insert(exe.to_ascii_lowercase()) {
                    out.push(AudioAppInfo { pid, exe });
                }
            }
        }
        Ok(out)
    }
}

/// Current pid for an exe with an audio session — resolves a persisted
/// `AudioSource::Application { exe }` at recording start. Includes inactive
/// sessions since the app might be silent now and start playing later.
pub fn find_session_pid(exe: &str) -> Option<u32> {
    audio_apps(false).ok()?.into_iter().find(|a| a.exe.eq_ignore_ascii_case(exe)).map(|a| a.pid)
}
