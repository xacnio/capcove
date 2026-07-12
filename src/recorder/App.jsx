import { useEffect, useState } from "react";
import { invoke, listen } from "../lib/tauri.js";
import { createT } from "../lib/i18n.js";
import { MdCropFree, MdFullscreen, MdWindow, MdMic, MdMicOff, MdFiberManualRecord, MdStop } from "react-icons/md";
import { SiYoutube } from "react-icons/si";
import TitleBar from "../components/TitleBar.jsx";

function fmtElapsed(startedAt) {
  const secs = Math.max(0, Math.floor(Date.now() / 1000 - startedAt));
  const m = String(Math.floor(secs / 60)).padStart(2, "0");
  const s = String(secs % 60).padStart(2, "0");
  return `${m}:${s}`;
}

function IconToggle({ icon: Icon, active, onClick, title, disabled }) {
  return (
    <button
      title={title}
      onClick={onClick}
      disabled={disabled}
      className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-md border transition-colors ${
        active
          ? "border-accent-500 bg-accent-500/10 text-accent-400"
          : "border-stone-800 bg-stone-900 text-stone-400 hover:border-stone-700"
      } ${disabled ? "cursor-not-allowed opacity-40" : ""}`}
    >
      <Icon size={15} />
    </button>
  );
}

export default function App() {
  const [lang, setLang] = useState("en");
  const t = createT(lang);
  const [mode, setMode] = useState("area");
  const [session, setSession] = useState(null);
  const [micMuted, setMicMuted] = useState(false);
  const [elapsed, setElapsed] = useState("00:00");
  const [starting, setStarting] = useState(false);
  const [opacity, setOpacity] = useState(100);
  const [wantLive, setWantLive] = useState(false);
  const [windowTarget, setWindowTarget] = useState(null);

  useEffect(() => {
    invoke("recorder_window_ready").catch(() => {});
    // Seed from the backend's current mode — it may have been opened straight
    // into a specific mode by the gallery toolbar — then apply it so the
    // frame reflects it once this window's mounted.
    //
    // Window mode is the one exception: its frame/tracking is driven by the
    // picked target (`recorder_track_window`), not by `recorder_set_mode`
    // (whose non-"area" branch unconditionally *hides* the frame and bumps
    // the tracking epoch). Calling it here would race the gallery's
    // picker-first flow — the bar is built (and this effect fires) *after*
    // `start_window_tracking` already started tracking the just-picked
    // window, so re-invoking `recorder_set_mode("window")` on mount would
    // immediately hide that frame and kill the loop that just started.
    invoke("recorder_current_mode").then((m) => {
      const initial = m ?? "area";
      setMode(initial);
      if (initial !== "window") {
        invoke("recorder_set_mode", { mode: initial }).catch(() => {});
      }
    }).catch(() => {
      invoke("recorder_set_mode", { mode: "area" }).catch(() => {});
    });
    // The `recorder-window-picked` event alone can't be relied on to reach
    // this window while it's still loading (see `recorder_pick_window_select`
    // on the backend) — pull the last pick directly, and (re-)start tracking
    // for it so Window mode always ends up actually tracking something.
    invoke("recorder_current_window_target").then((w) => {
      if (w) {
        setWindowTarget(w);
        invoke("recorder_track_window", { hwnd: w.hwnd }).catch(() => {});
      }
    }).catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    invoke("get_settings").then((s) => {
      setLang(s.language ?? "en");
      setMicMuted(!!s.video?.audio?.mic_muted);
    }).catch(() => {});
    invoke("get_recording_status").then(setSession).catch(() => {});

    const unsubs = [];
    listen("recording-started", (e) => setSession(e.payload)).then((u) => unsubs.push(u));
    listen("recording-stopped", () => setSession(null)).then((u) => unsubs.push(u));
    listen("settings-changed", async () => {
      const s = await invoke("get_settings").catch(() => null);
      if (s) {
        setLang(s.language ?? "en");
        setMicMuted(!!s.video?.audio?.mic_muted);
      }
    }).then((u) => unsubs.push(u));
    // Fired once the full-screen window picker (opened by `recorder_pick_window`)
    // reports a pick — the backend already brought that window forward, showed
    // the frame, and started tracking it; here we just sync local UI state.
    listen("recorder-window-picked", (e) => {
      setWindowTarget(e.payload);
      setMode("window");
    }).then((u) => unsubs.push(u));
    // Mode changed elsewhere (the gallery toolbar) — mirror it locally. The
    // backend already applied the visual switch; this only syncs the UI.
    listen("recorder-mode-changed", (e) => {
      if (e.payload) setMode(e.payload);
    }).then((u) => unsubs.push(u));
    return () => unsubs.forEach((fn) => fn?.());
  }, []);

  // The frame is hidden (not destroyed) when minimizing — bring it back once
  // the bar regains focus, matching whatever mode is still selected.
  //
  // `onFocusChanged`'s subscription is async, and this effect's deps change
  // often (every mode/target switch) — if the effect re-runs before the
  // previous `.then()` resolves, `unlisten` is still undefined when cleanup
  // runs, so the *old* listener (closed over the previous mode) never
  // actually unsubscribes. It then fires forever with stale state — e.g. the
  // very next time the bar regains focus, silently resetting the mode back
  // to whatever it was when that listener was created. `cancelled` catches
  // this: if cleanup already ran by the time the promise resolves, the new
  // listener is unsubscribed immediately instead of being kept alive.
  useEffect(() => {
    const win = window.__TAURI__?.window?.getCurrentWindow?.();
    if (!win) return;
    let cancelled = false;
    let unlisten;
    win.onFocusChanged(({ payload: focused }) => {
      if (!focused) return;
      // `recorder_resume_area`/`recorder_track_window`, not `recorder_set_mode`
      // — resuming after a minimize must never reposition the bar (it never
      // actually moved; only `recorder_set_mode`'s fresh-switch path should
      // snap it to a new spot).
      if (mode === "area") invoke("recorder_resume_area").catch(() => {});
      else if (mode === "window" && windowTarget) invoke("recorder_track_window", { hwnd: windowTarget.hwnd }).catch(() => {});
    }).then((u) => {
      if (cancelled) { u(); return; }
      unlisten = u;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [mode, windowTarget]);

  useEffect(() => {
    if (!session) { setElapsed("00:00"); return; }
    const tick = () => setElapsed(fmtElapsed(session.started_at));
    tick();
    const id = setInterval(tick, 1000);
    return () => clearInterval(id);
  }, [session?.started_at]);

  const recording = !!session;

  const changeMode = (next) => {
    if (recording || next === mode) return;
    setMode(next);
    invoke("recorder_set_mode", { mode: next }).catch(() => {});
  };

  // Opens the full-screen picker (live preview + titles); re-clickable even
  // while already in Window mode, to pick a different window.
  const openWindowPicker = () => {
    if (recording) return;
    invoke("recorder_pick_window").catch(() => {});
  };

  const startRecording = async () => {
    setStarting(true);
    try {
      await invoke("recorder_start", { mode, live: wantLive, window: mode === "window" ? windowTarget : null });
      setWantLive(false);
    } finally {
      setStarting(false);
    }
  };
  const stopRecording = () => invoke("stop_recording").catch(() => {});

  const toggleMic = async () => {
    const fresh = await invoke("get_settings").catch(() => null);
    if (!fresh) return;
    const nextMuted = !(fresh.video?.audio?.mic_muted ?? false);
    const next = { ...fresh, video: { ...fresh.video, audio: { ...fresh.video?.audio, mic_muted: nextMuted } } };
    setMicMuted(nextMuted);
    await invoke("save_settings", { settings: next }).catch(() => {});
  };

  // Before recording: arms/disarms starting with YouTube live already on.
  // While recording: toggles live streaming for the session in progress.
  const toggleYoutube = () => {
    if (recording) {
      invoke("toggle_live_streaming").catch(() => {});
    } else {
      setWantLive((w) => !w);
    }
  };

  const changeOpacity = (e) => {
    const v = Number(e.target.value);
    setOpacity(v);
    invoke("recorder_set_opacity", { percent: v }).catch(() => {});
  };

  const handleMinimize = () => invoke("recorder_minimize").catch(() => {});
  const handleClose = () => invoke("recorder_close").catch(() => {});

  return (
    <div className="flex h-screen flex-col">
      <TitleBar title="Capcove" lang={lang} noMaximize onMinimize={handleMinimize} onClose={handleClose}
        right={
          !recording ? (
            <button
              onClick={startRecording}
              disabled={starting || (mode === "window" && !windowTarget)}
              title={t("recorder.start")}
              className="flex h-8 items-center gap-1.5 rounded-md bg-accent-500 px-3 text-xs font-semibold text-stone-950 transition-colors hover:bg-accent-400 disabled:opacity-60"
            >
              <MdFiberManualRecord size={13} />
            </button>
          ) : (
            <div className="flex items-center gap-1.5">
              <span className="flex items-center gap-1 rounded-md border border-red-500/30 bg-red-500/10 px-2 py-1.5 text-[11px] font-semibold text-red-400">
                <MdFiberManualRecord size={8} className="animate-pulse" />
                {elapsed}
              </span>
              <button
                onClick={stopRecording}
                title={t("recorder.stop")}
                className="flex h-8 w-8 items-center justify-center rounded-md bg-stone-800 text-stone-100 transition-colors hover:bg-stone-700"
              >
                <MdStop size={14} />
              </button>
            </div>
          )
        }
      >
        <IconToggle icon={MdCropFree} active={mode === "area"} onClick={() => changeMode("area")} title={t("recorder.area")} disabled={recording} />
        <IconToggle icon={MdWindow} active={mode === "window"} onClick={openWindowPicker} title={windowTarget?.title || t("recorder.window")} disabled={recording} />
        <IconToggle icon={MdFullscreen} active={mode === "fullscreen"} onClick={() => changeMode("fullscreen")} title={t("recorder.fullscreen")} disabled={recording} />
        <IconToggle icon={micMuted ? MdMicOff : MdMic} active={!micMuted} onClick={toggleMic} title={t("recorder.mic")} />
        <IconToggle icon={SiYoutube} active={recording ? !!session?.live : wantLive} onClick={toggleYoutube} title={t("recorder.youtube")} />
        <input
          type="range"
          min={40}
          max={100}
          value={opacity}
          onChange={changeOpacity}
          title={t("recorder.opacity")}
          className="h-1.5 w-16 shrink-0 accent-accent-500"
        />
      </TitleBar>
    </div>
  );
}
