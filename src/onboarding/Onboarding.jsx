import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "../lib/tauri.js";
import { useT } from "../lib/i18n.js";
import { Toggle, Row, Button, Card, inputCls } from "../components/settingsUI.jsx";
import { RecordingModeCard, FPS_OPTIONS, RESOLUTION_OPTIONS, ENCODER_LABELS } from "../components/RecordSettingsCard.jsx";
import { ShortcutCard } from "../components/ShortcutEditor.jsx";
import LegalDocModal from "../components/LegalDocModal.jsx";
import { LEGAL_VERSION } from "../lib/legal.js";
import logo from "../assets/logo.png";

const STEPS = ["welcome", "language", "recordingMode", "video", "audio", "shortcuts", "drive", "startup", "finish"];
const TOTAL = STEPS.length;

const LANGUAGES = [
  { code: "en", label: "English" },
  { code: "tr", label: "Türkçe" },
];

// Quick video subset for the wizard: resolution, frame rate, encoder — the
// same fields (and options) as Settings → Record, so nothing here is a
// onboarding-only rephrasing.
function VideoQuickStep({ draft, apply, t }) {
  const video = draft.video ?? {};
  const [encoders, setEncoders] = useState([]);
  useEffect(() => { invoke("list_available_encoders").then(setEncoders).catch(() => setEncoders([])); }, []);
  const setV = (patch) => apply({ video: { ...video, ...patch } });
  return (
    <Card>
      <Row label={t("settings.video.resolution")}>
        <select className={inputCls} value={video.resolution ?? "native"} onChange={(e) => setV({ resolution: e.target.value })}>
          {RESOLUTION_OPTIONS.map(([val, label]) => <option key={val} value={val}>{label}</option>)}
        </select>
      </Row>
      <Row label={t("settings.video.fps")}>
        <select className={inputCls} value={video.fps ?? 60} onChange={(e) => setV({ fps: Number(e.target.value) })}>
          {FPS_OPTIONS.map((f) => <option key={f} value={f}>{f}</option>)}
        </select>
      </Row>
      <Row label={t("settings.video.encoder")}>
        <select className={inputCls} value={video.encoder ?? "auto"} onChange={(e) => setV({ encoder: e.target.value })}>
          <option value="auto">{ENCODER_LABELS.auto ?? "Auto"}</option>
          {encoders.filter((e) => e.available).map((e) => (
            <option key={e.kind} value={e.kind}>{ENCODER_LABELS[e.kind] ?? e.label ?? e.kind}</option>
          ))}
        </select>
      </Row>
    </Card>
  );
}

// Quick audio subset: the System Audio + Microphone primary device rows,
// mirroring `PrimaryDeviceRow`'s add/remove-source semantics from the full
// Audio settings card.
function AudioQuickStep({ draft, apply, t }) {
  const video = draft.video ?? {};
  const audio = video.audio ?? {};
  const sources = audio.sources ?? [];
  const [devices, setDevices] = useState({ outputs: [], inputs: [] });
  useEffect(() => {
    invoke("list_audio_devices").then(setDevices).catch(() => setDevices({ outputs: [], inputs: [] }));
  }, []);
  const applyAudio = (next) => apply({ video: { ...video, audio: { ...audio, sources: next } } });

  const DeviceRow = ({ kind, label, devs }) => {
    const source = sources.find((s) => s.kind === kind);
    const setDevice = (deviceId) => {
      const idx = sources.findIndex((s) => s.kind === kind);
      const entry = { ...(idx >= 0 ? sources[idx] : {}), kind, device_id: deviceId, label, enabled: true };
      applyAudio(idx >= 0 ? sources.map((s, i) => (i === idx ? entry : s)) : [...sources, entry]);
    };
    const toggle = () => {
      const idx = sources.findIndex((s) => s.kind === kind);
      if (idx >= 0) applyAudio(sources.filter((_, i) => i !== idx));
      else setDevice(devs[0]?.id ?? "");
    };
    return (
      <Row label={label}>
        <div className="flex items-center gap-2">
          {source && (
            <select className={inputCls} value={source.device_id ?? devs[0]?.id ?? ""} onChange={(e) => setDevice(e.target.value)}>
              {devs.length === 0 && <option value="">{t("settings.audio.noDevices")}</option>}
              {devs.map((d) => <option key={d.id} value={d.id}>{d.label}</option>)}
            </select>
          )}
          <Toggle labeled checked={!!source} onChange={toggle} />
        </div>
      </Row>
    );
  };

  return (
    <Card>
      <DeviceRow kind="system_output" label={t("settings.audio.systemAudio")} devs={devices.outputs ?? []} />
      <DeviceRow kind="microphone" label={t("settings.audio.microphone")} devs={devices.inputs ?? []} />
    </Card>
  );
}

export default function Onboarding({ onClose }) {
  const [draft,           setDraft]           = useState(null);
  const [step,            setStep]            = useState(0);
  const [drive,           setDrive]           = useState({ connected: false, email: null, name: null, photo: null });
  const [connecting,      setConnecting]      = useState(false);
  const [driveError,      setDriveError]      = useState("");
  const [hasBuiltinCreds, setHasBuiltinCreds] = useState(true);
  const [legalDoc,        setLegalDoc]        = useState(null);
  const draftRef  = useRef(null);
  const saveTimer = useRef(null);

  const lang = draft?.language ?? "en";
  const t    = useT(lang);
  // Admin/elevation mode only exists on Windows (UAC) — hide that tip elsewhere.
  const isWindows = typeof navigator !== "undefined" && navigator.userAgent.includes("Windows");

  useEffect(() => {
    (async () => {
      const s = await invoke("get_settings");
      draftRef.current = s;
      setDraft(s);
      invoke("has_builtin_credentials").then(setHasBuiltinCreds).catch(() => {});
      invoke("get_drive_status").then(setDrive).catch(() => {});
    })();
  }, []);

  const saveNow = useCallback(async (next) => {
    clearTimeout(saveTimer.current);
    await invoke("save_settings", { settings: next ?? draftRef.current });
  }, []);

  const scheduleSave = useCallback((next) => {
    clearTimeout(saveTimer.current);
    saveTimer.current = setTimeout(() => saveNow(next), 300);
  }, [saveNow]);

  const apply = useCallback((patch) => {
    setDraft((prev) => {
      const next = { ...prev, ...patch };
      draftRef.current = next;
      scheduleSave(next);
      return next;
    });
  }, [scheduleSave]);

  const finish = useCallback(async () => {
    clearTimeout(saveTimer.current);
    const currentVersion = await invoke("get_app_version").catch(() => "");
    const next = { ...draftRef.current, onboarded: true, accepted_legal_version: LEGAL_VERSION, last_seen_version: currentVersion };
    await invoke("save_settings", { settings: next });
    onClose();
  }, [onClose]);

  const connectDrive = async () => {
    setDriveError("");
    setConnecting(true);
    try {
      await invoke("connect_drive", { loginHint: null });
      const status = await invoke("get_drive_status");
      setDrive(status);
      apply({ sync_enabled: true });
    } catch (e) {
      setDriveError(String(e));
    } finally {
      setConnecting(false);
    }
  };

  const stepKey = STEPS[step];
  const isFirst = step === 0;
  const isLast  = step === TOTAL - 1;

  // Backdrop — covers the content area below the gallery TitleBar.
  // Solid (not blurred): backdrop-filter doesn't render over the transparent,
  // vibrancy-backed gallery window on macOS, so it just shows flat gray there.
  return (
    <div className="absolute inset-0 z-[100] flex items-center justify-center bg-black/85">

      {/* Modal card */}
      <div className="relative flex flex-col w-[520px] max-h-[88%] rounded-2xl border border-stone-700/60 bg-stone-950 shadow-2xl shadow-black/80">

        {/* Header */}
        <div className="flex items-start justify-between px-6 pt-6 pb-4 shrink-0">
          <div className="flex flex-col gap-1">
            {draft ? (
              <>
                <h2 className="text-lg font-semibold text-stone-100 leading-tight">
                  {t(`onboarding.steps.${stepKey}.title`)}
                </h2>
                <p className="text-xs text-stone-500">
                  {t("onboarding.stepOf")(step + 1, TOTAL)}
                </p>
              </>
            ) : (
              <h2 className="text-lg font-semibold text-stone-100">…</h2>
            )}
          </div>
          <button onClick={finish}
            className="text-xs text-stone-600 hover:text-stone-400 transition px-2.5 py-1.5 rounded-lg hover:bg-stone-800 shrink-0 mt-0.5">
            {t("onboarding.skip")}
          </button>
        </div>

        {/* Progress bar */}
        <div className="flex items-center gap-1 px-6 pb-4 shrink-0">
          {STEPS.map((_, i) => (
            <div key={i} className={`h-0.5 flex-1 rounded-full transition-all duration-300 ${
              i < step   ? "bg-accent-500" :
              i === step ? "bg-accent-400" :
                           "bg-stone-800"
            }`} />
          ))}
        </div>

        {/* Scrollable body */}
        {draft && (
          <div className="flex-1 overflow-y-auto min-h-0">
            <div className="px-6 pb-2 flex flex-col gap-4">

              <p className="text-sm text-stone-400 leading-relaxed">
                {t(`onboarding.steps.${stepKey}.body`)}
              </p>

              {/* Welcome */}
              {stepKey === "welcome" && (
                <div className="flex justify-center py-4">
                  <img src={logo} alt="" className="h-20 w-20" />
                </div>
              )}

              {/* Language */}
              {stepKey === "language" && (
                <Card>
                  <div className="flex flex-col gap-0.5 py-1">
                    {LANGUAGES.map(({ code, label }) => (
                      <label key={code}
                        className="flex cursor-pointer items-center justify-between px-4 py-2.5 hover:bg-stone-800/50 rounded-lg transition-colors">
                        <span className="text-sm text-stone-200">{label}</span>
                        <input type="radio" name="ob_language" value={code}
                          checked={lang === code}
                          onChange={() => apply({ language: code })}
                          className="accent-sky-500" />
                      </label>
                    ))}
                  </div>
                </Card>
              )}

              {/* Recording mode — same card Settings → Record uses, so the
                  control a user sees here is the exact one they'll find
                  again later, not a onboarding-only rephrasing of it. */}
              {stepKey === "recordingMode" && (
                <div className="flex flex-col gap-3">
                  <RecordingModeCard settings={draft} apply={apply} t={t} />
                  {(draft.video?.replay_buffer?.game_detect_mode ?? "off") !== "off" && (
                    <Card>
                      <Row label={t("settings.replayBuffer.minutes")} hint={t("settings.replayBuffer.minutesHint")}>
                        <div className="flex items-center gap-3">
                          <input type="range" min="1" max="30" step="1"
                            value={draft.video?.replay_buffer?.buffer_minutes ?? 5}
                            onChange={(e) => apply({ video: { ...draft.video, replay_buffer: { ...draft.video?.replay_buffer, buffer_minutes: Number(e.target.value) } } })}
                            className="w-32 accent-accent-500 cursor-pointer" />
                          <span className="text-sm font-medium text-stone-300 w-14 text-right">
                            {draft.video?.replay_buffer?.buffer_minutes ?? 5} min
                          </span>
                        </div>
                      </Row>
                    </Card>
                  )}
                </div>
              )}

              {stepKey === "video" && <VideoQuickStep draft={draft} apply={apply} t={t} />}

              {stepKey === "audio" && <AudioQuickStep draft={draft} apply={apply} t={t} />}

              {/* Shortcuts */}
              {stepKey === "shortcuts" && (
                <div className="flex flex-col gap-3">
                  <div className="flex items-center justify-between rounded-xl border border-stone-800 bg-stone-900 px-4 py-3">
                    <span className="text-sm text-stone-200">{t("settings.shortcuts.title")}</span>
                    <Toggle
                      checked={draft.hotkeys_enabled ?? true}
                      onChange={(v) => apply({ hotkeys_enabled: v })}
                    />
                  </div>
                  {(draft.shortcuts || []).map((slot, idx) => (
                    <ShortcutCard
                      key={slot.id}
                      slot={slot}
                      onChange={(updated) => {
                        const next = draft.shortcuts.map((s, i) => i === idx ? updated : s);
                        apply({ shortcuts: next });
                      }}
                      onRemove={() => apply({ shortcuts: draft.shortcuts.filter((_, i) => i !== idx) })}
                      t={t}
                    />
                  ))}
                  <Button onClick={() => {
                    const id = Date.now().toString(36) + Math.random().toString(36).slice(2, 7);
                    apply({ shortcuts: [...(draft.shortcuts || []), {
                      id, combo: "", capture: "record_window", actions: [], show_in_menu: false, label: "",
                    }]});
                  }}>
                    + {t("settings.shortcuts.addShortcut")}
                  </Button>
                </div>
              )}

              {/* Drive */}
              {stepKey === "drive" && (
                <div className="flex flex-col gap-3">
                  <div className="flex items-center justify-between rounded-xl border border-stone-800 bg-stone-900 px-4 py-3">
                    <span className="text-sm text-stone-200">{t("onboarding.steps.drive.syncLabel")}</span>
                    <Toggle
                      checked={draft.sync_enabled ?? true}
                      onChange={(v) => apply({ sync_enabled: v })}
                    />
                  </div>
                  {drive.connected ? (
                    <div className="rounded-xl border border-emerald-500/30 bg-emerald-500/5 px-4 py-3 flex items-center gap-3">
                      {drive.photo ? (
                        <img src={drive.photo} referrerPolicy="no-referrer"
                          className="w-9 h-9 rounded-full object-cover shrink-0" />
                      ) : (
                        <div className="w-9 h-9 rounded-full bg-gradient-to-br from-violet-500 to-blue-600
                          flex items-center justify-center text-sm font-bold text-white shrink-0">
                          {(drive.name || drive.email || "G")[0].toUpperCase()}
                        </div>
                      )}
                      <div className="min-w-0">
                        <p className="text-sm font-medium text-emerald-300">
                          {t("onboarding.steps.drive.connected")(drive.email || drive.name || "")}
                        </p>
                        {drive.name && drive.email && (
                          <p className="text-xs text-stone-500 truncate">{drive.email}</p>
                        )}
                      </div>
                    </div>
                  ) : (
                    <div className="flex flex-col gap-2">
                      {!hasBuiltinCreds && (
                        <p className="text-xs text-accent-400/80">{t("settings.advanced.credentialsHintNoBuiltin")}</p>
                      )}
                      <Button variant="primary" disabled={connecting || !hasBuiltinCreds} onClick={connectDrive}>
                        {connecting ? t("onboarding.steps.drive.connecting") : t("onboarding.steps.drive.connect")}
                      </Button>
                      {driveError && <p className="text-xs text-red-400">{driveError}</p>}
                      <button onClick={() => setStep((s) => s + 1)}
                        className="text-xs text-stone-500 hover:text-stone-300 transition self-start">
                        {t("onboarding.steps.drive.skip")}
                      </button>
                    </div>
                  )}
                </div>
              )}

              {/* Startup */}
              {stepKey === "startup" && (
                <div className="flex flex-col gap-3">
                  <div className="flex items-center justify-between rounded-xl border border-stone-800 bg-stone-900 px-4 py-3">
                    <span className="text-sm text-stone-200">{t("settings.record.startWithWindows")}</span>
                    <Toggle
                      checked={draft.autostart ?? false}
                      onChange={(v) => apply({ autostart: v })}
                    />
                  </div>
                  {isWindows && (
                    <div className="flex items-center justify-between rounded-xl border border-stone-800 bg-stone-900 px-4 py-3">
                      <div>
                        <p className="text-sm text-stone-200">{t("settings.admin.runAsAdmin")}</p>
                        <p className="text-xs text-stone-500 mt-0.5">{t("settings.admin.runAsAdminHint")}</p>
                      </div>
                      <Toggle
                        checked={draft.run_as_admin ?? false}
                        onChange={(v) => apply({ run_as_admin: v })}
                      />
                    </div>
                  )}
                </div>
              )}

              {/* Finish */}
              {stepKey === "finish" && (
                <div className="flex flex-col items-center gap-3 py-4">
                  <div className="text-4xl select-none">🎉</div>
                  <p className="text-xs text-stone-500 text-center">
                    {t("onboarding.legalNotice")}{" "}
                    <button onClick={() => setLegalDoc("terms")} className="underline hover:text-stone-300">{t("settings.about.terms")}</button>
                    {" & "}
                    <button onClick={() => setLegalDoc("privacy")} className="underline hover:text-stone-300">{t("settings.about.privacy")}</button>
                  </p>
                </div>
              )}

            </div>
          </div>
        )}

        {/* Footer */}
        <div className="flex items-center justify-between gap-3 px-6 py-4 border-t border-stone-800/60 shrink-0">
          <Button onClick={() => setStep((s) => s - 1)} disabled={isFirst || !draft}>
            {t("onboarding.back")}
          </Button>
          {isLast ? (
            <Button variant="primary" onClick={finish} disabled={!draft}>
              {t("onboarding.finish")}
            </Button>
          ) : (
            <Button variant="primary" onClick={() => setStep((s) => s + 1)} disabled={!draft}>
              {t("onboarding.next")}
            </Button>
          )}
        </div>

      </div>

      {legalDoc && (
        <LegalDocModal doc={legalDoc} title={t(`settings.about.${legalDoc}`)} lang={lang} t={t} onClose={() => setLegalDoc(null)} />
      )}
    </div>
  );
}
