import { useCallback, useEffect, useRef, useState } from "react";
import { invoke, listen, emit } from "../lib/tauri.js";
import {
  MdKeyboard, MdVideocam, MdVolumeUp, MdTune, MdInfo, MdSearch, MdSportsEsports, MdNotifications, MdFiberManualRecord, MdStorage, MdMusicNote, MdCalculate, MdAutoAwesome, MdHighQuality, MdReplay, MdLiveTv, MdShield,
} from "react-icons/md";
import { GamesCard } from "../components/GamesCard.jsx";
import { SiGoogledrive, SiYoutube } from "react-icons/si";
import { Toggle, Radio, Row, Card, Button, inputCls } from "../components/settingsUI.jsx";
import PermissionRow from "../components/PermissionRow.jsx";
import { ShortcutCard } from "../components/ShortcutEditor.jsx";
import { RecordSettingsCard, RecordingModeCard, YoutubeLiveSettingsCard, AudioSettingsCard, ReplayBufferCard, NotificationSettingsCard, IndicatorSettingsCard, SoundEffectsCard } from "../components/RecordSettingsCard.jsx";
import SizeCalculatorCard from "../components/SizeCalculatorCard.jsx";
import QualityWizardModal from "../components/QualityWizardModal.jsx";
import { StorageSettingsCard } from "../components/StorageSettingsCard.jsx";
import { Flag } from "../components/Flag.jsx";
import LegalDocModal from "../components/LegalDocModal.jsx";
import WhatsNewModal from "../components/WhatsNewModal.jsx";
import { compareVersions } from "../lib/version.js";
import logo from "../assets/logo.png";

const LANGUAGES = [
  { code: "en", label: "English" },
  { code: "tr", label: "Türkçe" },
];

// Content strings the sidebar search matches against, keyed by page id.
const CONTENT_KEYS = {
  shortcuts: [
    "settings.shortcuts.title", "settings.shortcuts.addShortcut", "settings.shortcuts.noSlots",
    "settings.shortcuts.hint", "settings.shortcuts.moreOptions", "settings.shortcuts.icon",
    "settings.shortcuts.multiMonitor", "settings.shortcuts.showInMenu", "settings.shortcuts.remove",
    "settings.shortcuts.captureRecordWindow", "settings.shortcuts.captureRecordArea",
    "settings.shortcuts.captureRecordMonitor", "settings.shortcuts.captureSaveReplay",
    "settings.shortcuts.captureOpenWheel", "settings.shortcuts.captureRecordWindowHint",
    "settings.shortcuts.captureRecordAreaHint", "settings.shortcuts.captureRecordMonitorHint",
    "settings.shortcuts.captureSaveReplayHint", "settings.shortcuts.captureOpenWheelHint",
  ],
  mode: [
    "settings.recordingMode.title",
    "settings.recordingMode.clips.label", "settings.recordingMode.full_session.label", "settings.recordingMode.off.label",
    "settings.recordingMode.clips.desc", "settings.recordingMode.full_session.desc", "settings.recordingMode.off.desc",
    "settings.recordingMode.youtubeLive", "settings.recordingMode.youtubeLiveHint",
  ],
  quality: [
    "settings.quality.title", "settings.video.refreshEncoders",
    "settings.quality.guideToggle", "settings.quality.captureTitle",
    "settings.quality.sections.video", "settings.quality.sections.encoding", "settings.quality.sections.output",
    "settings.quality.presets.low.label", "settings.quality.presets.standard.label", "settings.quality.presets.high.label", "settings.quality.presets.custom.label",
    "settings.quality.presets.low.desc", "settings.quality.presets.standard.desc", "settings.quality.presets.high.desc", "settings.quality.presets.custom.desc",
    "settings.video.fps", "settings.video.resolution", "settings.video.bitrate", "settings.video.encoder", "settings.video.container", "settings.video.audioCodec",
    "settings.video.unavailable", "settings.video.captureCursor",
    "settings.video.excludeOverlayWindows", "settings.video.excludeOverlayWindowsHint",
    "settings.video.cropTitlebar", "settings.video.cropTitlebarHint",
    "settings.video.minimizedBehavior", "settings.video.minimizedBehaviorHint",
    "settings.video.minimizedOptions.branded", "settings.video.minimizedOptions.black", "settings.video.minimizedOptions.freeze", "settings.video.minimizedOptions.pause",
    "settings.video.hideOverlaysFromCapture", "settings.video.hideOverlaysFromCaptureHint",
    "settings.video.codecGuide.title",
    "settings.video.codecGuide.tabs.encoder", "settings.video.codecGuide.tabs.container", "settings.video.codecGuide.tabs.audio",
    "settings.video.codecGuide.encoders.auto", "settings.video.codecGuide.encoders.nvenc", "settings.video.codecGuide.encoders.amf",
    "settings.video.codecGuide.encoders.qsv", "settings.video.codecGuide.encoders.x264", "settings.video.codecGuide.encoders.x265",
    "settings.video.codecGuide.encoders.svt", "settings.video.codecGuide.encoders.aom",
    "settings.video.codecGuide.codecs.h264", "settings.video.codecGuide.codecs.hevc", "settings.video.codecGuide.codecs.av1",
    "settings.video.codecGuide.containers.mp4", "settings.video.codecGuide.containers.mkv", "settings.video.codecGuide.containers.mov",
    "settings.video.codecGuide.containers.mp4_fragmented", "settings.video.codecGuide.containers.mov_fragmented",
    "settings.video.codecGuide.audio.aac", "settings.video.codecGuide.audio.opus", "settings.video.codecGuide.audio.mp3", "settings.video.codecGuide.audio.flac",
    "settings.video.codecGuide.recommended", "settings.video.codecGuide.hideUnavailable",
  ],
  replay: [
    "settings.replayBuffer.title", "settings.replayBuffer.enable", "settings.replayBuffer.enableHint",
    "settings.replayBuffer.altTabPrivacy", "settings.replayBuffer.altTabPrivacyHint",
    "settings.replayBuffer.confirmSaveOnClose", "settings.replayBuffer.confirmSaveOnCloseHint",
    "settings.replayBuffer.storage", "settings.replayBuffer.storageHint", "settings.replayBuffer.storageDisk", "settings.replayBuffer.storageMemory",
    "settings.replayBuffer.minutes", "settings.replayBuffer.minutesHint",
  ],
  youtube: [
    "settings.youtubeLive.title", "settings.youtubeLive.noLocalRecording",
    "settings.youtubeLive.titleTemplateLabel", "settings.youtubeLive.titleTemplateHint", "settings.youtubeLive.preview",
    "settings.youtubeLive.privacyLabel", "settings.youtubeLive.privacyHint", "settings.youtubeLive.private", "settings.youtubeLive.unlisted", "settings.youtubeLive.public",
    "settings.youtubeLive.maxResolutionLabel", "settings.youtubeLive.maxResolutionHint",
    "settings.youtubeLive.maxBitrateLabel", "settings.youtubeLive.maxBitrateHint",
    "settings.youtubeLive.maxFpsLabel", "settings.youtubeLive.maxFpsHint",
    "settings.youtubeLive.advancedTitle",
    "settings.youtubeLive.keyframeLabel", "settings.youtubeLive.keyframeHint",
    "settings.youtubeLive.bufferLabel", "settings.youtubeLive.bufferHint",
    "settings.youtubeLive.audioCodecLabel", "settings.youtubeLive.audioCodecHint",
    "settings.youtubeLive.audioSampleRateLabel", "settings.youtubeLive.audioSampleRateHint",
    "settings.youtubeLive.table.title", "settings.youtubeLive.table.resolution",
    "settings.youtubeLive.table.av1hevcMin", "settings.youtubeLive.table.av1hevcMax", "settings.youtubeLive.table.h264", "settings.youtubeLive.table.cbrNote",
  ],
  audio: [
    "settings.audio.title", "settings.video.refreshEncoders",
    "settings.audio.separateTracks", "settings.audio.separateTracksOnHint", "settings.audio.separateTracksOffHint",
    "settings.audio.gameOnly", "settings.audio.gameOnlyHint",
    "settings.audio.systemAudio", "settings.audio.microphone",
    "settings.audio.renameTrack", "settings.audio.pickDevice", "settings.audio.addDevice",
    "settings.audio.mainMix", "settings.audio.mainMixHint",
    "settings.audio.apps", "settings.audio.noApps", "settings.audio.appsHint",
    "settings.audio.multiTrackMp4Hint", "settings.audio.noneSelectedHint", "settings.audio.notRunning", "settings.audio.noDevices",
  ],
  games: [
    "settings.games.title", "settings.games.addManual", "settings.games.sync", "settings.games.syncing",
    "settings.games.overrideDefault", "settings.games.overridesTitle", "settings.games.overrideReset",
    "settings.games.modeShort.clips", "settings.games.modeShort.full_session", "settings.games.modeShort.off",
    "settings.games.overrideYoutube", "settings.games.overrideOn", "settings.games.overrideOff", "settings.games.overrideFolder",
    "settings.games.customBadge", "settings.games.playingNow", "settings.games.exeLabel", "settings.games.addExe",
    "settings.games.myGames", "settings.games.allGames", "settings.games.searchPlaceholder",
    "settings.games.view.list", "settings.games.view.grid", "settings.games.emptyHint",
    "settings.games.nameLabel", "settings.games.browseExe", "settings.games.add",
    "settings.video.fps", "settings.video.resolution", "settings.video.bitrate", "settings.video.encoder", "settings.video.container", "settings.video.audioCodec", "settings.video.unavailable",
  ],
  indicator: [
    "settings.indicator.title", "settings.video.hudCorner", "settings.video.hudCornerHint",
    "settings.indicator.recording", "settings.indicator.recordingHint",
    "settings.indicator.buffer", "settings.indicator.bufferHint",
    "settings.indicator.mic", "settings.indicator.micHint",
  ],
  notifications: [
    "settings.notifications.title", "settings.video.toastCorner", "settings.video.toastCornerHint",
    "settings.video.toastCategory.recording", "settings.video.toastCategory.recordingHint",
    "settings.video.toastCategory.session", "settings.video.toastCategory.sessionHint",
    "settings.video.toastCategory.stream", "settings.video.toastCategory.streamHint",
    "settings.video.toastCategory.buffer", "settings.video.toastCategory.bufferHint",
    "settings.video.toastCategory.clip", "settings.video.toastCategory.clipHint",
  ],
  sounds: [
    "settings.sounds.title", "settings.sounds.preset", "settings.sounds.windowsSound", "settings.sounds.custom", "settings.sounds.preview",
    "settings.sounds.recordingStarted", "settings.sounds.recordingStartedHint",
    "settings.sounds.recordingStopped", "settings.sounds.recordingStoppedHint",
    "settings.sounds.bufferStarted", "settings.sounds.bufferStartedHint",
    "settings.sounds.bufferStopped", "settings.sounds.bufferStoppedHint",
    "settings.sounds.clipSaved", "settings.sounds.clipSavedHint",
    "settings.sounds.presets.soft_ping", "settings.sounds.presets.marimba", "settings.sounds.presets.glass",
    "settings.sounds.presets.soft_bell", "settings.sounds.presets.two_tone_up", "settings.sounds.presets.two_tone_down",
    "settings.sounds.presets.coin", "settings.sounds.presets.success", "settings.sounds.presets.alert",
  ],
  drive: [
    "settings.drive.title", "settings.drive.connected_plain", "settings.nav.drive.disconnected",
    "settings.drive.ytChannelUnknown", "settings.drive.ytChannelHint", "settings.drive.ytDisconnectChannel", "settings.drive.ytChangeChannel",
    "settings.drive.ytNotConnected", "settings.drive.ytNotConnectedHint", "settings.drive.ytConnectChannel",
    "settings.drive.sync",
    "settings.drive.folderName", "settings.drive.browseFolders", "settings.drive.selectFolder", "settings.drive.noFolders",
    "settings.drive.capcoveBackup", "settings.drive.folderEmpty",
    "settings.drive.postConnect.title", "settings.drive.postConnect.subtitle", "settings.drive.postConnect.loading",
    "settings.drive.postConnect.use", "settings.drive.postConnect.newDefault", "settings.drive.postConnect.useDefault",
    "settings.drive.postConnect.manualHint", "settings.drive.postConnect.manualPlaceholder", "settings.drive.postConnect.manualConfirm", "settings.drive.postConnect.skip",
    "settings.drive.disconnect", "settings.drive.connecting", "settings.drive.reconnect", "settings.drive.connect",
    "settings.drive.syncNow", "settings.drive.clear", "settings.drive.transferHistory", "settings.drive.noTransfers",
    "settings.drive.driveNote_pre", "settings.drive.driveNote_post",
    "settings.advanced.credentialsTitle", "settings.advanced.credentialsHint", "settings.advanced.credentialsHintNoBuiltin",
    "settings.advanced.mode", "settings.advanced.modeBuiltin", "settings.advanced.modeCustom",
    "settings.advanced.clientId", "settings.advanced.clientSecret", "settings.advanced.clientSecretPlaceholder",
  ],
  storage: [
    "settings.storage.settingsTitle", "settings.storage.refreshing", "settings.storage.folderLocation", "settings.record.browse",
    "settings.storage.usageTitle", "settings.storage.localLimitHint", "settings.storage.noLimit",
    "settings.storage.clips", "settings.storage.recordings", "settings.storage.other",
    "settings.storage.cloudSyncTitle", "settings.storage.cloudSyncDesc", "settings.storage.cloudSyncButton",
    "settings.storage.managementTitle",
    "settings.storage.autoDeleteTitle", "settings.storage.autoDeleteWarning", "settings.storage.autoDeleteDesc", "settings.storage.configureLimitHint",
    "settings.storage.onlyLongTitle", "settings.storage.onlyLongDesc",
    "settings.storage.recycleBinTitle", "settings.storage.recycleBinDesc",
    "settings.storage.keepFavoritesTitle", "settings.storage.keepFavoritesDesc",
    "settings.storage.deletionLog.title", "settings.storage.deletionLog.clear", "settings.storage.deletionLog.empty", "settings.storage.deletionLog.reasonStorageLimit",
  ],
  general: [
    "settings.language.title",
    "settings.startup.title", "settings.record.startWithWindows", "settings.record.openGalleryOnStart",
    "settings.admin.runAsAdmin", "settings.admin.runAsAdminHint", "settings.admin.running", "settings.admin.standard",
    "settings.admin.restartHint", "settings.admin.restartAsAdmin", "settings.admin.autostartNote",
    "settings.rerunSetup", "settings.rerunSetupHint",
  ],
  permissions: [
    "permissions.title", "permissions.body", "permissions.granted", "permissions.denied",
    "permissions.allow", "permissions.openSettings",
    "permissions.borderless_capture.label", "permissions.borderless_capture.desc",
    "permissions.microphone.label", "permissions.microphone.desc",
  ],
  about: [
    "settings.about.version", "settings.about.description", "settings.about.updateManagedByStore",
    "settings.about.autoUpdate", "settings.about.autoUpdateHint", "settings.about.checkForUpdates",
    "settings.about.checking", "settings.about.upToDate", "settings.about.updateError",
    "settings.about.downloadUpdate", "settings.about.downloading", "settings.about.restartNow",
    "settings.about.loadingHistory", "settings.about.releaseHistory",
    "settings.about.developer", "settings.about.platform", "settings.about.platformValue",
    "settings.about.license", "settings.about.licenseValue", "settings.about.terms", "settings.about.privacy",
    "settings.about.github", "settings.about.supportDev", "settings.about.repo", "settings.about.issues",
    "settings.about.creditsTitle", "settings.about.creditsDesc",
  ],
};

function useSettingsContentIndex(t) {
  const index = {};
  for (const [key, keys] of Object.entries(CONTENT_KEYS)) {
    index[key] = keys.map((k) => t(k)).filter((v) => typeof v === "string" && v);
  }
  // Returns `{label, desc, recommended}` objects, not plain strings.
  for (const modeKey of ["full", "localFirst", "manual"]) {
    const mode = t(`settings.drive.syncModes.${modeKey}`);
    if (mode && typeof mode === "object") {
      if (mode.label) index.drive.push(mode.label);
      if (mode.desc) index.drive.push(mode.desc);
    }
  }
  return index;
}

// Grouped nav — only pages backed by real Capcove features. `desc` feeds the
// centered hero header of the content pane. `isPackagedInstall` hides
// "Permissions" outside an MSIX/Store install — the OS consent concept it
// documents (borderless capture, microphone) simply doesn't exist for a plain
// .exe, which never asks for either.
function useNavGroups(t, isPackagedInstall) {
  return [
    {
      label: t("settings.groups.app"),
      items: [
        { id: "general",   label: t("settings.nav.general.label"),   Icon: MdTune,     desc: t("settings.pageDesc.general") },
        { id: "shortcuts", label: t("settings.nav.shortcuts.label"), Icon: MdKeyboard, desc: t("settings.pageDesc.shortcuts") },
        ...(isPackagedInstall
          ? [{ id: "permissions", label: t("settings.nav.permissions.label"), Icon: MdShield, desc: t("settings.pageDesc.permissions") }]
          : []),
        { id: "about",     label: t("settings.nav.about.label"),     Icon: MdInfo,     desc: t("settings.pageDesc.about") },
      ],
    },
    {
      label: t("settings.groups.recording"),
      items: [
        { id: "mode",    label: t("settings.recordingMode.title"), Icon: MdVideocam,      desc: t("settings.pageDesc.mode") },
        { id: "quality", label: t("settings.quality.title"),       Icon: MdHighQuality,   desc: t("settings.pageDesc.quality") },
        { id: "replay",  label: t("settings.replayBuffer.title"),  Icon: MdReplay,        desc: t("settings.pageDesc.replay") },
        { id: "youtube", label: t("settings.youtubeLive.title"),   Icon: MdLiveTv,        desc: t("settings.pageDesc.youtube") },
        { id: "audio",   label: t("settings.audio.title"),         Icon: MdVolumeUp,      desc: t("settings.pageDesc.audio") },
        { id: "games",   label: t("settings.games.title"),         Icon: MdSportsEsports, desc: t("settings.pageDesc.games") },
      ],
    },
    {
      label: t("settings.groups.alerts"),
      items: [
        { id: "indicator",     label: t("settings.nav.indicator.label"),     Icon: MdFiberManualRecord, desc: t("settings.pageDesc.indicator") },
        { id: "notifications", label: t("settings.nav.notifications.label"), Icon: MdNotifications,     desc: t("settings.pageDesc.notifications") },
        { id: "sounds",        label: t("settings.nav.sounds.label"),        Icon: MdMusicNote,         desc: t("settings.pageDesc.sounds") },
      ],
    },
    {
      label: t("settings.groups.cloud"),
      items: [
        { id: "drive", label: t("settings.nav.drive.label"), Icon: SiGoogledrive, desc: t("settings.pageDesc.drive") },
        { id: "storage", label: t("settings.nav.storage.label"), Icon: MdStorage, desc: t("settings.pageDesc.storage") },
      ],
    },
  ];
}

export default function SettingsView({ t, lang, dateLocale, onRerunWizard, requestedPage }) {
  const [settings,          setSettings]          = useState(null);
  const [drive,              setDrive]             = useState({ connected: false, email: null });
  const [transfers,          setTransfers]         = useState({ active: [], queued: [], history: [], queued_count: 0 });
  const [saveError,          setSaveError]         = useState("");
  const [connecting,         setConnecting]        = useState(false);
  const [customCreds,        setCustomCreds]       = useState(false);
  const [hasBuiltinCreds,    setHasBuiltinCreds]   = useState(true);
  const [driveFolders,       setDriveFolders]      = useState(null);
  const [loadingFolders,     setLoadingFolders]    = useState(false);
  const [ytChannel,          setYtChannel]         = useState(null);
  const [page,               setPage]              = useState("general");
  const [search,              setSearch]            = useState("");
  const [calculatorOpen,     setCalculatorOpen]    = useState(false);
  const [wizardOpen,         setWizardOpen]        = useState(false);

  // One-shot navigation request from the host (e.g. the titlebar avatar
  // jumping straight to the Drive page).
  useEffect(() => {
    if (requestedPage?.page) setPage(requestedPage.page);
  }, [requestedPage]);
  const [isElevated,         setIsElevated]        = useState(false);
  const [isPackagedInstall,  setIsPackagedInstall] = useState(false);
  const [capabilities,       setCapabilities]      = useState([]); // [{kind, status, settings_uri}] — see win_util::CapabilityKind
  const isWindows = typeof navigator !== "undefined" && navigator.userAgent.includes("Windows");
  const [postConnectFolders, setPostConnectFolders] = useState(null);
  const [postConnectLoading, setPostConnectLoading] = useState(false);
  const [postConnectManual,  setPostConnectManual]  = useState("");
  const [appVersion,         setAppVersion]         = useState("");
  const [legalDoc,           setLegalDoc]           = useState(null);
  const [updateStatus,       setUpdateStatus]       = useState("idle");
  const [updateInfo,         setUpdateInfo]         = useState(null);
  const [updateError,        setUpdateError]        = useState("");
  const [downloadProgress,   setDownloadProgress]   = useState(null);
  const [historyOpen,        setHistoryOpen]        = useState(false);
  const [history,            setHistory]            = useState(null);
  const [historyLoading,     setHistoryLoading]     = useState(false);
  const settingsRef = useRef(null);
  const saveTimer   = useRef(null);

  const refreshDriveStatus = useCallback(async () => {
    setDrive(await invoke("get_drive_status"));
  }, []);

  // Which capability's row button is mid-action — the OS consent prompt
  // itself can take a beat to appear, so the button shows a spinner instead
  // of looking unresponsive.
  const [pendingCapability, setPendingCapability] = useState(null);

  // Same action a permission row takes in the first-run explainer modal:
  // still askable → real OS prompt; already denied → that capability's
  // Settings page, since Windows won't prompt again.
  const actOnCapability = useCallback(async (kind, status) => {
    if (status === "denied") {
      const cap = capabilities.find((c) => c.kind === kind);
      if (cap) invoke("open_url", { url: cap.settings_uri }).catch(() => {});
      return;
    }
    setPendingCapability(kind);
    try {
      const result = await invoke("request_capability", { kind }).catch(() => "denied");
      setCapabilities((prev) => prev.map((c) => (c.kind === kind ? { ...c, status: result } : c)));
    } finally {
      setPendingCapability(null);
    }
  }, [capabilities]);

  const saveNow = useCallback(async (next) => {
    clearTimeout(saveTimer.current);
    try {
      await invoke("save_settings", { settings: next ?? settingsRef.current });
    } catch (e) {
      setSaveError(t("settings.saveError") + e);
    }
  }, [t]);

  const scheduleSave = useCallback((next) => {
    clearTimeout(saveTimer.current);
    saveTimer.current = setTimeout(() => saveNow(next), 300);
  }, [saveNow]);

  const apply = useCallback((patch) => {
    setSettings((prev) => {
      const next = { ...prev, ...patch };
      settingsRef.current = next;
      scheduleSave(next);
      return next;
    });
  }, [scheduleSave]);

  useEffect(() => {
    let unlisten = [];
    (async () => {
      const s = await invoke("get_settings");
      settingsRef.current = s;
      setSettings(s);

      invoke("has_builtin_credentials").then((v) => {
        setHasBuiltinCreds(v);
        setCustomCreds(!v || !!s.google_client_id);
      }).catch(() => { setCustomCreds(!!s.google_client_id); });
      invoke("get_is_elevated").then(setIsElevated).catch(() => {});
      invoke("is_packaged_install").then(setIsPackagedInstall).catch(() => {});
      invoke("capability_statuses").then(setCapabilities).catch(() => {});
      refreshDriveStatus();
      invoke("get_transfers").then(setTransfers).catch(() => {});
      unlisten.push(await listen("sync-transfers-changed", (event) => setTransfers(event.payload)));
      unlisten.push(await listen("settings-changed", async () => {
        clearTimeout(saveTimer.current);
        const fresh = await invoke("get_settings");
        settingsRef.current = fresh;
        setSettings(fresh);
      }));

      // Dev-only hook for store_screenshots.rs; stripped from prod builds.
      // Fakes a connected Drive account for the settings screenshot scene
      // without ever touching a real one.
      if (import.meta.env.DEV) {
        unlisten.push(await listen("store-screenshot-cmd", ({ payload }) => {
          if (payload?.action === "set-drive-demo") {
            setDrive({ connected: payload.connected, email: payload.email ?? null, name: payload.name ?? null, photo: payload.photo ?? null });
            requestAnimationFrame(() => setTimeout(() => emit("store-screenshot-ready", {}), 50));
          }
        }));
      }
    })();
    return () => unlisten.forEach((u) => u());
  }, [refreshDriveStatus]);

  useEffect(() => {
    invoke("get_app_version").then(setAppVersion).catch(() => {});
    let unlisten;
    (async () => {
      unlisten = await listen("update-download-progress", (event) => {
        setDownloadProgress(event.payload);
      });
    })();
    return () => unlisten?.();
  }, []);

  const checkForUpdate = async () => {
    setUpdateStatus("checking");
    setUpdateError("");
    try {
      const info = await invoke("check_for_update");
      if (info) {
        setUpdateInfo(info);
        setUpdateStatus("available");
      } else {
        setUpdateInfo(null);
        setUpdateStatus("up-to-date");
      }
    } catch (e) {
      setUpdateError(String(e));
      setUpdateStatus("error");
    }
  };

  const downloadUpdate = async () => {
    setUpdateStatus("downloading");
    setDownloadProgress(null);
    setUpdateError("");
    try {
      await invoke("download_and_install_update");
      setUpdateStatus("ready");
    } catch (e) {
      setUpdateError(String(e));
      setUpdateStatus("error");
    }
  };

  const openReleaseHistory = async () => {
    setHistoryOpen(true);
    if (history === null) {
      setHistoryLoading(true);
      try {
        setHistory(await invoke("get_release_history"));
      } catch {
        setHistory([]);
      } finally {
        setHistoryLoading(false);
      }
    }
  };

  const connect = async (loginHint) => {
    const hasCustom = settings.google_client_id?.trim() && settings.google_client_secret?.trim();
    if (!hasBuiltinCreds && !hasCustom) {
      setSaveError(t("settings.advanced.credentialsHintNoBuiltin"));
      return;
    }
    await saveNow();
    setConnecting(true);
    setSaveError("");
    try {
      await invoke("connect_drive", { loginHint });
      setPostConnectLoading(true);
      setPostConnectFolders([]);
      try {
        const folders = await invoke("list_drive_folders");
        setPostConnectFolders(folders);
      } catch {
        setPostConnectFolders([]);
      } finally {
        setPostConnectLoading(false);
      }
    } catch (e) {
      const msg = String(e);
      if (!msg.includes("cancelled")) setSaveError(msg);
    } finally {
      setConnecting(false);
      refreshDriveStatus();
    }
  };

  const cancelConnect = () => { invoke("cancel_drive_connect"); };
  const dismissPostConnect = () => { setPostConnectFolders(null); setPostConnectManual(""); };

  const pickPostConnectFolder = async (name) => {
    if (!name.trim()) return;
    apply({ drive_folder_name: name.trim() });
    dismissPostConnect();
    setTimeout(() => invoke("sync_now"), 400);
  };

  const disconnect = async () => {
    await invoke("disconnect_drive");
    refreshDriveStatus();
  };

  const reconnect = async () => {
    const hint = drive.email;
    await invoke("disconnect_drive");
    await connect(hint);
  };

  // The YouTube channel the dedicated token is bound to (uploads land there).
  useEffect(() => {
    if (!drive.connected || !drive.youtube_dedicated) { setYtChannel(null); return; }
    invoke("get_youtube_channel").then(setYtChannel).catch(() => setYtChannel(null));
  }, [drive.connected, drive.youtube_dedicated]);

  // YouTube-only OAuth into its own token slot; the main Google (Drive +
  // identity) connection stays untouched.
  const connectYtChannel = async () => {
    try {
      await invoke("connect_youtube");
    } catch { /* user closed the browser tab — nothing changed */ }
    setDrive(await invoke("get_drive_status"));
  };

  // Drops the channel connection (YouTube features off until reconnected).
  const disconnectYtChannel = async () => {
    await invoke("disconnect_youtube");
    setDrive(await invoke("get_drive_status"));
  };

  const syncNow = async () => { await invoke("sync_now"); };

  const groups = useNavGroups(t, isPackagedInstall);
  const contentIndex = useSettingsContentIndex(t);
  const needle = search.trim().toLowerCase();
  const contentMatches = (id, needleStr) =>
    contentIndex[id]?.some((s) => s.toLowerCase().includes(needleStr)) ?? false;
  const filteredGroups = needle
    ? groups
        .map((g) => ({
          ...g,
          items: g.items.filter((i) => i.label.toLowerCase().includes(needle) || contentMatches(i.id, needle)),
        }))
        .filter((g) => g.items.length > 0)
    : groups;
  const currentItem = groups.flatMap((g) => g.items).find((i) => i.id === page);

  if (!settings) {
    return <div className="flex flex-1 items-center justify-center text-stone-500 text-sm">{t("settings.loading")}</div>;
  }

  return (
    <div className="flex flex-1 min-h-0 animate-fade-in">
      <div className="flex w-60 shrink-0 flex-col border-r border-stone-800">
        <div className="p-3">
          <div className="relative">
            <MdSearch size={14} className="absolute left-2.5 top-1/2 -translate-y-1/2 text-stone-600" />
            <input value={search} onChange={(e) => setSearch(e.target.value)}
              placeholder={t("settings.searchPlaceholder")}
              className="w-full rounded-lg border border-stone-800 bg-stone-900 py-1.5 pl-8 pr-2.5 text-xs text-stone-200 outline-none transition focus:border-accent-500 placeholder:text-stone-600" />
          </div>
        </div>
        <div className="flex-1 overflow-y-auto px-2 pb-2">
          {filteredGroups.map((g) => (
            <div key={g.label} className="mb-4">
              <div className="mb-1.5 px-3 text-[10px] font-semibold uppercase tracking-widest text-stone-600">{g.label}</div>
              {g.items.map(({ id, label }) => (
                <button key={id} onClick={() => setPage(id)}
                  className={`flex w-full items-center rounded-lg px-3 py-2 text-left text-[13px] transition duration-200 active:scale-[0.97] active:duration-75 ${
                    page === id ? "bg-stone-800 font-medium text-stone-100" : "text-stone-400 hover:bg-stone-800/50 hover:text-stone-200"
                  }`}>
                  <span className="truncate">{label}</span>
                </button>
              ))}
            </div>
          ))}
        </div>
        <div className="border-t border-stone-800 px-4 py-2.5">
          <div className="px-1 text-[10px] leading-relaxed text-stone-600">
            {appVersion && <div>Capcove v{appVersion}</div>}
          </div>
        </div>
      </div>

      <div className="flex-1 overflow-y-auto bg-stone-950">
        {/* `key={page}` forces a remount on every sidebar click, so the
            fade-in replays for each page instead of only on Settings' own
            first mount. */}
        <div key={page} className="mx-auto flex max-w-2xl flex-col gap-4 px-6 py-8 animate-fade-in">
          {currentItem && page !== "about" && (
            <div className="mb-2 flex flex-col items-center gap-2 text-center">
              <currentItem.Icon size={40} className="text-stone-300" />
              <h2 className="text-xl font-bold text-stone-100">{currentItem.label}</h2>
              {currentItem.desc && <p className="max-w-md text-xs leading-relaxed text-stone-500">{currentItem.desc}</p>}
            </div>
          )}

          {page === "shortcuts" && (
            <Card
              title={t("settings.shortcuts.title")}
              right={
                <div className="flex items-center gap-3">
                  <button
                    onClick={() => {
                      const id = Date.now().toString(36) + Math.random().toString(36).slice(2, 7);
                      apply({ shortcuts: [...(settings.shortcuts || []), { id, combo: "", capture: "record_window", actions: [], show_in_menu: false, label: "" }] });
                    }}
                    className="text-xs font-medium text-accent-400 transition hover:text-accent-300"
                  >
                    + {t("settings.shortcuts.addShortcut")}
                  </button>
                  <Toggle labeled checked={settings.hotkeys_enabled} onChange={(v) => apply({ hotkeys_enabled: v })} />
                </div>
              }
            >
              {(!settings.shortcuts || settings.shortcuts.length === 0) ? (
                <div className="py-4 text-center text-xs text-stone-600">{t("settings.shortcuts.noSlots")}</div>
              ) : (
                <div className="flex flex-col gap-2 py-2">
                  {(settings.shortcuts || []).map((slot, idx) => (
                    <ShortcutCard
                      key={slot.id}
                      slot={slot}
                      onChange={(updated) => {
                        const next = settings.shortcuts.map((s, i) => i === idx ? updated : s);
                        apply({ shortcuts: next });
                      }}
                      onRemove={() => apply({ shortcuts: settings.shortcuts.filter((_, i) => i !== idx) })}
                      t={t}
                    />
                  ))}
                </div>
              )}
            </Card>
          )}

          {page === "mode" && <RecordingModeCard settings={settings} apply={apply} t={t} />}

          {page === "quality" && (
            <>
              {/* The two helper tools live here, on the quality page, instead
                  of a shared strip — they only make sense next to these knobs. */}
              <div className="mb-1 flex items-center justify-end gap-2">
                <button type="button" onClick={() => setWizardOpen(true)}
                  className="flex items-center gap-1.5 rounded-lg bg-accent-500/15 px-3 py-1.5 text-xs font-semibold text-accent-400 transition hover:bg-accent-500/25">
                  <MdAutoAwesome size={14} />
                  {t("settings.qualityWizard.title")}
                </button>
                <button type="button" onClick={() => setCalculatorOpen(true)}
                  className="flex items-center gap-1.5 rounded-lg bg-stone-900 px-3 py-1.5 text-xs font-semibold text-stone-400 transition hover:bg-stone-800 hover:text-stone-200">
                  <MdCalculate size={14} />
                  {t("settings.sizeCalculator.title")}
                </button>
              </div>
              <RecordSettingsCard settings={settings} apply={apply} t={t} />
            </>
          )}
          {page === "replay" && <ReplayBufferCard settings={settings} apply={apply} t={t} />}
          {page === "youtube" && <YoutubeLiveSettingsCard settings={settings} apply={apply} t={t} />}
          {calculatorOpen && (
            <SizeCalculatorCard settings={settings} t={t} onClose={() => setCalculatorOpen(false)} />
          )}
          {wizardOpen && (
            <QualityWizardModal settings={settings} t={t} apply={apply} onClose={() => setWizardOpen(false)} />
          )}
          {page === "games" && <GamesCard t={t} lang={lang} folders={settings.recording_folders ?? []} />}
          {page === "indicator" && <IndicatorSettingsCard settings={settings} apply={apply} t={t} />}
          {page === "notifications" && <NotificationSettingsCard settings={settings} apply={apply} t={t} />}
          {page === "sounds" && <SoundEffectsCard settings={settings} apply={apply} t={t} />}
          {page === "audio" && <AudioSettingsCard settings={settings} apply={apply} t={t} />}

          {page === "drive" && <>
          <Card
            title={t("settings.drive.title")}
            right={
              <span className={`text-xs font-medium ${drive.connected ? "text-emerald-400" : "text-stone-500"}`}>
                {drive.connected ? t("settings.drive.connected_plain") : t("settings.nav.drive.disconnected")}
              </span>
            }
          >
            {drive.connected && (
              <div className="flex items-center gap-3 px-3 py-3 mb-1 border-b border-stone-800/60">
                {drive.photo ? (
                  <img src={drive.photo} referrerPolicy="no-referrer"
                    className="w-10 h-10 rounded-full object-cover shrink-0" />
                ) : (
                  <div className="w-10 h-10 rounded-full bg-gradient-to-br from-violet-500 to-blue-600
                    flex items-center justify-center text-sm font-bold text-white shrink-0">
                    {(drive.name || drive.email || "G")[0].toUpperCase()}
                  </div>
                )}
                <div className="flex-1 min-w-0">
                  <p className="text-sm font-semibold text-stone-100 truncate">
                    {drive.name || drive.email || t("settings.drive.connected_plain")}
                  </p>
                  {drive.name && drive.email && (
                    <p className="text-xs text-stone-500 truncate">{drive.email}</p>
                  )}
                </div>
              </div>
            )}
            {drive.connected && (
              <div className="flex items-center gap-3 px-3 py-2.5 mb-1 border-b border-stone-800/60">
                <SiYoutube size={17} className="shrink-0 text-red-500" />
                {drive.youtube_dedicated ? (
                  <>
                    {ytChannel?.thumbnail && (
                      <img src={ytChannel.thumbnail} referrerPolicy="no-referrer"
                        className="h-7 w-7 shrink-0 rounded-full object-cover" />
                    )}
                    <div className="min-w-0 flex-1">
                      <p className="truncate text-xs font-semibold text-stone-100">
                        {ytChannel ? ytChannel.title : t("settings.drive.ytChannelUnknown")}
                      </p>
                      <p className="truncate text-[11px] text-stone-500">{t("settings.drive.ytChannelHint")}</p>
                    </div>
                    <button onClick={disconnectYtChannel}
                      className="shrink-0 text-[11px] font-medium text-stone-500 transition hover:text-red-400">
                      {t("settings.drive.ytDisconnectChannel")}
                    </button>
                    <Button onClick={connectYtChannel}>{t("settings.drive.ytChangeChannel")}</Button>
                  </>
                ) : (
                  <>
                    <div className="min-w-0 flex-1">
                      <p className="truncate text-xs font-semibold text-stone-100">{t("settings.drive.ytNotConnected")}</p>
                      <p className="truncate text-[11px] text-stone-500">{t("settings.drive.ytNotConnectedHint")}</p>
                    </div>
                    <Button variant="primary" onClick={connectYtChannel}>{t("settings.drive.ytConnectChannel")}</Button>
                  </>
                )}
              </div>
            )}
            <Row label={t("settings.drive.sync")}>
              <Toggle labeled checked={settings.sync_enabled} onChange={(v) => apply({ sync_enabled: v })} />
            </Row>

            <div className="flex flex-col gap-1.5 py-2 border-b border-stone-800">
              {[
                { value: "full",        ...t("settings.drive.syncModes.full") },
                { value: "local_first", ...t("settings.drive.syncModes.localFirst") },
                { value: "manual",      ...t("settings.drive.syncModes.manual") },
              ].map(({ value, label, desc, warn }) => (
                <div key={value} onClick={() => apply({ sync_mode: value })}
                  className="flex items-start gap-2.5 cursor-pointer px-3 py-1.5 rounded hover:bg-stone-800/50 transition">
                  <Radio checked={settings.sync_mode === value}
                    onChange={() => apply({ sync_mode: value })}
                    className="mt-0.5" />
                  <span className="flex flex-col">
                    <span className="text-xs text-stone-200 font-medium">{label}</span>
                    <span className="text-[11px] text-stone-500">{desc}</span>
                    {warn && settings.sync_mode === value && (
                      <span className="mt-0.5 text-[11px] text-amber-500/90">⚠ {warn}</span>
                    )}
                  </span>
                </div>
              ))}
            </div>

            <Row label={t("settings.drive.folderName")}>
              <div className="flex gap-1.5 items-center">
                <input type="text" value={settings.drive_folder_name}
                  onChange={(e) => apply({ drive_folder_name: e.target.value || "Capcove" })}
                  className={`${inputCls} w-32`} />
                {drive.connected && (
                  <button
                    onClick={async () => {
                      setLoadingFolders(true);
                      try { setDriveFolders(await invoke("list_drive_folders")); }
                      catch { setDriveFolders([]); }
                      finally { setLoadingFolders(false); }
                    }}
                    className="text-xs px-2 py-1 rounded bg-stone-800 hover:bg-stone-700 text-stone-300 transition whitespace-nowrap"
                  >
                    {loadingFolders ? "…" : t("settings.drive.browseFolders")}
                  </button>
                )}
              </div>
            </Row>

            {driveFolders !== null && (
              <div className="mx-3 mb-2 rounded border border-stone-700 bg-stone-900 overflow-hidden">
                <div className="flex items-center justify-between px-2.5 py-1.5 border-b border-stone-700">
                  <span className="text-xs text-stone-400">{t("settings.drive.selectFolder")}</span>
                  <button onClick={() => setDriveFolders(null)} className="text-stone-600 hover:text-stone-400 text-xs">✕</button>
                </div>
                {driveFolders.length === 0 ? (
                  <p className="text-xs text-stone-500 px-2.5 py-2">{t("settings.drive.noFolders")}</p>
                ) : (
                  <div className="max-h-40 overflow-y-auto">
                    {driveFolders.map((f) => (
                      <button key={f.id}
                        onClick={() => { apply({ drive_folder_name: f.name }); setDriveFolders(null); }}
                        className="w-full flex items-center justify-between px-2.5 py-1.5 text-left hover:bg-stone-800 transition">
                        <span className="text-xs text-stone-200 truncate">{f.name}</span>
                        <span className={`text-[10px] shrink-0 ml-2 ${f.is_capcove ? "text-sky-400" : "text-stone-500"}`}>
                          {f.is_capcove ? t("settings.drive.capcoveBackup") : f.empty ? t("settings.drive.folderEmpty") : ""}
                        </span>
                      </button>
                    ))}
                  </div>
                )}
                <div className="border-t border-stone-700 px-2.5 py-2 flex gap-2">
                  <input
                    type="text"
                    placeholder={t("settings.drive.postConnect.manualPlaceholder")}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" && e.target.value.trim()) {
                        apply({ drive_folder_name: e.target.value.trim() });
                        setDriveFolders(null);
                      }
                    }}
                    className={`${inputCls} flex-1 text-xs`}
                  />
                  <button
                    onMouseDown={(e) => {
                      const input = e.currentTarget.previousSibling;
                      if (input.value.trim()) { apply({ drive_folder_name: input.value.trim() }); setDriveFolders(null); }
                    }}
                    className="rounded px-2 py-1 text-xs bg-stone-700 hover:bg-stone-600 text-stone-300 transition whitespace-nowrap"
                  >{t("settings.drive.postConnect.manualConfirm")}</button>
                </div>
              </div>
            )}

            {postConnectFolders !== null && drive.connected && (
              <div className="my-1 rounded-xl border border-sky-500/30 bg-sky-500/5 overflow-hidden">
                <div className="flex items-center justify-between px-4 py-3 border-b border-sky-500/20">
                  <div>
                    <p className="text-sm font-semibold text-sky-300">{t("settings.drive.postConnect.title")}</p>
                    <p className="text-xs text-stone-400 mt-0.5">{t("settings.drive.postConnect.subtitle")}</p>
                  </div>
                  <button onClick={dismissPostConnect} className="text-stone-500 hover:text-stone-300 transition text-lg leading-none px-1">✕</button>
                </div>

                {postConnectLoading ? (
                  <p className="px-4 py-4 text-xs text-stone-400 animate-pulse">{t("settings.drive.postConnect.loading")}</p>
                ) : (
                  <div className="max-h-52 overflow-y-auto divide-y divide-stone-800/60">
                    {postConnectFolders.map((f) => (
                      <button key={f.id} onClick={() => pickPostConnectFolder(f.name)}
                        className="w-full flex items-center justify-between gap-2 px-4 py-2.5 text-left hover:bg-sky-500/10 transition group">
                        <span className="text-sm text-stone-200 truncate group-hover:text-sky-200 transition">{f.name}</span>
                        <span className="flex items-center gap-2 shrink-0">
                          {f.is_capcove && <span className="text-[10px] text-sky-400 bg-sky-400/10 px-1.5 py-0.5 rounded">{t("settings.drive.capcoveBackup")}</span>}
                          {f.empty     && <span className="text-[10px] text-stone-500">{t("settings.drive.folderEmpty")}</span>}
                          <span className="text-xs text-sky-400 opacity-0 group-hover:opacity-100 transition">{t("settings.drive.postConnect.use")}</span>
                        </span>
                      </button>
                    ))}
                    <button onClick={() => pickPostConnectFolder("Capcove")}
                      className="w-full flex items-center justify-between gap-2 px-4 py-2.5 text-left hover:bg-stone-800/50 transition group">
                      <span className="text-sm text-stone-400 group-hover:text-stone-200 transition">
                        {postConnectFolders.length === 0
                          ? t("settings.drive.postConnect.newDefault")
                          : t("settings.drive.postConnect.useDefault")}
                      </span>
                      <span className="text-xs text-stone-500 group-hover:text-stone-300 transition">{t("settings.drive.postConnect.use")}</span>
                    </button>
                  </div>
                )}

                <div className="border-t border-sky-500/15 px-4 py-3 flex flex-col gap-2">
                  <span className="text-[11px] text-stone-500">{t("settings.drive.postConnect.manualHint")}</span>
                  <div className="flex gap-2">
                    <input
                      type="text"
                      value={postConnectManual}
                      onChange={(e) => setPostConnectManual(e.target.value)}
                      onKeyDown={(e) => { if (e.key === "Enter" && postConnectManual.trim()) pickPostConnectFolder(postConnectManual); }}
                      placeholder={t("settings.drive.postConnect.manualPlaceholder")}
                      className={`${inputCls} flex-1 text-sm`}
                    />
                    <button
                      disabled={!postConnectManual.trim()}
                      onClick={() => pickPostConnectFolder(postConnectManual)}
                      className="rounded-lg px-3 py-1.5 text-sm font-medium bg-sky-600 text-white hover:bg-sky-500 disabled:opacity-40 disabled:cursor-not-allowed transition"
                    >{t("settings.drive.postConnect.manualConfirm")}</button>
                  </div>
                </div>

                <div className="border-t border-sky-500/20 px-4 py-2 flex justify-end">
                  <button onClick={dismissPostConnect} className="text-xs text-stone-500 hover:text-stone-300 transition">
                    {t("settings.drive.postConnect.skip")}
                  </button>
                </div>
              </div>
            )}

            <div className="flex flex-wrap items-center gap-2 py-3">
              {drive.connected ? (
                <>
                  <Button variant="danger" onClick={disconnect}>{t("settings.drive.disconnect")}</Button>
                  <Button variant="primary" disabled={connecting} onClick={reconnect}>
                    {connecting ? t("settings.drive.connecting") : t("settings.drive.reconnect")}
                  </Button>
                </>
              ) : (
                <Button variant="primary" disabled={connecting} onClick={() => connect(null)}>
                  {connecting ? t("settings.drive.connecting") : t("settings.drive.connect")}
                </Button>
              )}
              {connecting && (
                <Button onClick={cancelConnect}>{t("common.cancel")}</Button>
              )}
              <Button onClick={syncNow}>{t("settings.drive.syncNow")}</Button>
              {(transfers.active?.length > 0 || transfers.queued?.length > 0 || transfers.history?.length > 0) && (
                <button
                  onClick={() => setTransfers({ active: [], queued: [], history: [], queued_count: 0 })}
                  className="ml-auto text-xs text-stone-600 hover:text-stone-400 transition"
                >{t("settings.drive.clear")}</button>
              )}
            </div>

            <div className="mb-3 rounded-lg border border-stone-800 bg-stone-950">
              <div className="flex items-center justify-between border-b border-stone-800 px-3 py-2">
                <span className="text-xs font-medium text-stone-400">{t("settings.drive.transferHistory")}</span>
                {(transfers.active?.length > 0 || transfers.queued?.length > 0 || transfers.history?.length > 0) && (
                  <button onClick={() => setTransfers({ active: [], queued: [], history: [], queued_count: 0 })}
                    className="text-xs text-stone-600 hover:text-stone-400 transition">{t("settings.drive.clear")}</button>
                )}
              </div>
              {(!transfers.active?.length && !transfers.queued?.length && !transfers.history?.length) ? (
                <p className="px-3 py-3 text-xs text-stone-600">{t("settings.drive.noTransfers")}</p>
              ) : (
                <div className="max-h-48 overflow-y-auto">
                  {[...(transfers.active || []), ...(transfers.queued || []), ...(transfers.history || [])].map((tr, i) => (
                    <div key={i} className="flex items-start gap-2 border-b border-stone-800/60 px-3 py-1.5 last:border-0">
                      <span className={`mt-0.5 shrink-0 text-xs ${
                        tr.status === "done"      ? "text-emerald-400" :
                        tr.status === "error"     ? "text-red-400"     :
                        tr.status === "uploading" ? "text-blue-400"    : "text-stone-500"
                      }`}>
                        {tr.status === "done" ? "✓" : tr.status === "error" ? "✗" : tr.status === "uploading" ? "↑" : "·"}
                      </span>
                      <div className="min-w-0 flex-1">
                        <p className="truncate text-xs text-stone-300">
                          {tr.file}
                          {tr.file === "File Scan" && tr.total > 0 && ` (${tr.sent}/${tr.total})`}
                        </p>
                        {tr.message && (
                          <p className={`truncate text-xs ${tr.status === "error" ? "text-red-400/80" : "text-stone-500"}`}>{tr.message}</p>
                        )}
                      </div>
                      <span className="shrink-0 text-xs text-stone-600">
                        {new Date(tr.time).toLocaleTimeString(dateLocale, { hour: "2-digit", minute: "2-digit", second: "2-digit" })}
                      </span>
                    </div>
                  ))}
                </div>
              )}
            </div>

            {saveError && <p className="pb-2 text-xs text-red-400">{saveError}</p>}
            <p className="pb-3 text-xs leading-relaxed text-stone-500">
              {t("settings.drive.driveNote_pre")}
              <code className="rounded bg-stone-800 px-1 py-0.5 text-stone-300">drive.file</code>
              {t("settings.drive.driveNote_post")}
            </p>
          </Card>

          <Card title={t("settings.advanced.credentialsTitle")}>
            <div className="py-3 text-xs text-stone-500 leading-relaxed">
              {hasBuiltinCreds ? t("settings.advanced.credentialsHint") : t("settings.advanced.credentialsHintNoBuiltin")}
            </div>
            {hasBuiltinCreds && (
              <Row label={t("settings.advanced.mode")}>
                <select
                  value={customCreds ? "custom" : "builtin"}
                  onChange={(e) => {
                    if (e.target.value === "builtin") {
                      setCustomCreds(false);
                      apply({ google_client_id: "", google_client_secret: "" });
                    } else {
                      setCustomCreds(true);
                    }
                  }}
                  className={`${inputCls} w-44`}
                >
                  <option value="builtin">{t("settings.advanced.modeBuiltin")}</option>
                  <option value="custom">{t("settings.advanced.modeCustom")}</option>
                </select>
              </Row>
            )}
            {customCreds && (
              <>
                <Row label={t("settings.advanced.clientId")}>
                  <input type="text" value={settings.google_client_id}
                    placeholder="xxxx.apps.googleusercontent.com"
                    onChange={(e) => apply({ google_client_id: e.target.value.trim() })}
                    className={`${inputCls} w-52`} />
                </Row>
                <Row label={t("settings.advanced.clientSecret")}>
                  <input type="password" value={settings.google_client_secret}
                    placeholder={t("settings.advanced.clientSecretPlaceholder")}
                    onChange={(e) => apply({ google_client_secret: e.target.value.trim() })}
                    className={`${inputCls} w-52`} />
                </Row>
              </>
            )}
          </Card>
          </>}

          {page === "storage" && (
            <StorageSettingsCard settings={settings} apply={apply} t={t} lang={lang} onOpenDrive={() => setPage("drive")} />
          )}

          {page === "general" && (
            <>
              <Card title={t("settings.language.title")}>
                <div className="flex items-center gap-1.5 py-3">
                  {LANGUAGES.map(({ code, label }) => (
                    <button key={code} onClick={() => apply({ language: code })}
                      className={`flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-xs font-medium transition ${
                        (settings.language ?? "en") === code ? "bg-stone-800 text-stone-100" : "text-stone-500 hover:bg-stone-800/60"
                      }`}>
                      <Flag code={code} /> {label}
                    </button>
                  ))}
                </div>
              </Card>
              <Card title={t("settings.startup.title")}>
                <Row label={t("settings.record.startWithWindows")}>
                  <Toggle labeled checked={settings.autostart} onChange={(v) => apply({ autostart: v })} />
                </Row>
                <Row label={t("settings.record.openGalleryOnStart")}>
                  <Toggle labeled checked={settings.start_with_gallery} onChange={(v) => apply({ start_with_gallery: v })} />
                </Row>
                {isWindows && (
                  <Row
                    label={t("settings.admin.runAsAdmin")}
                    hint={t("settings.admin.runAsAdminHint")}
                  >
                    <span className={`text-xs font-medium px-2 py-0.5 rounded-full ${isElevated ? "bg-emerald-500/20 text-emerald-400" : "bg-stone-800 text-stone-500"}`}>
                      {isElevated ? t("settings.admin.running") : t("settings.admin.standard")}
                    </span>
                    <Toggle labeled checked={settings.run_as_admin ?? false} onChange={(v) => apply({ run_as_admin: v })} />
                  </Row>
                )}
                {isWindows && (settings.run_as_admin ?? false) && !isElevated && (
                  <Row label={t("settings.admin.restartHint")}>
                    <Button variant="primary" onClick={() => invoke("request_admin")}>
                      {t("settings.admin.restartAsAdmin")}
                    </Button>
                  </Row>
                )}
                {isWindows && (settings.run_as_admin ?? false) && (settings.autostart ?? false) && isElevated && (
                  <div className="pb-3 text-xs text-accent-400/80">{t("settings.admin.autostartNote")}</div>
                )}
              </Card>
              <Card title={t("settings.logs.title")}>
                <Row label={t("settings.logs.hint")}>
                  <Button onClick={() => invoke("open_logs")}>
                    {t("settings.logs.open")}
                  </Button>
                </Row>
              </Card>
              <Card title={t("settings.rerunSetup")}>
                <button onClick={onRerunWizard}
                  className="flex w-full items-center gap-3 py-3 text-left transition hover:opacity-80">
                  <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg bg-accent-500/15 text-accent-400 text-base">
                    ✦
                  </span>
                  <span className="text-xs text-stone-500">{t("settings.rerunSetupHint")}</span>
                </button>
              </Card>
            </>
          )}

          {page === "permissions" && isPackagedInstall && (
            <Card title={t("permissions.title")}>
              <p className="pt-3 text-xs text-stone-500">{t("permissions.body")}</p>
              {capabilities.map((c) => (
                <PermissionRow key={c.kind} t={t} capability={c} pending={pendingCapability === c.kind} onAct={actOnCapability} />
              ))}
            </Card>
          )}

          {page === "about" && (
            <div className="flex flex-col items-center gap-5 pt-2 pb-6 text-center">
              <img src={logo} alt="" className="h-20 w-20" />
              <div className="flex flex-col gap-1">
                <h2 className="text-xl font-semibold text-stone-100">Capcove</h2>
                <p className="text-sm text-stone-500">
                  {appVersion ? `${t("settings.about.versionPrefix")} ${appVersion}` : t("settings.about.version")}
                </p>
              </div>
              <p className="max-w-xs text-sm text-stone-400 leading-relaxed">{t("settings.about.description")}</p>

              <div className="w-full rounded-xl border border-stone-800 bg-stone-900 overflow-hidden text-left">
                {isPackagedInstall ? (
                  <div className="px-4 py-2.5 text-[11px] text-stone-500">
                    {t("settings.about.updateManagedByStore")}
                  </div>
                ) : (
                <>
                <div className="flex items-center justify-between px-4 py-2.5 border-b border-stone-800">
                  <div>
                    <p className="text-xs text-stone-300">{t("settings.about.autoUpdate")}</p>
                    <p className="text-[11px] text-stone-600">{t("settings.about.autoUpdateHint")}</p>
                  </div>
                  <Toggle labeled checked={settings.auto_update ?? true} onChange={(v) => apply({ auto_update: v })} />
                </div>

                <div className="px-4 py-3">
                  {updateStatus === "idle" && (
                    <button
                      onClick={checkForUpdate}
                      className="w-full rounded-lg border border-stone-700 bg-stone-800/60 px-3 py-2 text-xs text-stone-300 hover:bg-stone-700/60 hover:text-stone-100 transition-colors"
                    >
                      {t("settings.about.checkForUpdates")}
                    </button>
                  )}
                  {updateStatus === "checking" && (
                    <p className="text-xs text-stone-500">{t("settings.about.checking")}</p>
                  )}
                  {updateStatus === "up-to-date" && (
                    <div className="flex flex-col gap-2">
                      <p className="text-xs text-emerald-400">{t("settings.about.upToDate")}</p>
                      <button
                        onClick={checkForUpdate}
                        className="w-full rounded-lg border border-stone-700 bg-stone-800/60 px-3 py-2 text-xs text-stone-300 hover:bg-stone-700/60 hover:text-stone-100 transition-colors"
                      >
                        {t("settings.about.checkForUpdates")}
                      </button>
                    </div>
                  )}
                  {updateStatus === "error" && (
                    <div className="flex flex-col gap-2">
                      <p className="text-xs text-red-400">{t("settings.about.updateError")}: {updateError}</p>
                      <button
                        onClick={checkForUpdate}
                        className="w-full rounded-lg border border-stone-700 bg-stone-800/60 px-3 py-2 text-xs text-stone-300 hover:bg-stone-700/60 hover:text-stone-100 transition-colors"
                      >
                        {t("settings.about.checkForUpdates")}
                      </button>
                    </div>
                  )}
                  {updateStatus === "available" && updateInfo && (
                    <div className="flex flex-col gap-2">
                      <p className="text-xs text-accent-400">
                        {t("settings.about.versionAvailable").replace("{version}", updateInfo.version)}
                      </p>
                      {updateInfo.body && (
                        <p className="max-h-24 overflow-y-auto whitespace-pre-wrap text-[11px] text-stone-500">
                          {updateInfo.body}
                        </p>
                      )}
                      <button
                        onClick={downloadUpdate}
                        className="w-full rounded-lg border border-accent-700/50 bg-accent-900/20 px-3 py-2 text-xs text-accent-400 hover:bg-accent-800/30 hover:text-accent-300 transition-colors"
                      >
                        {t("settings.about.downloadUpdate")}
                      </button>
                    </div>
                  )}
                  {updateStatus === "downloading" && (
                    <div className="flex flex-col gap-2">
                      <p className="text-xs text-stone-500">{t("settings.about.downloading")}</p>
                      {downloadProgress?.total ? (
                        <div className="h-1.5 w-full overflow-hidden rounded-full bg-stone-800">
                          <div
                            className="h-full bg-accent-500 transition-all"
                            style={{ width: `${Math.min(100, (downloadProgress.downloaded / downloadProgress.total) * 100)}%` }}
                          />
                        </div>
                      ) : null}
                    </div>
                  )}
                  {updateStatus === "ready" && (
                    <p className="text-xs text-emerald-400">{t("settings.about.restartNow")}</p>
                  )}
                </div>
                </>
                )}

                <button
                  onClick={openReleaseHistory}
                  disabled={historyLoading}
                  className="flex w-full items-center justify-between border-t border-stone-800 px-4 py-2.5 text-xs text-stone-400 hover:bg-stone-800/50 hover:text-stone-200 transition-colors disabled:opacity-50"
                >
                  {historyLoading ? t("settings.about.loadingHistory") : t("settings.about.releaseHistory")}
                </button>
              </div>

              {historyOpen && history !== null && (
                <WhatsNewModal
                  releases={[...history]
                    .filter((r) => !appVersion || compareVersions(r.version, appVersion) <= 0)
                    .sort((a, b) => compareVersions(b.version, a.version))}
                  lang={lang}
                  t={t}
                  onClose={() => setHistoryOpen(false)}
                />
              )}

              <div className="w-full rounded-xl border border-stone-800 bg-stone-900 overflow-hidden text-left">
                <div className="flex items-center justify-between px-4 py-2.5 border-b border-stone-800">
                  <span className="text-xs text-stone-500">{t("settings.about.developer")}</span>
                  <span className="text-xs text-stone-300">Alperen Çetin (xacnio)</span>
                </div>
                <div className="flex items-center justify-between px-4 py-2.5 border-b border-stone-800">
                  <span className="text-xs text-stone-500">{t("settings.about.platform")}</span>
                  <span className="text-xs text-stone-300">{t("settings.about.platformValue")}</span>
                </div>
                <div className="flex items-center justify-between px-4 py-2.5">
                  <span className="text-xs text-stone-500">{t("settings.about.license")}</span>
                  <button onClick={() => setLegalDoc("license")}
                    className="text-xs text-stone-300 underline hover:text-stone-100 transition-colors">
                    {t("settings.about.licenseValue")}
                  </button>
                </div>
              </div>

              <div className="flex gap-2 w-full">
                <button
                  onClick={() => setLegalDoc("terms")}
                  className="flex flex-1 items-center justify-center gap-2 rounded-lg border border-stone-700 bg-stone-800/60 px-3 py-2 text-xs text-stone-300 hover:bg-stone-700/60 hover:text-stone-100 transition-colors"
                >
                  {t("settings.about.terms")}
                </button>
                <button
                  onClick={() => setLegalDoc("privacy")}
                  className="flex flex-1 items-center justify-center gap-2 rounded-lg border border-stone-700 bg-stone-800/60 px-3 py-2 text-xs text-stone-300 hover:bg-stone-700/60 hover:text-stone-100 transition-colors"
                >
                  {t("settings.about.privacy")}
                </button>
              </div>

              <div className="flex flex-col gap-2 w-full">
                <div className="flex gap-2">
                  <button
                    onClick={() => invoke("open_url", { url: "https://github.com/xacnio" })}
                    className="flex flex-1 items-center justify-center gap-2 rounded-lg border border-stone-700 bg-stone-800/60 px-3 py-2 text-xs text-stone-300 hover:bg-stone-700/60 hover:text-stone-100 transition-colors"
                  >
                    <svg viewBox="0 0 16 16" className="h-3.5 w-3.5 fill-current shrink-0" aria-hidden="true">
                      <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38
                        0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13
                        -.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66
                        .07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15
                        -.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27
                        .68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12
                        .51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48
                        0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z"/>
                    </svg>
                    {t("settings.about.github")}
                  </button>
                  <button
                    onClick={() => invoke("open_url", { url: "https://buymeacoffee.com/xacnio" })}
                    className="flex flex-1 items-center justify-center gap-2 rounded-lg border border-yellow-700/50 bg-yellow-900/20 px-3 py-2 text-xs text-yellow-400 hover:bg-yellow-800/30 hover:text-yellow-300 transition-colors"
                  >
                    <span className="text-sm leading-none">☕</span>
                    {t("settings.about.supportDev")}
                  </button>
                </div>
                <div className="flex gap-2">
                  <button
                    onClick={() => invoke("open_url", { url: "https://github.com/xacnio/capcove" })}
                    className="flex flex-1 items-center justify-center gap-2 rounded-lg border border-stone-700 bg-stone-800/60 px-3 py-2 text-xs text-stone-300 hover:bg-stone-700/60 hover:text-stone-100 transition-colors"
                  >
                    <svg viewBox="0 0 16 16" className="h-3 w-3 fill-current shrink-0" aria-hidden="true">
                      <path d="M2 2.5A2.5 2.5 0 014.5 0h8.75a.75.75 0 01.75.75v12.5a.75.75 0 01-.75.75h-2.5a.75.75 0 110-1.5h1.75v-2h-8a1 1 0 00-.714 1.7.75.75 0 01-1.072 1.05A2.495 2.495 0 012 11.5v-9zm10.5-1V9h-8c-.356 0-.694.074-1 .208V2.5a1 1 0 011-1h8zM5 12.25v3.25a.25.25 0 00.4.2l1.45-1.087a.25.25 0 01.3 0L8.6 15.7a.25.25 0 00.4-.2v-3.25a.25.25 0 00-.25-.25h-3.5a.25.25 0 00-.25.25z"/>
                    </svg>
                    {t("settings.about.repo")}
                  </button>
                  <button
                    onClick={() => invoke("open_url", { url: "https://github.com/xacnio/capcove/issues" })}
                    className="flex flex-1 items-center justify-center gap-2 rounded-lg border border-stone-700 bg-stone-800/60 px-3 py-2 text-xs text-stone-300 hover:bg-stone-700/60 hover:text-stone-100 transition-colors"
                  >
                    <svg viewBox="0 0 16 16" className="h-3 w-3 fill-current shrink-0" aria-hidden="true">
                      <path d="M8 9.5a1.5 1.5 0 100-3 1.5 1.5 0 000 3z"/><path fillRule="evenodd" d="M8 0a8 8 0 100 16A8 8 0 008 0zM1.5 8a6.5 6.5 0 1113 0 6.5 6.5 0 01-13 0z"/>
                    </svg>
                    {t("settings.about.issues")}
                  </button>
                </div>
              </div>

              <div className="w-full text-left">
                <p className="mb-2 text-xs font-medium text-stone-400 uppercase tracking-wider px-0.5">{t("settings.about.creditsTitle")}</p>
                <p className="mb-3 text-xs text-stone-600">{t("settings.about.creditsDesc")}</p>
                <div className="rounded-xl border border-stone-800 bg-stone-900 overflow-hidden divide-y divide-stone-800/70">
                  {[
                    { name: "Tauri",         license: "MIT / Apache-2.0", url: "https://tauri.app" },
                    { name: "React",         license: "MIT",              url: "https://react.dev" },
                    { name: "Tailwind CSS",  license: "MIT",              url: "https://tailwindcss.com" },
                    { name: "Vite",          license: "MIT",              url: "https://vitejs.dev" },
                    { name: "react-icons",   license: "MIT",              url: "https://react-icons.github.io/react-icons" },
                    { name: "TanStack Virtual", license: "MIT",           url: "https://tanstack.com/virtual" },
                    { name: "marked",        license: "MIT",              url: "https://marked.js.org" },
                    { name: "Manrope",       license: "OFL-1.1",          url: "https://manropefont.com" },
                    { name: "Tokio",         license: "MIT",              url: "https://tokio.rs" },
                    { name: "reqwest",       license: "MIT / Apache-2.0", url: "https://github.com/seanmonstar/reqwest" },
                    { name: "windows-rs",    license: "MIT / Apache-2.0", url: "https://github.com/microsoft/windows-rs" },
                    { name: "windows-capture", license: "MIT",            url: "https://github.com/NiiightmareXD/windows-capture" },
                    { name: "xcap",          license: "MIT",              url: "https://github.com/nashaofu/xcap" },
                    { name: "image-rs",      license: "MIT / Apache-2.0", url: "https://github.com/image-rs/image" },
                    { name: "serde / serde_json", license: "MIT / Apache-2.0", url: "https://serde.rs" },
                    { name: "notify",        license: "MIT / Apache-2.0", url: "https://github.com/notify-rs/notify" },
                    { name: "chrono",        license: "MIT / Apache-2.0", url: "https://github.com/chronotope/chrono" },
                    { name: "keyring-rs",    license: "MIT",              url: "https://github.com/open-source-cooperative/keyring-rs" },
                    { name: "zeroize",       license: "MIT / Apache-2.0", url: "https://github.com/RustCrypto/utils/tree/master/zeroize" },
                    { name: "trash",         license: "MIT / Apache-2.0", url: "https://github.com/Byron/trash-rs" },
                    { name: "FFmpeg",        license: "GPL-3.0 (binary)", url: "https://github.com/BtbN/FFmpeg-Builds" },
                  ].map(({ name, license, url }) => (
                    <button
                      key={name}
                      onClick={() => invoke("open_url", { url })}
                      className="flex w-full items-center justify-between px-4 py-2 hover:bg-stone-800/50 transition-colors group"
                    >
                      <span className="text-xs text-stone-300 group-hover:text-stone-100 transition-colors">{name}</span>
                      <span className="text-[10px] text-stone-600 group-hover:text-stone-500 transition-colors font-mono">{license}</span>
                    </button>
                  ))}
                </div>
              </div>
            </div>
          )}
        </div>
      </div>

      {legalDoc && (
        <LegalDocModal doc={legalDoc} title={t(`settings.about.${legalDoc}`)} lang={lang} t={t} onClose={() => setLegalDoc(null)} />
      )}
    </div>
  );
}
