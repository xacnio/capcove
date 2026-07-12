import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { SiYoutube } from "react-icons/si";
import { MdAdd, MdCropFree, MdWindow, MdFullscreen } from "react-icons/md";
import { invoke, listen, emit } from "../lib/tauri.js";
import { useT } from "../lib/i18n.js";
import { useClockSeconds } from "../lib/useClockSeconds.js";
import * as Icon from "./icons.jsx";
import TitleBar from "../components/TitleBar.jsx";
import Onboarding from "../onboarding/Onboarding.jsx";
import LegalUpdateModal from "../onboarding/LegalUpdateModal.jsx";
import WhatsNewModal from "../components/WhatsNewModal.jsx";
import UpdateAvailableModal from "../components/UpdateAvailableModal.jsx";
import { LEGAL_VERSION } from "../lib/legal.js";
import { compareVersions } from "../lib/version.js";
import VideoGrid from "./VideoGrid.jsx";
import IconRail from "./IconRail.jsx";
import FilterDropdown from "./FilterDropdown.jsx";
import TagManageModal from "./TagManageModal.jsx";
import FolderEditModal from "./FolderEditModal.jsx";
import GameSettingsModal from "./GameSettingsModal.jsx";
import TransferPanel from "./TransferPanel.jsx";
import SettingsView from "./SettingsView.jsx";
import EditorView from "./EditorView.jsx";
import CrashRecoveryModal from "./CrashRecoveryModal.jsx";
import ReplayCrashRecoveryModal from "./ReplayCrashRecoveryModal.jsx";
import PendingClipModal from "./PendingClipModal.jsx";
import StorageSummaryModal from "./StorageSummaryModal.jsx";
import { useAppIcon } from "./appIcons.js";

export default function App() {
  const [settings, setSettings] = useState(null);
  // "gallery" (flat, every video recursively, no folder/game browsing) |
  // "folders" (the original games/folders explorer) | "settings" | "editor"
  const [view, setView] = useState("folders");
  const [editorPath, setEditorPath] = useState(null); // source file for the editor view
  const [openPlayerRequest, setOpenPlayerRequest] = useState(null); // {path, name} | null — dev-only, see store_screenshots.rs's "goto-player"
  const [closePlayerToken, setClosePlayerToken] = useState(0); // dev-only, see store_screenshots.rs's "close-player"
  const [connected, setConnected] = useState(false);
  const [driveEmail, setDriveEmail] = useState(null);
  const [drivePhoto, setDrivePhoto] = useState(null);
  const [settingsRequest, setSettingsRequest] = useState(null); // {page} — one-shot page request for SettingsView
  const [transfers, setTransfers] = useState({ active: [], queued: [], history: [], queued_count: 0 });
  const [showOnboarding, setShowOnboarding] = useState(false);
  const [showLegalUpdate, setShowLegalUpdate] = useState(false);
  const [whatsNewReleases, setWhatsNewReleases] = useState(null);
  const [pendingUpdate, setPendingUpdate] = useState(null);
  const [crashRecovery, setCrashRecovery] = useState(null); // {name, path, outcome} | null — see `recording::check_crash_recovery`
  const [replayCrashRecovery, setReplayCrashRecovery] = useState(null); // {segment_count} | null — see `replay_buffer::stage_replay_buffer_crash_recovery`
  const [pendingClip, setPendingClip] = useState(null); // {game} | null — see `replay_buffer::stop_replay_buffer_for_pending_save`
  const [storageSummary, setStorageSummary] = useState(null); // {new_deletions, over_limit, use_recycle_bin} | null — see `video_thumb::check_storage_startup_summary`
  const [driveFull, setDriveFull] = useState(null); // { pct } | null — Drive ≥90% full, uploads auto-paused
  // Recordings root, formatted for the folder-explorer breadcrumb (e.g.
  // "~/Videos/Capcove/") — see `recordings_root_display`.
  const [recordingsRootDisplay, setRecordingsRootDisplay] = useState("");
  const [tags, setTags] = useState([]);
  const [tagFilter, setTagFilter] = useState(null);
  const [kindFilter, setKindFilter] = useState(null); // null (all) | "video" | "clip"
  const [favoritesOnly, setFavoritesOnly] = useState(false);
  // Explorer-style back/forward history; each entry is `{ game, folder }`.
  // Always updated via `setNav`'s functional form (reading `prev`, never
  // `nav` directly) since some listeners are registered once on mount.
  const [nav, setNav] = useState({ history: [{ game: null, folder: null }], index: 0 });
  const selectedGame = nav.history[nav.index].game;
  const selectedFolderId = nav.history[nav.index].folder;
  const navigateTo = (game, folder) => {
    setNav((prev) => ({
      history: [...prev.history.slice(0, prev.index + 1), { game, folder }],
      index: prev.index + 1,
    }));
  };
  const canGoBack = nav.index > 0;
  const canGoForward = nav.index < nav.history.length - 1;
  const goBack = () => setNav((prev) => (prev.index > 0 ? { ...prev, index: prev.index - 1 } : prev));
  const goForward = () => setNav((prev) => (prev.index < prev.history.length - 1 ? { ...prev, index: prev.index + 1 } : prev));
  // "Up" is a distinct navigation (one level up the hierarchy), not history
  // back — pushes a new entry via `navigateTo` like a tile click would.
  const canGoUp = selectedGame != null || selectedFolderId != null;
  const goUp = () => {
    if (selectedFolderId != null) navigateTo(selectedGame, null);
    else if (selectedGame != null) navigateTo(null, null);
  };
  // "Show in Folder View": navigates to the video's folder, then asks
  // `VideoGrid` to scroll to and briefly ring that card.
  const [highlightVideoName, setHighlightVideoName] = useState(null);
  const showInFolderView = (video) => {
    setView("folders");
    navigateTo(video.app ?? null, video.folder_id ?? null);
    setHighlightVideoName(video.name);
  };
  // Bumped by the breadcrumb's refresh button — `VideoGrid` re-fetches
  // whenever this changes (see its own `refreshToken` effect).
  const [refreshToken, setRefreshToken] = useState(0);
  // Spins the breadcrumb's refresh icon for exactly the reload it triggered
  // (see `VideoGrid.jsx`'s `onRefreshingChange`), not every other reason the
  // video list happens to reload (deletes, tag toggles, sync events, …).
  const [refreshing, setRefreshing] = useState(false);
  // Right-click on a breadcrumb segment — {x, y, run} | null. `run` is
  // whatever `open_*_folder` invoke opening that segment's real directory.
  const [explorerMenu, setExplorerMenu] = useState(null);
  const [search, setSearch] = useState("");
  const [sortBy, setSortBy] = useState("dateDesc");
  const [groupBy, setGroupBy] = useState("day");
  // "xl" (340px cards) is this gallery's original, only-ever size — kept as
  // the default so a user who never touches this control sees no change.
  const [viewMode, setViewMode] = useState(() => localStorage.getItem("capcove.videoViewMode") || "xl");
  const setViewModePersist = (v) => { setViewMode(v); localStorage.setItem("capcove.videoViewMode", v); };
  const [visibleCount, setVisibleCount] = useState(0);
  const [showTagManage, setShowTagManage] = useState(false);
  const [recording, setRecording] = useState(null);
  const [recorderMode, setRecorderMode] = useState(null);
  const [syncing, setSyncing] = useState(false);
  const [showTransfers, setShowTransfers] = useState(false);
  const introCheckedRef = useRef(false);
  // Whichever of the two main views (flat gallery / folder explorer) was
  // active before opening the editor — its "back" button returns here
  // instead of always landing on one specific view.
  const lastGalleryViewRef = useRef("folders");
  useEffect(() => {
    if (view === "gallery" || view === "folders") lastGalleryViewRef.current = view;
  }, [view]);

  // Just opens/closes the panel — no sync trigger.
  const toggleTransfers = () => setShowTransfers((v) => !v);

  // `sync_now` is fire-and-forget on the backend, so this spinner is just an
  // acknowledgement, not a real operation tracker.
  const handleRefreshClick = () => {
    setSyncing(true);
    setTimeout(() => setSyncing(false), 900);
    invoke("sync_now").catch((err) => console.error("sync_now failed:", err));
  };

  const lang = settings?.language ?? "en";
  const t = useT(lang);
  const dateLocale = lang === "tr" ? "tr-TR" : "en-US";

  useEffect(() => {
    invoke("get_recording_status").then(setRecording).catch(() => {});
    let unlisten = [];
    (async () => {
      unlisten.push(await listen("recording-started", (e) => setRecording(e.payload)));
      unlisten.push(await listen("recording-stopped", () => setRecording(null)));
    })();
    return () => unlisten.forEach((u) => u());
  }, []);

  // Replay buffer has no start/stop events (it can be flipped by game
  // detection, settings, or the tray at any time) — poll its status.
  const [replayStatus, setReplayStatus] = useState(null);
  // Set once by the dev-only `set-replay-demo` command below — once frozen,
  // the poll must stop overwriting it with the real (off, on a screenshot
  // rig) status a few seconds later.
  const demoReplayFrozenRef = useRef(false);
  useEffect(() => {
    const load = () => {
      if (demoReplayFrozenRef.current) return;
      invoke("get_replay_buffer_status").then(setReplayStatus).catch(() => {});
    };
    load();
    const id = setInterval(load, 3000);
    return () => clearInterval(id);
  }, []);

  const reloadTags = useCallback(() => { invoke("list_tags").then(setTags).catch(() => {}); }, []);
  useEffect(() => { reloadTags(); }, [reloadTags]);

  // Per-game and per-folder (keyed by `RecordingFolder.id`, see
  // `VideoItem.folder_id`) video counts + up to 4 cover videos for the
  // tiles' thumbnail collage, derived from VideoGrid's full unfiltered list.
  const TILE_COVER_COUNT = 4;
  const [gameCounts, setGameCounts] = useState({});
  const [gameCovers, setGameCovers] = useState({}); // app name -> [{ name, modified }, ...]
  const [folderCounts, setFolderCounts] = useState({});
  const [folderCovers, setFolderCovers] = useState({}); // folder id -> [{ name, modified }, ...]
  // "none" | "partial" | "full" — whether a game/folder's recordings are
  // backed up to Drive at all, so tiles can show the same at-a-glance cloud
  // status a single video card already does.
  const [gameDriveStatus, setGameDriveStatus] = useState({});
  const [folderDriveStatus, setFolderDriveStatus] = useState({});

  // Every video kind has some thumbnail source, not always local ffmpeg —
  // YouTube-hosted entries use the YouTube API, Drive-only cards use Drive's
  // preview. `VideoGrid.jsx`'s `useTileCoverThumbs` picks the right one.
  const handleVideosChanged = useCallback((videos) => {
    const byGame = {};
    const byFolder = {};
    for (const v of videos) {
      const entry = {
        name: v.name, modified: v.modified, kind: v.kind, youtubeId: v.youtube_video_id,
        driveOnly: !!v.drive_only, driveSynced: !!v.drive_synced, driveId: v.drive_id,
      };
      // No detected app (Desktop/monitor/area captures) isn't a game tile at
      // all — those recordings show directly at the folder root instead (see
      // `VideoGrid.jsx`'s `rootOnly` filter).
      if (v.app) (byGame[v.app] ??= []).push(entry);
      if (v.folder_id) (byFolder[v.folder_id] ??= []).push(entry);
    }
    const summarize = (byKey) => {
      const counts = {};
      const covers = {};
      const driveStatus = {};
      for (const [key, list] of Object.entries(byKey)) {
        counts[key] = list.length;
        covers[key] = [...list].sort((a, b) => (b.modified ?? 0) - (a.modified ?? 0)).slice(0, TILE_COVER_COUNT);
        const backedUp = list.filter((e) => e.driveOnly || e.driveSynced).length;
        driveStatus[key] = backedUp === 0 ? "none" : backedUp === list.length ? "full" : "partial";
      }
      return [counts, covers, driveStatus];
    };
    const [gCounts, gCovers, gDriveStatus] = summarize(byGame);
    const [fCounts, fCovers, fDriveStatus] = summarize(byFolder);
    setGameCounts(gCounts);
    setGameCovers(gCovers);
    setGameDriveStatus(gDriveStatus);
    setFolderCounts(fCounts);
    setFolderCovers(fCovers);
    setFolderDriveStatus(fDriveStatus);
  }, []);

  // Explorer-style tiles for a given navigation level — a plain function
  // (not just the current level's memo below) so the breadcrumb's chevron
  // dropdowns can also ask "what's under this other, non-current level"
  // (e.g. every game, while actually inside one of its folders).
  const recordingFolders = settings?.recording_folders ?? [];
  const computeTilesFor = (game, folder) => {
    const folderTile = (f) => ({
      kind: "folder", id: f.id, name: f.name,
      count: folderCounts[f.id] ?? 0,
      covers: folderCovers[f.id] ?? [],
      driveStatus: folderDriveStatus[f.id] ?? "none",
    });
    if (folder != null) return [];
    if (game != null) {
      return recordingFolders.filter((f) => f.game === game).map(folderTile);
    }
    // Root: every detected game plus every global folder, side by side.
    return [
      ...Object.keys(gameCounts).sort().map((app) => ({
        kind: "game", name: app,
        count: gameCounts[app] ?? 0,
        covers: gameCovers[app] ?? [],
        driveStatus: gameDriveStatus[app] ?? "none",
      })),
      ...recordingFolders.filter((f) => !f.game).map(folderTile),
    ];
  };
  // Memoized so unrelated re-renders (e.g. the replay-buffer status poll)
  // don't hand VideoGrid a fresh array reference and reset scroll position.
  const tiles = useMemo(
    () => computeTilesFor(selectedGame, selectedFolderId),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [selectedFolderId, selectedGame, recordingFolders, folderCounts, folderCovers, folderDriveStatus, gameCounts, gameCovers, gameDriveStatus]
  );
  // `null` once fully drilled into a specific folder — controls whether
  // VideoGrid shows the tiles section and the breadcrumb's "+ New folder" button.
  const tilesLabel = selectedFolderId != null ? null : t("gallery.folders.title");
  // The flat "gallery" view shares the "folders" view's whole toolbar/grid,
  // just never drills into a game/folder or shows tiles — see the render's
  // breadcrumb block and the `tiles`/`selectedGame`/`selectedFolderId` props
  // handed to `VideoGrid` below.
  const folderMode = view === "folders";

  const openTile = (tile) => {
    if (tile.kind === "game") navigateTo(tile.name, null);
    else navigateTo(selectedGame, tile.id);
  };
  // Breadcrumb's game icon — falls back to the generic monitor glyph below while it loads.
  const breadcrumbGameIcon = useAppIcon(selectedGame);
  const selectedFolderName = selectedFolderId != null
    ? recordingFolders.find((f) => f.id === selectedFolderId)?.name
    : null;
  // Breadcrumb chevron dropdowns — each shows what's *under the segment to
  // its left*, so you can jump straight to a sibling instead of going back
  // first. Cheap to recompute plainly each render; these lists are tiny.
  const rootTileOptions = computeTilesFor(null, null).map((tl) => ({
    key: tl.kind === "folder" ? tl.id : tl.name,
    label: tl.name,
    onClick: () => (tl.kind === "folder" ? navigateTo(null, tl.id) : navigateTo(tl.name, null)),
  }));
  // Sibling folders one level up: a game's own folders, or every other global folder.
  const gameTileOptions = selectedFolderId != null
    ? (selectedGame != null
        ? recordingFolders.filter((f) => f.game === selectedGame)
        : recordingFolders.filter((f) => !f.game)
      ).map((f) => ({ key: f.id, label: f.name, onClick: () => navigateTo(selectedGame, f.id) }))
    : [];

  // Folder create/edit modal — see `FolderEditModal.jsx`. Create is scoped
  // to wherever the "+ New folder" tile was clicked; edit is reached by
  // right-clicking a folder tile.
  const [folderModal, setFolderModal] = useState(null); // null | {mode:"create",game} | {mode:"edit",folder}
  const openCreateFolder = () => setFolderModal({ mode: "create", game: selectedGame });
  const openEditFolder = (tile) => {
    const folder = recordingFolders.find((f) => f.id === tile.id);
    if (folder) setFolderModal({ mode: "edit", folder });
  };

  // Per-game settings modal — reached by right-clicking a game tile (see
  // `GameSettingsModal.jsx`), a shortcut to the same overrides editor as
  // Settings > Games without hunting through the full games list.
  const [gameSettingsFor, setGameSettingsFor] = useState(null); // null | game display name
  const openGameSettings = (tile) => setGameSettingsFor(tile.name);

  // Right-click on a game/folder tile opens this menu (Show in Explorer /
  // settings) instead of jumping straight into settings. {tile, x, y} | null.
  const [tileMenu, setTileMenu] = useState(null);
  const openTileMenu = (tile, x, y) => setTileMenu({ tile, x, y });
  const revealTileInExplorer = (tile) => {
    if (tile.kind === "game") invoke("open_game_folder", { name: tile.name }).catch(() => {});
    else invoke("open_recording_folder", { folderId: tile.id }).catch(() => {});
  };

  useEffect(() => { document.title = `Capcove — ${t("gallery.title")}`; }, [lang]);

  const refreshDriveStatus = useCallback(async () => {
    const status = await invoke("get_drive_status").catch(() => ({ connected: false }));
    setConnected(status.connected);
    setDriveEmail(status.email ?? null);
    setDrivePhoto(status.photo ?? null);
  }, []);

  // Shows releases newer than the last one seen, up to the current version,
  // so nothing in between is skipped.
  const checkWhatsNew = useCallback(async (s) => {
    const lastSeen = s.last_seen_version;
    const current = await invoke("get_app_version").catch(() => null);
    if (!current) return false;
    if (lastSeen && compareVersions(current, lastSeen) > 0) {
      const releases = await invoke("get_release_history").catch(() => []);
      const pending = releases
        .filter((r) => compareVersions(r.version, lastSeen) > 0 && compareVersions(r.version, current) <= 0)
        .sort((a, b) => compareVersions(b.version, a.version));
      if (pending.length > 0) {
        setWhatsNewReleases(pending);
        return true;
      }
    }
    if (lastSeen !== current) {
      invoke("save_settings", { settings: { ...s, last_seen_version: current } });
    }
    return false;
  }, []);

  // Surfaces an update the startup auto-check already found, once per
  // version, when nothing else is already claiming the screen.
  const checkPendingUpdate = useCallback(async (s) => {
    const info = await invoke("get_pending_update").catch(() => null);
    if (info && info.version !== s.last_notified_update_version) {
      setPendingUpdate(info);
    }
  }, []);

  useEffect(() => { invoke("main_ready").catch(() => {}); }, []);

  // Warns (and reflects the backend's auto-paused uploads) when the connected
  // Drive is ≥90% full. Runs on mount and on every gallery reopen, so it shows
  // "every time you open the app" as long as the condition holds.
  const checkDriveFull = useCallback(async () => {
    try {
      const info = await invoke("get_storage_info");
      if (info?.drive_limit && info.drive_usage / info.drive_limit >= 0.9) {
        setDriveFull({ pct: Math.round((info.drive_usage / info.drive_limit) * 100) });
      } else {
        setDriveFull(null);
      }
    } catch { /* offline / not connected — nothing to warn about */ }
  }, []);

  useEffect(() => {
    let unlisten = [];
    (async () => {
      const s = await invoke("get_settings");
      setSettings(s);
      refreshDriveStatus();
      invoke("get_transfers").then(setTransfers).catch(() => {});
      invoke("recordings_root_display").then(setRecordingsRootDisplay).catch(() => {});
      // Covers the startup race: `check_crash_recovery` can fire before this
      // window is done mounting and listening for it.
      invoke("get_crash_recovery_result").then((r) => { if (r) setCrashRecovery(r); }).catch(() => {});
      invoke("get_replay_crash_recovery_result").then((r) => { if (r) setReplayCrashRecovery(r); }).catch(() => {});
      invoke("get_pending_clip_result").then((r) => { if (r) setPendingClip(r); }).catch(() => {});
      invoke("get_storage_summary_result").then((r) => { if (r) setStorageSummary(r); }).catch(() => {});
      checkDriveFull();

      // The store-screenshot automation (store_screenshots.rs) drives its own
      // scene navigation over a synthetic library — none of the first-run
      // prompts below should ever interrupt it.
      const autoMode = await invoke("is_store_screenshot_mode").catch(() => false);
      if (!introCheckedRef.current) {
        introCheckedRef.current = true;
        if (autoMode) { /* no-op */ }
        else if (!s.onboarded) setShowOnboarding(true);
        else if (s.accepted_legal_version !== LEGAL_VERSION) setShowLegalUpdate(true);
        else checkWhatsNew(s).then((shown) => { if (!shown) checkPendingUpdate(s); });
      }

      invoke("recorder_current_mode").then(setRecorderMode).catch(() => {});
      unlisten.push(await listen("recorder-mode-changed", (e) => setRecorderMode(e.payload ?? null)));
      unlisten.push(await listen("sync-transfers-changed", (e) => setTransfers(e.payload)));
      unlisten.push(await listen("settings-changed", async () => {
        setSettings(await invoke("get_settings"));
        invoke("recordings_root_display").then(setRecordingsRootDisplay).catch(() => {});
      }));
      unlisten.push(await listen("navigate-settings", () => setView("settings")));
      unlisten.push(await listen("crash-recovery", (e) => setCrashRecovery(e.payload)));
      unlisten.push(await listen("replay-crash-recovery", (e) => setReplayCrashRecovery(e.payload)));
      unlisten.push(await listen("replay-buffer-pending-clip", (e) => setPendingClip(e.payload)));
      unlisten.push(await listen("storage-summary", (e) => setStorageSummary(e.payload)));
      // Backend re-checks Drive quota periodically; surface the warning live if
      // it crosses the threshold while the app is open, and clear it if freed.
      unlisten.push(await listen("drive-capacity", (e) => {
        const ratio = e.payload?.ratio;
        if (e.payload?.over && ratio != null) setDriveFull({ pct: Math.round(ratio * 100) });
        else setDriveFull(null);
      }));
      unlisten.push(await listen("show-onboarding", () => {
        setView("folders");
        setShowOnboarding(true);
      }));
      // The gallery window is hidden, not destroyed, on close, so this never
      // remounts — re-check on every reopen instead of only at first mount.
      unlisten.push(await listen("gallery-opened", async () => {
        const s = await invoke("get_settings");
        refreshDriveStatus();
        checkDriveFull();
        if (await invoke("is_store_screenshot_mode").catch(() => false)) return;
        if (!s.onboarded) return;
        if (s.accepted_legal_version !== LEGAL_VERSION) setShowLegalUpdate(true);
        else checkWhatsNew(s).then((shown) => { if (!shown) checkPendingUpdate(s); });
      }));

      // Dev-only hook for store_screenshots.rs; stripped from prod builds.
      if (import.meta.env.DEV) {
        unlisten.push(await listen("store-screenshot-cmd", ({ payload }) => {
          if (payload?.action === "goto-view") setView(payload.view);
          else if (payload?.action === "goto-folder-game") navigateTo(payload.game ?? null, payload.folder ?? null);
          else if (payload?.action === "goto-settings") { setSettingsRequest({ page: payload.page, tab: payload.tab }); setView("settings"); }
          else if (payload?.action === "goto-editor") { setEditorPath(payload.path); setView("editor"); }
          else if (payload?.action === "goto-player") { setView("gallery"); setOpenPlayerRequest({ path: payload.path, name: payload.name }); }
          else if (payload?.action === "close-player") setClosePlayerToken((t) => t + 1);
          else if (payload?.action === "set-view-mode") setViewModePersist(payload.mode);
          else if (payload?.action === "set-replay-demo") {
            demoReplayFrozenRef.current = true;
            setReplayStatus(payload.status ?? null);
          }
          requestAnimationFrame(() => setTimeout(() => emit("store-screenshot-ready", {}), 50));
        }));
        // Tells the Rust side this listener actually exists now — under
        // `npm run tauri dev`, Vite's on-demand cold compile can push this
        // mount several seconds out, and a `store-screenshot-cmd` emitted
        // before this line runs is simply lost (nothing was listening yet).
        emit("store-screenshot-frontend-ready", {});
      }
    })();
    return () => unlisten.forEach((u) => u());
  }, [refreshDriveStatus, checkWhatsNew, checkPendingUpdate]);

  const rerunWizard = () => {
    setView("folders");
    setShowOnboarding(true);
  };

  return (
    <div className="flex h-screen flex-col bg-stone-950 text-stone-100 animate-fade-in">
      <TitleBar
        lang={lang}
        right={
          <>
            <div className="flex items-center gap-0.5 rounded-full bg-stone-900/60 p-0.5">
              {[
                { mode: "area", icon: MdCropFree, label: t("recorder.area") },
                { mode: "window", icon: MdWindow, label: t("recorder.window") },
                { mode: "fullscreen", icon: MdFullscreen, label: t("recorder.fullscreen") },
              ].map(({ mode, icon: ModeIcon, label }) => (
                <button
                  key={mode}
                  onClick={() => invoke("recorder_open_mode", { mode }).catch(() => {})}
                  title={label}
                  className={`flex h-8 w-8 items-center justify-center rounded-full transition ${
                    recorderMode === mode
                      ? "bg-accent-500/15 text-accent-400"
                      : "text-stone-500 hover:bg-stone-800 hover:text-stone-200"
                  }`}
                >
                  <ModeIcon size={17} />
                </button>
              ))}
            </div>
            <div className="relative">
              <button onClick={toggleTransfers}
                title={(transfers.active?.length > 0 || transfers.queued?.length > 0)
                  ? t("gallery.transfer.title")
                  : (connected ? (driveEmail || t("settings.drive.connected_plain")) : t("settings.nav.drive.disconnected"))}
                className="flex h-9 w-9 items-center justify-center rounded-full text-stone-500 transition hover:bg-stone-800 hover:text-stone-200">
                <Icon.Cloud size={18} />
              </button>
              {(transfers.active?.length > 0 || transfers.queued?.length > 0) && (
                <span className="pointer-events-none absolute right-0.5 top-0.5 flex h-4 min-w-4 items-center justify-center rounded-full bg-accent-500 px-1 text-[9px] font-bold text-stone-950">
                  {transfers.active.length + transfers.queued.length}
                </span>
              )}
              {showTransfers && (
                <TransferPanel t={t} transfers={transfers} syncing={syncing} onRefresh={handleRefreshClick} onClose={() => setShowTransfers(false)} />
              )}
            </div>
            <button
              title={connected ? (driveEmail || t("settings.drive.connected_plain")) : t("settings.nav.drive.disconnected")}
              onClick={() => { setSettingsRequest({ page: "drive" }); setView("settings"); }}
              className="flex h-9 w-9 items-center justify-center overflow-hidden rounded-full ring-1 ring-stone-700 transition hover:ring-stone-500"
            >
              {drivePhoto ? (
                <img src={drivePhoto} referrerPolicy="no-referrer" className="h-full w-full object-cover" />
              ) : (
                <span className={`flex h-full w-full items-center justify-center bg-stone-800 text-xs font-bold ${connected ? "text-stone-200" : "text-stone-600"}`}>
                  {(driveEmail || "?")[0].toUpperCase()}
                </span>
              )}
            </button>
          </>
        }
      >
        {recording && <RecordingBadge session={recording} t={t} onStop={() => setRecording(null)} />}
        {replayStatus?.running && <ReplayBufferBadge status={replayStatus} t={t} onStopped={() => setReplayStatus(null)} />}
      </TitleBar>
      <div className="relative flex flex-1 min-h-0">
        {/* IconRail lives here, outside the per-view branches, so its position
            never shifts when switching between gallery/settings/editor. */}
        <IconRail t={t} view={view} settings={settings} onNavigate={setView}
          onOpenStorage={() => { setSettingsRequest({ page: "storage" }); setView("settings"); }} />
        <div className="flex flex-1 min-h-0 flex-col">
        {view === "settings" && (
            <SettingsView t={t} lang={lang} dateLocale={dateLocale} onRerunWizard={rerunWizard} requestedPage={settingsRequest} />
        )}
        {view === "editor" && (
            <EditorView t={t} lang={lang} initialPath={editorPath} onBack={() => setView(lastGalleryViewRef.current)} />
        )}
        {/* Kept mounted (just hidden) when Settings/Editor is active, so
            VideoGrid's list/scroll/selection state survives stepping away. */}
        <div className={`flex flex-1 min-h-0 flex-col ${(view === "folders" || view === "gallery") ? "" : "hidden"}`}>
        {/* "gallery": flat, every video recursively, no game/folder
            browsing — same toolbar and card grid as "folders" (`folderMode`,
            computed above the return), just never drills in or shows tiles
            (see the breadcrumb block and the
            `tiles`/`selectedGame`/`selectedFolderId` props below). */}
        {/* Toolbar: icon actions + compact filter dropdowns on the left;
            clip count, backup pill, sort, search on the right. */}
        <div className="flex flex-wrap items-center justify-between gap-3 border-b border-stone-800 bg-stone-900/40 px-3 py-1.5 shrink-0">
          <div className="flex items-center gap-0.5">
            <button
              title={t("gallery.import")}
              onClick={async () => {
                const p = await invoke("pick_video_file").catch(() => null);
                if (p) { setEditorPath(p); setView("editor"); }
              }}
              className="flex h-8 w-8 items-center justify-center rounded-lg text-stone-400 transition hover:bg-stone-800 hover:text-stone-200"
            >
              <Icon.FilmPlus size={16} />
            </button>
            <button
              title={t("gallery.video.newEdit")}
              onClick={() => { setEditorPath(null); setView("editor"); }}
              className="flex h-8 w-8 items-center justify-center rounded-lg text-stone-400 transition hover:bg-stone-800 hover:text-stone-200"
            >
              <Icon.Crop size={15} />
            </button>
            <div className="mx-1.5 h-5 w-px bg-stone-800" />
            {/* Videos / Clips segmented filter (clip = replay save or editor export) */}
            <div className="mr-1.5 flex items-center rounded-lg bg-stone-900 p-0.5">
              {[
                [null, t("gallery.kindFilter.all")],
                ["video", t("gallery.kindFilter.videos")],
                ["clip", t("gallery.kindFilter.clips")],
              ].map(([value, label]) => (
                <button key={String(value)} onClick={() => setKindFilter(value)}
                  className={`rounded-md px-2.5 py-1 text-xs font-medium transition ${
                    kindFilter === value ? "bg-stone-700 text-stone-100" : "text-stone-500 hover:text-stone-300"
                  }`}>
                  {label}
                </button>
              ))}
            </div>
            <button
              onClick={() => setFavoritesOnly((v) => !v)}
              title={t("gallery.favoritesFilter.title")}
              className={`flex items-center gap-1 rounded-lg px-2 py-1.5 text-xs transition ${
                favoritesOnly ? "bg-amber-500/15 text-amber-400" : "text-stone-400 hover:bg-stone-800 hover:text-stone-200"
              }`}
            >
              <Icon.Star size={14} fill={favoritesOnly ? "currentColor" : "none"} />
            </button>
            <FilterDropdown
              icon={Icon.Tag}
              allLabel={t("gallery.tags.allTags")}
              options={tags.map((tg) => ({ value: tg.id, label: tg.name, color: tg.color }))}
              value={tagFilter}
              onChange={setTagFilter}
            />
            <button onClick={() => setShowTagManage(true)} title={t("gallery.tags.manage")}
              className="flex h-8 w-8 items-center justify-center rounded-lg text-stone-500 transition hover:bg-stone-800 hover:text-stone-200">
              <Icon.Pencil size={13} />
            </button>
          </div>
          <div className="flex items-center gap-2">
            <span className="text-xs font-medium text-stone-300">{t("gallery.clipCount")(visibleCount)}</span>
            <div className="flex items-center gap-0.5 rounded-lg bg-stone-900 p-0.5">
              {[
                { id: "2xl", icon: Icon.GridXXL },
                { id: "xl", icon: Icon.GridXL },
                { id: "large", icon: Icon.GridLarge },
                { id: "medium", icon: Icon.LayoutGrid },
                { id: "small", icon: Icon.GridSmall },
                { id: "list", icon: Icon.Rows },
              ].map(({ id, icon: Icn }) => (
                <button key={id} title={t("gallery.viewModes." + id)} onClick={() => setViewModePersist(id)}
                  className={`flex h-6 w-6 items-center justify-center rounded transition ${
                    viewMode === id ? "bg-stone-700 text-stone-100" : "text-stone-600 hover:text-stone-400"
                  }`}>
                  <Icn size={13} />
                </button>
              ))}
            </div>
            <FilterDropdown
              icon={Icon.ArrowUpDown}
              allowNull={false}
              title={t("gallery.sort")}
              options={Object.entries(t("gallery.sortOptions")).map(([key, label]) => ({ value: key, label }))}
              value={sortBy}
              onChange={setSortBy}
            />
            <FilterDropdown
              icon={Icon.LayoutGrid}
              allowNull={false}
              title={t("gallery.group")}
              options={Object.entries(t("gallery.groupOptions")).map(([key, label]) => ({ value: key, label }))}
              value={groupBy}
              onChange={setGroupBy}
            />
            <div className="relative">
              <Icon.Search size={13} className="absolute left-2.5 top-1/2 -translate-y-1/2 text-stone-600" />
              <input
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                placeholder={t("gallery.search")}
                className="w-36 rounded-lg bg-stone-900 py-1.5 pl-8 pr-2.5 text-xs text-stone-200 outline-none transition placeholder:text-stone-600 focus:bg-stone-800"
              />
            </div>
          </div>
        </div>

        {/* Breadcrumb mirrors the on-disk path in "folders" mode (left-click
            navigates, right-click opens in Explorer); "gallery" is a static label. */}
        <div className="flex items-center gap-1 border-b border-stone-800 bg-stone-900/20 px-3 py-2 shrink-0">
          {folderMode ? (
            <>
              <button onClick={goBack} disabled={!canGoBack} title={t("gallery.folders.back")}
                className="flex h-7 w-7 items-center justify-center rounded-md text-stone-400 transition hover:bg-stone-800 hover:text-stone-200 disabled:opacity-30 disabled:hover:bg-transparent disabled:hover:text-stone-400">
                <Icon.ChevronLeft size={17} />
              </button>
              <button onClick={goForward} disabled={!canGoForward} title={t("gallery.folders.forward")}
                className="flex h-7 w-7 items-center justify-center rounded-md text-stone-400 transition hover:bg-stone-800 hover:text-stone-200 disabled:opacity-30 disabled:hover:bg-transparent disabled:hover:text-stone-400">
                <Icon.ChevronRight size={17} />
              </button>
              <button onClick={goUp} disabled={!canGoUp} title={t("gallery.folders.up")}
                className="flex h-7 w-7 items-center justify-center rounded-md text-stone-400 transition hover:bg-stone-800 hover:text-stone-200 disabled:opacity-30 disabled:hover:bg-transparent disabled:hover:text-stone-400">
                <Icon.ArrowUp size={16} />
              </button>
              <button onClick={() => setRefreshToken((n) => n + 1)} disabled={refreshing} title={t("gallery.folders.refresh")}
                className="flex h-7 w-7 items-center justify-center rounded-md text-stone-400 transition hover:bg-stone-800 hover:text-stone-200 disabled:opacity-60">
                <Icon.Refresh size={16} className={refreshing ? "animate-spin" : ""} />
              </button>
              <div className="mx-1 h-5 w-px bg-stone-800" />
              <button
                onClick={() => navigateTo(null, null)}
                onContextMenu={(e) => {
                  e.preventDefault();
                  setExplorerMenu({ x: e.clientX, y: e.clientY, run: () => invoke("open_videos_folder").catch(() => {}) });
                }}
                className="flex items-center gap-1.5 rounded-md px-1.5 py-1 text-xs font-semibold text-stone-200 transition hover:bg-stone-800"
              >
                <Icon.Folder size={13} className="shrink-0 text-accent-400" />
                {recordingsRootDisplay}
              </button>
              {selectedGame != null && (
                <>
                  <BreadcrumbChevron options={rootTileOptions} current={selectedGame} />
                  <button
                    onClick={() => navigateTo(selectedGame, null)}
                    onContextMenu={(e) => {
                      e.preventDefault();
                      setExplorerMenu({ x: e.clientX, y: e.clientY, run: () => invoke("open_game_folder", { name: selectedGame }).catch(() => {}) });
                    }}
                    className="flex items-center gap-1.5 rounded-md px-1.5 py-1 text-xs font-semibold text-stone-200 transition hover:bg-stone-800"
                  >
                    {breadcrumbGameIcon
                      ? <img src={breadcrumbGameIcon} alt="" className="h-3.5 w-3.5 shrink-0 rounded object-cover" />
                      : <Icon.Monitor size={13} className="shrink-0 text-accent-400" />}
                    {selectedGame}
                  </button>
                </>
              )}
              {selectedFolderName != null && (
                <>
                  <BreadcrumbChevron options={gameTileOptions} current={selectedFolderId} />
                  <button
                    onClick={() => navigateTo(selectedGame, selectedFolderId)}
                    onContextMenu={(e) => {
                      e.preventDefault();
                      setExplorerMenu({ x: e.clientX, y: e.clientY, run: () => invoke("open_recording_folder", { folderId: selectedFolderId }).catch(() => {}) });
                    }}
                    className="flex items-center gap-1.5 rounded-md px-1.5 py-1 text-xs font-semibold text-stone-200 transition hover:bg-stone-800"
                  >
                    <Icon.Folder size={13} className="text-accent-400" /> {selectedFolderName}
                  </button>
                </>
              )}
            </>
          ) : (
            <span className="flex items-center gap-1.5 px-1.5 py-1 text-xs font-semibold text-stone-200">
              <Icon.LayoutGrid size={13} className="text-accent-400" /> {t("gallery.folders.allVideos")}
            </span>
          )}
          {/* Same visibility rule as the tiles section (`tilesLabel`) —
              hidden once inside a specific folder, since folders can't nest
              (and never shown at all in the flat "gallery" view). */}
          {folderMode && tilesLabel != null && (
            <button onClick={openCreateFolder}
              className="ml-auto flex items-center gap-1 rounded-lg px-2 py-1 text-xs font-semibold text-stone-500 transition hover:bg-stone-800 hover:text-stone-200">
              <MdAdd size={14} /> {t("gallery.folderModal.createTile")}
            </button>
          )}
          {explorerMenu && (
            <ExplorerMenu
              x={explorerMenu.x}
              y={explorerMenu.y}
              onOpen={explorerMenu.run}
              onClose={() => setExplorerMenu(null)}
              t={t}
            />
          )}
        </div>

        {/* Main content — VideoGrid owns its own scroll region (needed for
            the virtualizer to measure/scroll it directly). */}
        <VideoGrid
          t={t}
          lang={lang}
          tags={tags}
          recording={recording}
          refreshToken={refreshToken}
          tagFilter={tagFilter}
          kindFilter={kindFilter}
          favoritesOnly={favoritesOnly}
          selectedGame={folderMode ? selectedGame : null}
          selectedFolderId={folderMode ? selectedFolderId : null}
          rootOnly={folderMode}
          tiles={folderMode ? tiles : []}
          tilesLabel={folderMode ? tilesLabel : null}
          onOpenTile={openTile}
          onEditFolder={openEditFolder}
          onOpenGameSettings={openGameSettings}
          search={search}
          sortBy={sortBy}
          onSortChange={setSortBy}
          groupBy={groupBy}
          viewMode={viewMode}
          onVideosChanged={handleVideosChanged}
          onFilteredCount={setVisibleCount}
          onRefreshingChange={setRefreshing}
          onEdit={(p) => { setEditorPath(p); setView("editor"); }}
          onShowInFolderView={showInFolderView}
          onTileContextMenu={openTileMenu}
          highlightName={highlightVideoName}
          onHighlightDone={() => setHighlightVideoName(null)}
          openPlayerRequest={openPlayerRequest}
          onOpenPlayerDone={() => setOpenPlayerRequest(null)}
          closePlayerToken={closePlayerToken}
        />
        </div>
        </div>

        {folderModal && (
          <FolderEditModal
            t={t}
            mode={folderModal.mode}
            game={folderModal.game}
            folder={folderModal.folder}
            onClose={() => setFolderModal(null)}
          />
        )}

        {gameSettingsFor && (
          <GameSettingsModal
            t={t}
            name={gameSettingsFor}
            folders={recordingFolders}
            onClose={() => setGameSettingsFor(null)}
          />
        )}

        {tileMenu && (
          <TileContextMenu
            t={t}
            x={tileMenu.x}
            y={tileMenu.y}
            tile={tileMenu.tile}
            onOpenExplorer={() => revealTileInExplorer(tileMenu.tile)}
            onOpenSettings={() => (tileMenu.tile.kind === "game" ? openGameSettings(tileMenu.tile) : openEditFolder(tileMenu.tile))}
            onClose={() => setTileMenu(null)}
          />
        )}

        {showTagManage && (
          <TagManageModal
            t={t}
            tags={tags}
            onClose={() => setShowTagManage(false)}
            onSave={async (next) => {
              await invoke("save_tags", { tags: next }).catch(() => {});
              setTags(next);
            }}
          />
        )}

        {showOnboarding && (
          <Onboarding onClose={() => setShowOnboarding(false)} />
        )}

        {/* Terms/Privacy re-acceptance after either doc changes */}
        {showLegalUpdate && (
          <LegalUpdateModal t={t} lang={lang} onAccept={async () => {
            setShowLegalUpdate(false);
            const s = await invoke("get_settings");
            const shown = await checkWhatsNew(s);
            if (!shown) checkPendingUpdate(s);
          }} />
        )}

        {/* What's New since the last version the user saw */}
        {whatsNewReleases && (
          <WhatsNewModal releases={whatsNewReleases} lang={lang} t={t} onClose={async () => {
            const s = await invoke("get_settings");
            const current = await invoke("get_app_version").catch(() => null);
            if (current) invoke("save_settings", { settings: { ...s, last_seen_version: current } });
            setWhatsNewReleases(null);
            checkPendingUpdate(s);
          }} />
        )}

        {/* Update found by the startup auto-check */}
        {pendingUpdate && (
          <UpdateAvailableModal info={pendingUpdate} lang={lang} t={t} onClose={async () => {
            const s = await invoke("get_settings");
            invoke("save_settings", { settings: { ...s, last_notified_update_version: pendingUpdate.version } });
            setPendingUpdate(null);
          }} />
        )}

        {/* Capcove closed unexpectedly during a recording last time.
            Replay-buffer recovery takes priority when both are pending, since
            it needs an explicit choice rather than just a dismiss. */}
        {replayCrashRecovery ? (
          <ReplayCrashRecoveryModal result={replayCrashRecovery} t={t} onClose={() => setReplayCrashRecovery(null)} />
        ) : crashRecovery ? (
          <CrashRecoveryModal result={crashRecovery} t={t} onClose={() => setCrashRecovery(null)}
            onGoToSettings={() => { setView("settings"); setSettingsRequest({ page: "quality" }); }} />
        ) : pendingClip ? (
          <PendingClipModal result={pendingClip} t={t} onClose={() => setPendingClip(null)} />
        ) : storageSummary ? (
          <StorageSummaryModal result={storageSummary} t={t} onClose={() => setStorageSummary(null)}
            onGoToSettings={() => { setView("settings"); setSettingsRequest({ page: "storage" }); }} />
        ) : driveFull && (
          <div className="fixed inset-0 z-[80] flex items-center justify-center bg-black/60 p-6" onClick={() => setDriveFull(null)}>
            <div className="w-full max-w-sm rounded-xl border border-stone-800 bg-stone-900 p-5" onClick={(e) => e.stopPropagation()}>
              <div className="mb-2 flex items-center gap-2.5">
                <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-red-500/15 text-red-400">
                  <Icon.Cloud size={18} />
                </span>
                <div className="text-sm font-semibold text-stone-100">{t("gallery.driveFull.title")}</div>
              </div>
              <div className="mb-4 text-[13px] leading-relaxed text-stone-400">{t("gallery.driveFull.body")(driveFull.pct)}</div>
              <div className="flex justify-end gap-2">
                <button onClick={() => setDriveFull(null)}
                  className="rounded-lg px-3.5 py-1.5 text-[13px] font-medium text-stone-300 transition hover:bg-stone-800">
                  {t("gallery.driveFull.dismiss")}
                </button>
                <button onClick={() => { setDriveFull(null); setView("settings"); setSettingsRequest({ page: "storage" }); }}
                  className="rounded-lg bg-accent-400 px-3.5 py-1.5 text-[13px] font-medium text-stone-950 transition hover:bg-accent-300">
                  {t("gallery.driveFull.manage")}
                </button>
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

// Breadcrumb separator: a chevron that flips down and opens a dropdown of
// whatever's under the segment to its left (siblings of the segment to its
// right) — jump straight across instead of going back up first. Same
// click-outside-to-close pattern as `FilterDropdown.jsx`.
function BreadcrumbChevron({ options, current }) {
  const [open, setOpen] = useState(false);
  const ref = useRef(null);

  useEffect(() => {
    if (!open) return;
    const onClickOutside = (e) => { if (ref.current && !ref.current.contains(e.target)) setOpen(false); };
    document.addEventListener("mousedown", onClickOutside);
    return () => document.removeEventListener("mousedown", onClickOutside);
  }, [open]);

  return (
    <div className="relative" ref={ref}>
      <button
        onClick={() => setOpen((o) => !o)}
        className="flex h-5 w-5 items-center justify-center rounded text-stone-600 transition hover:bg-stone-800 hover:text-stone-300"
      >
        <Icon.ChevronRight size={12} className={`transition-transform ${open ? "rotate-90" : ""}`} />
      </button>
      {open && (
        <div className="absolute left-0 top-6 z-30 max-h-72 w-52 overflow-y-auto rounded-lg border border-stone-700 bg-stone-900 p-1.5 shadow-xl">
          {options.length === 0 ? (
            <div className="px-2 py-1.5 text-[11px] text-stone-600">—</div>
          ) : (
            options.map((o) => (
              <button
                key={o.key}
                onClick={() => { o.onClick(); setOpen(false); }}
                className={`flex w-full items-center gap-2 truncate rounded-md px-2 py-1.5 text-left text-xs transition ${
                  o.key === current ? "bg-accent-500/15 text-accent-300" : "text-stone-300 hover:bg-stone-800"
                }`}
              >
                <Icon.Folder size={12} className="shrink-0 text-accent-400" />
                <span className="truncate">{o.label}</span>
              </button>
            ))
          )}
        </div>
      )}
    </div>
  );
}

// Right-click menu for a breadcrumb segment — just "Open in Explorer" for
// now, portalled to escape the breadcrumb bar's own stacking/clipping.
function ExplorerMenu({ x, y, onOpen, onClose, t }) {
  const ref = useRef(null);

  useEffect(() => {
    const onClickOutside = (e) => { if (ref.current && !ref.current.contains(e.target)) onClose(); };
    document.addEventListener("mousedown", onClickOutside);
    return () => document.removeEventListener("mousedown", onClickOutside);
  }, [onClose]);

  return createPortal(
    <div
      ref={ref}
      style={{ position: "fixed", left: x, top: y }}
      className="z-[100] w-48 rounded-lg border border-stone-700 bg-stone-900 py-1.5 shadow-2xl"
    >
      <button
        onClick={() => { onOpen(); onClose(); }}
        className="flex w-full items-center gap-2 px-3 py-2 text-left text-xs text-stone-200 transition hover:bg-stone-800"
      >
        <Icon.Folder size={13} className="text-accent-400" /> {t("gallery.folders.openInExplorer")}
      </button>
    </div>,
    document.body
  );
}

// Right-click menu for a game/folder tile: reveal its real directory in
// Explorer, or open its settings (per-game overrides, or folder edit) — the
// latter used to fire directly on right-click, with no way to reach Explorer.
function TileContextMenu({ x, y, tile, onOpenExplorer, onOpenSettings, onClose, t }) {
  const ref = useRef(null);
  const isGame = tile.kind === "game";

  useEffect(() => {
    const onClickOutside = (e) => { if (ref.current && !ref.current.contains(e.target)) onClose(); };
    document.addEventListener("mousedown", onClickOutside);
    return () => document.removeEventListener("mousedown", onClickOutside);
  }, [onClose]);

  const left = Math.min(x, window.innerWidth - 192 - 8);
  const top = Math.min(y, window.innerHeight - 88);

  return createPortal(
    <div ref={ref} style={{ position: "fixed", left, top }}
      className="z-[100] w-48 rounded-lg border border-stone-700 bg-stone-900 py-1.5 shadow-2xl" onClick={(e) => e.stopPropagation()}>
      <button onClick={() => { onOpenExplorer(); onClose(); }}
        className="flex w-full items-center gap-2 px-3 py-2 text-left text-xs text-stone-200 transition hover:bg-stone-800">
        <Icon.Folder size={13} className="text-accent-400" /> {t("gallery.folders.openInExplorer")}
      </button>
      <button onClick={() => { onOpenSettings(); onClose(); }}
        className="flex w-full items-center gap-2 px-3 py-2 text-left text-xs text-stone-200 transition hover:bg-stone-800">
        <Icon.Gear size={13} className="text-stone-400" /> {t(isGame ? "gallery.folders.gameSettings" : "gallery.folders.folderSettings")}
      </button>
    </div>,
    document.body
  );
}

// Status block: the game's wide cover art as a fading backdrop behind the
// two-line status text. Falls back to the square icon, then a tinted pill.
function GameStatusBlock({ appName, dotClass, line1, line1Class, tint, children }) {
  const cover = useAppIcon(appName ? `${appName}__cover` : null);
  const icon = useAppIcon(appName);
  const art = cover || icon;

  return (
    <div className={`relative flex h-full min-w-[240px] items-center gap-2 overflow-hidden ${art ? "" : tint}`}>
      {art && (
        <>
          <img src={art} alt="" className="absolute inset-0 h-full w-full object-cover" />
          <div className="absolute inset-0 bg-gradient-to-r from-stone-950/55 via-stone-950/75 to-stone-950" />
        </>
      )}
      <div className="relative z-10 flex min-w-0 items-center gap-2.5 px-3.5 py-1.5">
        <span className={`h-2.5 w-2.5 shrink-0 rounded-full ${dotClass}`} />
        <div className="min-w-0 leading-snug">
          {/* Small status line on top, the game name as the prominent line under it.
              `tabular-nums` fixes the digit width so the live timer counting up
              doesn't reflow the line and jiggle the buttons to its right. */}
          <div className={`text-[11px] font-medium tabular-nums ${line1Class}`}>{line1}</div>
          {appName && <div className="max-w-[200px] truncate text-sm font-bold text-stone-100 drop-shadow">{appName}</div>}
        </div>
        {children}
      </div>
    </div>
  );
}

// Two-line live status in the titlebar, over the game's catalog cover art
// when the icon cache has it.
function RecordingBadge({ session, t, onStop }) {
  const now = useClockSeconds();
  const [paused, setPaused] = useState(false);
  useEffect(() => {
    invoke("get_recording_paused").then(setPaused).catch(() => {});
  }, []);
  const elapsed = Math.max(0, Math.floor(now / 1000) - session.started_at);
  const mins = Math.floor(elapsed / 60);
  const secs = elapsed % 60;
  // RecordTarget is serde-externally-tagged: `{ window: {..} }` / `"monitor"` / `{ area: {..} }`.
  const appName = session.target?.window?.app;

  const togglePause = () => {
    const next = !paused;
    setPaused(next);
    invoke("pause_recording", { paused: next }).catch(() => setPaused(!next));
  };

  return (
    <GameStatusBlock
      appName={appName}
      dotClass={paused ? "bg-amber-400" : "animate-pulse bg-red-500"}
      line1={
        <span className="inline-flex items-center gap-1">
          {paused
            ? (t("gallery.recording.paused") ?? "Paused")
            : `${t("gallery.recording.active") ?? "Recording"} · ${mins}:${secs.toString().padStart(2, "0")}`}
          {session.live && <SiYoutube size={11} className="text-red-500" title={t("gallery.recording.live") ?? "Streaming live"} />}
        </span>
      }
      line1Class={paused ? "text-amber-300" : "text-red-300"}
      tint={`rounded-lg ${paused ? "bg-amber-500/10" : "bg-red-500/10"}`}
    >
      <button
        onClick={togglePause}
        title={paused ? (t("gallery.recording.resume") ?? "Resume") : (t("gallery.recording.pause") ?? "Pause")}
        className="relative z-10 flex h-6 w-6 items-center justify-center rounded-md bg-stone-700/60 text-stone-100 transition hover:bg-stone-600"
      >
        {paused ? <Icon.Play size={12} /> : <Icon.Pause size={12} />}
      </button>
      <button
        onClick={() => invoke("stop_recording").then(onStop).catch(() => {})}
        className="relative z-10 rounded-md bg-red-500/30 px-2.5 py-1 text-xs font-semibold text-red-100 transition hover:bg-red-500/50"
      >
        {t("gallery.recording.stop") ?? "Stop"}
      </button>
    </GameStatusBlock>
  );
}

// Clip-buffer live status over the buffered game's cover art: how much
// footage is buffered right now, plus a one-click "Save" that flushes the
// buffered window to a file (same as the Save Replay hotkey).
function ReplayBufferBadge({ status, t, onStopped }) {
  const [saving, setSaving] = useState(false);
  const now = useClockSeconds();
  // `buffered_seconds` only refreshes on a 3s status poll, so shown raw it
  // jumps 3 at a time. Derive it from `started_at` and tick locally each
  // second instead (capped at the buffer length so it stops where the backend
  // does once the buffer is full), falling back to the raw value pre-start.
  const secs = status.started_at != null
    ? Math.min(status.max_seconds ?? Infinity, Math.max(0, Math.floor(now / 1000) - status.started_at))
    : (status.buffered_seconds ?? 0);
  const mins = Math.floor(secs / 60);
  const rem = secs % 60;

  const save = async () => {
    setSaving(true);
    try { await invoke("save_replay"); } catch {}
    setSaving(false);
  };

  return (
    <GameStatusBlock
      appName={status.app}
      dotClass="bg-accent-400"
      line1={`${t("gallery.replay.active")} · ${mins}:${rem.toString().padStart(2, "0")}`}
      line1Class="text-accent-300"
      tint="rounded-lg bg-accent-500/10"
    >
      <button
        onClick={save}
        disabled={saving}
        className="relative z-10 rounded-md bg-accent-500/30 px-2.5 py-1 text-xs font-semibold text-accent-100 transition hover:bg-accent-500/50 disabled:opacity-50"
      >
        {saving ? "…" : t("gallery.replay.save")}
      </button>
      <button
        onClick={() => invoke("stop_replay_buffer").then(onStopped).catch(() => {})}
        title={t("gallery.replay.stop")}
        className="relative z-10 flex h-6 w-6 items-center justify-center rounded-md bg-stone-700/60 text-stone-300 transition hover:bg-stone-600 hover:text-red-300"
      >
        <Icon.Square size={10} />
      </button>
    </GameStatusBlock>
  );
}

