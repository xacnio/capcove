import { useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { invoke, listen } from "../lib/tauri.js";
import { MdSearch, MdSportsEsports, MdSync, MdAdd, MdClose, MdGridView, MdViewList, MdChevronRight } from "react-icons/md";
import { Toggle, Card, Button, inputCls, OverrideTile } from "./settingsUI.jsx";
import { FPS_OPTIONS, BITRATE_OPTIONS, CONTAINER_OPTIONS, CONTAINER_LABELS, ENCODER_GROUPS, ENCODER_LABELS, AUDIO_CODECS, RESOLUTION_OPTIONS, RESOLUTION_LABELS } from "./RecordSettingsCard.jsx";
import { relativeTime } from "../lib/relativeTime.js";

// Module-level icon cache for game rows: exe(lower) → dataURL | null(missing).
// Separate from appIcons.js because catalog rows resolve by exe via
// fetch_game_icon; custom games fall back to get_app_icon-by-name instead.
const iconCache = new Map();
const iconPending = new Map();

// Shared lazy-image plumbing for both the square icon and the wide cover:
// one module-level cache keyed by `<kind>:<exe>`, one in-flight promise per
// key so a row scrolled past twice doesn't fetch twice.
function useLazyGameImage(cacheKey, shouldFetch, fetcher) {
  const [img, setImg] = useState(() => iconCache.get(cacheKey) ?? null);

  useEffect(() => {
    if (iconCache.has(cacheKey)) { setImg(iconCache.get(cacheKey) ?? null); return; }
    if (!shouldFetch) { setImg(null); return; }
    let p = iconPending.get(cacheKey);
    if (!p) {
      p = fetcher()
        // Backend returns a complete data URL (webp from the embedded pack,
        // png otherwise).
        .then((v) => (v.startsWith("data:") ? v : `data:image/png;base64,${v}`))
        .catch(() => null)
        .then((v) => { iconCache.set(cacheKey, v); iconPending.delete(cacheKey); return v; });
      iconPending.set(cacheKey, p);
    }
    let cancelled = false;
    p.then((v) => { if (!cancelled) setImg(v); });
    return () => { cancelled = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [cacheKey, shouldFetch]);

  return img;
}

function useGameIcon(exe, hasIconUrl, name, isCustom) {
  // Custom games aren't in the catalog; their icon is cached to disk under
  // the game's display *name*, the same key `get_app_icon` reads by.
  return useLazyGameImage(
    `icon:${exe.toLowerCase()}`,
    isCustom || hasIconUrl,
    () => (isCustom ? invoke("get_app_icon", { appName: name }) : invoke("fetch_game_icon", { exe }))
  );
}

// Wide catalog cover art, used as the row's fading backdrop — cached
// backend-side under `<name>__cover`, so a cover fetched here is reused everywhere.
function useGameCover(exe, hasCoverUrl) {
  return useLazyGameImage(
    `cover:${exe.toLowerCase()}`,
    hasCoverUrl,
    () => invoke("fetch_game_cover", { exe })
  );
}

// Fixed heights the virtualizer must be able to predict: a list row, one
// exe sub-row inside the detail modal, and a grid-mode tile row (poster +
// caption + gap).
const ROW_H = 58;
const SUB_H = 34;
const GRID_ROW_H = 200;

// Per-game capture overrides: recording-mode pills + the quality knobs as
// mini tiles, everything defaulting to the global settings. Exported so the
// gallery's folder-tile right-click can reuse it in a standalone modal
// without pulling in the whole Settings > Games list.
export function OverridesPanel({ game, t, onSet, onReset, folders = [], encoderAvailability = {} }) {
  const ov = game.overrides ?? {};
  const rbov = ov.replay_buffer_video ?? {};
  const def = <option value="">{t("settings.games.overrideDefault")}</option>;
  const defaultLabel = t("settings.games.overrideDefault");
  const setCount = Object.values(ov).filter((v) => v != null).length;

  // Nested override: collapses back to `null` once every sub-field is
  // unset again, so `setCount`/`onReset` above keep treating "nothing
  // customized" as the field being absent rather than an empty object.
  const setBufferOverride = (field, value) => {
    const next = { ...rbov, [field]: value };
    const allNull = Object.values(next).every((v) => v == null);
    onSet("replay_buffer_video", allNull ? null : next);
  };

  return (
    <div className="border-t border-stone-800/50 pt-2">
      <div className="mb-1.5 flex items-center justify-between gap-2">
        <span className="flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-wider text-stone-500">
          {t("settings.games.overridesTitle")}
          {setCount > 0 && (
            <span className="rounded bg-accent-500/15 px-1.5 py-px text-[9px] font-bold text-accent-300">{setCount}</span>
          )}
        </span>
        <div className="flex items-center gap-2">
          {setCount > 0 && (
            <button onClick={onReset}
              className="text-[10px] font-medium text-stone-500 transition hover:text-red-400">
              {t("settings.games.overrideReset")}
            </button>
          )}
          {/* recording-mode pills, same language as the Recording Mode card */}
          <div className="flex items-center rounded-lg bg-stone-950 p-0.5">
            {[[null, defaultLabel], ...["clips", "full_session", "off"].map((m) => [m, t(`settings.games.modeShort.${m}`)])].map(([m, label]) => (
              <button key={String(m)} onClick={() => onSet("game_detect_mode", m)}
                className={`rounded-md px-2 py-0.5 text-[10px] font-semibold transition ${
                  (ov.game_detect_mode ?? null) === m
                    ? m === null ? "bg-stone-700 text-stone-200" : "bg-accent-500/20 text-accent-300"
                    : "text-stone-600 hover:text-stone-400"
                }`}>
                {label}
              </button>
            ))}
          </div>
        </div>
      </div>

      <div className="grid grid-cols-4 gap-2">
        <OverrideTile label={t("settings.video.fps")} display={`${ov.fps} FPS`} overridden={ov.fps != null}
          defaultLabel={defaultLabel} value={ov.fps} onChange={(v) => onSet("fps", v === null ? null : Number(v))}>
          {def}
          {FPS_OPTIONS.map((f) => <option key={f} value={f}>{f} FPS</option>)}
        </OverrideTile>
        <OverrideTile label={t("settings.video.resolution")} display={RESOLUTION_LABELS[ov.resolution] ?? ""}
          overridden={ov.resolution != null} defaultLabel={defaultLabel} value={ov.resolution}
          onChange={(v) => onSet("resolution", v)}>
          {def}
          {RESOLUTION_OPTIONS.map(([v, label]) => <option key={v} value={v}>{label}</option>)}
        </OverrideTile>
        <OverrideTile label={t("settings.video.bitrate")} display={ov.bitrate_kbps != null ? `${(ov.bitrate_kbps / 1000).toFixed(0)} Mbps` : ""}
          overridden={ov.bitrate_kbps != null} defaultLabel={defaultLabel} value={ov.bitrate_kbps}
          onChange={(v) => onSet("bitrate_kbps", v === null ? null : Number(v))}>
          {def}
          {BITRATE_OPTIONS.map((b) => <option key={b} value={b}>{(b / 1000).toFixed(0)} Mbps</option>)}
        </OverrideTile>
        <OverrideTile label={t("settings.video.encoder")} display={ENCODER_LABELS[ov.encoder] ?? ""} overridden={ov.encoder != null}
          defaultLabel={defaultLabel} value={ov.encoder} onChange={(v) => onSet("encoder", v)}>
          {def}
          {ENCODER_GROUPS.map((group) => (
            <optgroup key={group.label} label={group.label}>
              {/* Grayed out (native `disabled`) rather than hidden when
                  this machine's hardware doesn't support them. */}
              {group.options.map((opt) => (
                <option key={opt} value={opt} disabled={encoderAvailability[opt] === false}>
                  {ENCODER_LABELS[opt]}{encoderAvailability[opt] === false ? ` (${t("settings.video.unavailable")})` : ""}
                </option>
              ))}
            </optgroup>
          ))}
        </OverrideTile>
        <OverrideTile label={t("settings.video.container")} display={ov.container ? CONTAINER_LABELS[ov.container] : ""} overridden={ov.container != null}
          defaultLabel={defaultLabel} value={ov.container} onChange={(v) => onSet("container", v)}>
          {def}
          {CONTAINER_OPTIONS.map((c) => <option key={c} value={c}>{CONTAINER_LABELS[c]}</option>)}
        </OverrideTile>
        <OverrideTile label={t("settings.video.audioCodec")} display={(AUDIO_CODECS.find(([v]) => v === ov.audio_codec) ?? [])[1] ?? ""}
          overridden={ov.audio_codec != null} defaultLabel={defaultLabel} value={ov.audio_codec}
          onChange={(v) => onSet("audio_codec", v)}>
          {def}
          {AUDIO_CODECS.map(([v, label]) => <option key={v} value={v}>{label}</option>)}
        </OverrideTile>
        <OverrideTile label={t("settings.games.overrideYoutube")}
          display={ov.youtube_live != null ? t(ov.youtube_live ? "settings.games.overrideOn" : "settings.games.overrideOff") : ""}
          overridden={ov.youtube_live != null} defaultLabel={defaultLabel}
          value={ov.youtube_live == null ? "" : String(ov.youtube_live)}
          onChange={(v) => onSet("youtube_live", v === null ? null : v === "true")}>
          {def}
          <option value="true">{t("settings.games.overrideOn")}</option>
          <option value="false">{t("settings.games.overrideOff")}</option>
        </OverrideTile>
        <OverrideTile label={t("settings.games.overrideFolder")}
          display={(folders.find((f) => f.id === ov.folder_id) ?? {}).name ?? ""}
          overridden={ov.folder_id != null} defaultLabel={defaultLabel}
          value={ov.folder_id ?? ""} onChange={(v) => onSet("folder_id", v)}>
          {def}
          {folders.map((f) => <option key={f.id} value={f.id}>{f.name}</option>)}
        </OverrideTile>
      </div>

      {/* This game's own replay-buffer-only quality — wins over both the
          tiles above and the global "custom video for the buffer" setting
          whenever the buffer is targeting this game specifically. */}
      <div className="mt-3 border-t border-stone-800/50 pt-2">
        <span className="mb-1.5 block text-[10px] font-semibold uppercase tracking-wider text-stone-500">
          {t("settings.games.replayBufferVideoTitle")}
        </span>
        <div className="grid grid-cols-4 gap-2">
          <OverrideTile label={t("settings.video.fps")} display={`${rbov.fps} FPS`} overridden={rbov.fps != null}
            defaultLabel={defaultLabel} value={rbov.fps} onChange={(v) => setBufferOverride("fps", v === null ? null : Number(v))}>
            {def}
            {FPS_OPTIONS.map((f) => <option key={f} value={f}>{f} FPS</option>)}
          </OverrideTile>
          <OverrideTile label={t("settings.video.resolution")} display={RESOLUTION_LABELS[rbov.resolution] ?? ""}
            overridden={rbov.resolution != null} defaultLabel={defaultLabel} value={rbov.resolution}
            onChange={(v) => setBufferOverride("resolution", v)}>
            {def}
            {RESOLUTION_OPTIONS.map(([v, label]) => <option key={v} value={v}>{label}</option>)}
          </OverrideTile>
          <OverrideTile label={t("settings.video.bitrate")} display={rbov.bitrate_kbps != null ? `${(rbov.bitrate_kbps / 1000).toFixed(0)} Mbps` : ""}
            overridden={rbov.bitrate_kbps != null} defaultLabel={defaultLabel} value={rbov.bitrate_kbps}
            onChange={(v) => setBufferOverride("bitrate_kbps", v === null ? null : Number(v))}>
            {def}
            {BITRATE_OPTIONS.map((b) => <option key={b} value={b}>{(b / 1000).toFixed(0)} Mbps</option>)}
          </OverrideTile>
          <OverrideTile label={t("settings.video.encoder")} display={ENCODER_LABELS[rbov.encoder] ?? ""} overridden={rbov.encoder != null}
            defaultLabel={defaultLabel} value={rbov.encoder} onChange={(v) => setBufferOverride("encoder", v)}>
            {def}
            {ENCODER_GROUPS.map((group) => (
              <optgroup key={group.label} label={group.label}>
                {group.options.map((opt) => (
                  <option key={opt} value={opt} disabled={encoderAvailability[opt] === false}>
                    {ENCODER_LABELS[opt]}{encoderAvailability[opt] === false ? ` (${t("settings.video.unavailable")})` : ""}
                  </option>
                ))}
              </optgroup>
            ))}
          </OverrideTile>
          <OverrideTile label={t("settings.video.container")} display={rbov.container ? CONTAINER_LABELS[rbov.container] : ""} overridden={rbov.container != null}
            defaultLabel={defaultLabel} value={rbov.container} onChange={(v) => setBufferOverride("container", v)}>
            {def}
            {CONTAINER_OPTIONS.map((c) => <option key={c} value={c}>{CONTAINER_LABELS[c]}</option>)}
          </OverrideTile>
          <OverrideTile label={t("settings.video.audioCodec")} display={(AUDIO_CODECS.find(([v]) => v === rbov.audio_codec) ?? [])[1] ?? ""}
            overridden={rbov.audio_codec != null} defaultLabel={defaultLabel} value={rbov.audio_codec}
            onChange={(v) => setBufferOverride("audio_codec", v)}>
            {def}
            {AUDIO_CODECS.map(([v, label]) => <option key={v} value={v}>{label}</option>)}
          </OverrideTile>
        </div>
      </div>
    </div>
  );
}

// Small green "playing now" pulse, shared by the row, tile, and modal.
function PlayingDot() {
  return (
    <span className="relative flex h-1.5 w-1.5 shrink-0">
      <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-emerald-400 opacity-60" />
      <span className="relative inline-flex h-1.5 w-1.5 rounded-full bg-emerald-400" />
    </span>
  );
}

// List-mode row: just identity + the enable toggle. Everything else
// (overrides, exe list, removal) lives in the detail modal `onOpen` opens.
function GameRow({ game, playing, lang, t, onOpen, onToggle }) {
  const icon = useGameIcon(game.exes[0].exe, game.has_icon_url, game.name, game.custom);
  const cover = useGameCover(game.exes[0].exe, game.has_cover_url);
  return (
    <div className="flex cursor-pointer items-center gap-3 border-b border-stone-800/50 px-3 transition hover:bg-stone-900/60"
      style={{ height: ROW_H }} onClick={onOpen}>
      {/* Covers are portrait (box-art aspect), so they take the icon's
          slot as a small poster thumbnail; square icon and generic glyph
          are the fallbacks. */}
      {cover
        ? <img src={cover} alt="" className={`h-12 w-9 shrink-0 rounded-md object-cover ${game.enabled ? "" : "opacity-40 grayscale"}`} />
        : icon
          ? <img src={icon} alt="" className="h-9 w-9 shrink-0 rounded-lg object-cover" />
          : <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-stone-800 text-stone-600"><MdSportsEsports size={18} /></span>}
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className={`truncate text-sm font-medium ${game.enabled ? "text-stone-100" : "text-stone-500"}`}>{game.name}</span>
          {game.custom && (
            <span className="shrink-0 rounded bg-accent-500/15 px-1.5 py-0.5 text-[10px] font-semibold text-accent-300">
              {t("settings.games.customBadge")}
            </span>
          )}
        </div>
        {playing ? (
          <div className="flex items-center gap-1.5 text-[11px] font-medium text-emerald-400">
            <PlayingDot /> {t("settings.games.playingNow")}
          </div>
        ) : game.last_played && (
          <div className="text-[11px] text-accent-400/70">
            {t("settings.games.lastPlayed")(relativeTime(game.last_played * 1000, lang))}
          </div>
        )}
      </div>
      <span onClick={(e) => e.stopPropagation()}>
        <Toggle labeled checked={game.enabled} onChange={onToggle} />
      </span>
      <MdChevronRight size={16} className="shrink-0 text-stone-600" />
    </div>
  );
}

// Grid-mode tile: poster (or icon on a dark slab) + name + toggle. Same
// click-to-open-modal behavior as the list row.
function GameTile({ game, playing, t, onOpen, onToggle }) {
  const icon = useGameIcon(game.exes[0].exe, game.has_icon_url, game.name, game.custom);
  const cover = useGameCover(game.exes[0].exe, game.has_cover_url);
  return (
    <div className="group cursor-pointer" onClick={onOpen}>
      <div className={`relative h-[150px] w-full overflow-hidden rounded-lg border border-stone-800 bg-stone-900 transition group-hover:border-stone-600 ${game.enabled ? "" : "opacity-50"}`}>
        {cover
          ? <img src={cover} alt="" className={`h-full w-full object-cover ${game.enabled ? "" : "grayscale"}`} />
          : icon
            ? <div className="flex h-full w-full items-center justify-center"><img src={icon} alt="" className="h-12 w-12 rounded-xl object-cover" /></div>
            : <div className="flex h-full w-full items-center justify-center text-stone-700"><MdSportsEsports size={28} /></div>}
        {playing && (
          <span className="absolute left-1.5 top-1.5 flex items-center gap-1 rounded bg-black/70 px-1.5 py-0.5 text-[9px] font-semibold text-emerald-400">
            <PlayingDot /> {t("settings.games.playingNow")}
          </span>
        )}
        {game.custom && (
          <span className="absolute right-1.5 top-1.5 rounded bg-accent-500/80 px-1.5 py-0.5 text-[9px] font-bold text-stone-950">
            {t("settings.games.customBadge")}
          </span>
        )}
      </div>
      <div className="mt-1.5 flex items-center justify-between gap-1.5 px-0.5">
        <span className={`truncate text-[11.5px] font-medium ${game.enabled ? "text-stone-200" : "text-stone-500"}`} title={game.name}>
          {game.name}
        </span>
        <span onClick={(e) => e.stopPropagation()} className="shrink-0 scale-[0.8]">
          <Toggle checked={game.enabled} onChange={onToggle} />
        </span>
      </div>
    </div>
  );
}

// Everything the old inline expansion held (overrides, exe list, removal),
// as a centered modal — same overlay/panel pattern as TagManageModal.
function GameDetailModal({ game, playing, lang, t, onClose, onToggle, onToggleExe, onRemove, onRemoveExe, onAddExe, addingExe, onSetOverride, onResetOverrides, folders, encoderAvailability }) {
  const icon = useGameIcon(game.exes[0].exe, game.has_icon_url, game.name, game.custom);
  const cover = useGameCover(game.exes[0].exe, game.has_cover_url);
  // The per-game default-folder override can point at any global folder or
  // one of this game's own, never another game's.
  const availableFolders = folders.filter((f) => !f.game || f.game === game.name);
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-6" onClick={onClose}>
      <div className="flex max-h-[85vh] w-full max-w-xl flex-col rounded-xl border border-stone-800 bg-stone-900" onClick={(e) => e.stopPropagation()}>
        {/* Header: art + identity + master toggle */}
        <div className="flex items-center gap-3.5 border-b border-stone-800 p-4">
          {cover
            ? <img src={cover} alt="" className="h-20 w-[60px] shrink-0 rounded-lg object-cover" />
            : icon
              ? <img src={icon} alt="" className="h-14 w-14 shrink-0 rounded-xl object-cover" />
              : <span className="flex h-14 w-14 shrink-0 items-center justify-center rounded-xl bg-stone-800 text-stone-600"><MdSportsEsports size={26} /></span>}
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <span className="truncate text-base font-semibold text-stone-100">{game.name}</span>
              {game.custom && (
                <span className="shrink-0 rounded bg-accent-500/15 px-1.5 py-0.5 text-[10px] font-semibold text-accent-300">
                  {t("settings.games.customBadge")}
                </span>
              )}
            </div>
            {playing ? (
              <div className="mt-0.5 flex items-center gap-1.5 text-[11px] font-medium text-emerald-400">
                <PlayingDot /> {t("settings.games.playingNow")}
              </div>
            ) : game.last_played ? (
              <div className="mt-0.5 text-[11px] text-accent-400/70">
                {t("settings.games.lastPlayed")(relativeTime(game.last_played * 1000, lang))}
              </div>
            ) : null}
          </div>
          <Toggle labeled checked={game.enabled} onChange={onToggle} />
          <button onClick={onClose} className="rounded p-1 text-stone-500 transition hover:bg-stone-800 hover:text-stone-200">
            <MdClose size={16} />
          </button>
        </div>

        <div className="min-h-0 overflow-y-auto p-4">
          <OverridesPanel game={game} t={t} onSet={onSetOverride} onReset={onResetOverrides} folders={availableFolders}
            encoderAvailability={encoderAvailability} />

          <div className="mt-3 border-t border-stone-800/50 pt-2">
            <div className="mb-1 text-[10px] font-semibold uppercase tracking-wider text-stone-500">
              {t("settings.games.exeLabel")}
            </div>
            {game.exes.map((e) => (
              <div key={e.exe} className="flex items-center justify-between gap-3" style={{ height: SUB_H }}>
                <span className={`truncate font-mono text-[11px] ${e.enabled ? "text-stone-400" : "text-stone-600"}`}>{e.exe}.exe</span>
                <div className="flex items-center gap-2">
                  <Toggle checked={e.enabled} onChange={() => onToggleExe(e.exe)} />
                  {game.custom && game.exes.length > 1 && (
                    <button onClick={() => onRemoveExe(e.exe)} title={t("common.delete")}
                      className="shrink-0 rounded p-0.5 text-stone-600 transition hover:bg-stone-800 hover:text-red-400">
                      <MdClose size={13} />
                    </button>
                  )}
                </div>
              </div>
            ))}
            {game.custom && (
              <button onClick={onAddExe} disabled={addingExe}
                style={{ height: SUB_H }}
                className="flex w-full items-center gap-1.5 text-[11px] font-medium text-accent-400 transition hover:text-accent-300 disabled:opacity-60">
                <MdAdd size={13} /> {addingExe ? t("common.loading") : t("settings.games.addExe")}
              </button>
            )}
          </div>

          {game.custom && (
            <div className="mt-3 border-t border-stone-800/50 pt-3">
              <button onClick={onRemove}
                className="flex items-center gap-1.5 text-[11px] font-medium text-stone-500 transition hover:text-red-400">
                <MdClose size={13} /> {t("common.delete")}
              </button>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

const rowKey = (g) => `${g.custom ? "c:" : ""}${g.name}:${g.exes[0].exe}`;

export function GamesCard({ t, lang, folders = [] }) {
  const [games, setGames] = useState(null); // null = loading
  const [search, setSearch] = useState("");
  const [detailKey, setDetailKey] = useState(null); // rowKey of the modal's game
  const [view, setView] = useState(() => localStorage.getItem("capcove.gamesView") || "list");
  const [cols, setCols] = useState(4); // grid-mode column count, from container width
  const [currentGame, setCurrentGame] = useState(null); // display name from detection
  const [encoders, setEncoders] = useState(null); // null = loading, [] = fetch failed

  const setViewMode = (v) => { setView(v); localStorage.setItem("capcove.gamesView", v); };

  // "Playing now" badge: poll the detection loop's current game.
  useEffect(() => {
    const load = () => invoke("get_current_game").then(setCurrentGame).catch(() => {});
    load();
    const id = setInterval(load, 3000);
    return () => clearInterval(id);
  }, []);

  // Same source RecordSettingsCard's encoder picker uses — lets the
  // per-game encoder override gray out options this machine can't encode with.
  useEffect(() => {
    invoke("list_available_encoders").then(setEncoders).catch(() => setEncoders([]));
  }, []);
  const encoderAvailability = Object.fromEntries((encoders ?? []).map((e) => [e.kind, e.available]));
  const [syncing, setSyncing] = useState(false);
  const [syncProgress, setSyncProgress] = useState(null); // {done,total}
  const [adding, setAdding] = useState(false);
  const [newExe, setNewExe] = useState("");
  const [newName, setNewName] = useState("");
  const [newIcon, setNewIcon] = useState(null); // data:image/png;base64,... from the picked exe
  const [browsing, setBrowsing] = useState(false);
  const scrollRef = useRef(null);

  const load = () => invoke("list_games").then(setGames).catch(() => setGames([]));
  useEffect(() => { load(); }, []);

  useEffect(() => {
    let unlisten;
    (async () => { unlisten = await listen("games-sync-progress", (e) => setSyncProgress(e.payload)); })();
    return () => unlisten?.();
  }, []);

  const sync = async () => {
    setSyncing(true);
    setSyncProgress(null);
    try { await invoke("sync_games"); } catch {}
    setSyncing(false);
    setSyncProgress(null);
    load();
  };

  // Lets the user pick the actual .exe instead of typing its name — fills
  // in the stem, a suggested display name, and grabs the icon from the file.
  const browseExe = async () => {
    setBrowsing(true);
    try {
      const path = await invoke("pick_exe_file").catch(() => null);
      if (!path) return;
      const info = await invoke("inspect_exe_file", { path }).catch(() => null);
      if (!info) return;
      setNewExe(info.exe_stem);
      setNewName((cur) => cur.trim() ? cur : info.suggested_name);
      setNewIcon(info.icon ?? null);
    } finally {
      setBrowsing(false);
    }
  };

  const addGame = async () => {
    const exe = newExe.trim().replace(/\.exe$/i, "");
    if (!exe) return;
    await invoke("add_custom_game", { exe, name: newName.trim() || exe, icon: newIcon }).catch(() => {});
    setNewExe(""); setNewName(""); setNewIcon(null); setAdding(false);
    load();
  };

  const toggle = async (game) => {
    // Optimistic flip — 7k rows re-fetch would be wasteful per click.
    const enabled = !game.enabled;
    setGames((gs) => gs.map((g) =>
      g === game ? { ...g, enabled, exes: g.exes.map((e) => ({ ...e, enabled })) } : g
    ));
    await invoke("set_game_enabled", { exes: game.exes.map((e) => e.exe), enabled }).catch(() => {});
  };

  const toggleExe = async (game, exe) => {
    const enabled = !game.exes.find((e) => e.exe === exe)?.enabled;
    setGames((gs) => gs.map((g) => {
      if (g !== game) return g;
      const exes = g.exes.map((e) => (e.exe === exe ? { ...e, enabled } : e));
      return { ...g, exes, enabled: exes.some((e) => e.enabled) };
    }));
    await invoke("set_game_enabled", { exes: [exe], enabled }).catch(() => {});
  };

  // Deletes the whole custom game (every exe registered under its name) —
  // NOT just the first exe, now that one custom game can have several.
  const removeCustom = async (game) => {
    await invoke("remove_custom_game_group", { name: game.name }).catch(() => {});
    load();
  };

  const removeExe = async (game, exe) => {
    await invoke("remove_custom_game", { exe }).catch(() => {});
    load();
  };

  // Adds another exe to an *existing* custom game, reusing its name so
  // `list_games` groups it into the same row (see games_db.rs).
  const [addingExeFor, setAddingExeFor] = useState(null); // game.name currently browsing for
  const addExeToGame = async (game) => {
    setAddingExeFor(game.name);
    try {
      const path = await invoke("pick_exe_file").catch(() => null);
      if (!path) return;
      const info = await invoke("inspect_exe_file", { path }).catch(() => null);
      if (!info) return;
      await invoke("add_custom_game", { exe: info.exe_stem, name: game.name, icon: info.icon }).catch(() => {});
      load();
    } finally {
      setAddingExeFor(null);
    }
  };

  const setOverride = async (game, field, value) => {
    const next = { ...(game.overrides ?? {}), [field]: value };
    const isEmpty = Object.values(next).every((v) => v == null);
    setGames((gs) => gs.map((g) => (g === game ? { ...g, overrides: isEmpty ? null : next } : g)));
    await invoke("set_game_overrides", { name: game.name, overrides: next }).catch(() => {});
  };

  const resetOverrides = async (game) => {
    setGames((gs) => gs.map((g) => (g === game ? { ...g, overrides: null } : g)));
    await invoke("set_game_overrides", { name: game.name, overrides: {} }).catch(() => {});
  };

  const needle = search.trim().toLowerCase();
  const filtered = useMemo(() => {
    if (!games) return [];
    let list = needle
      ? games.filter((g) => g.name.toLowerCase().includes(needle) || g.exes.some((e) => e.exe.toLowerCase().includes(needle)))
      : games;
    // The game being played right now pins to the very top.
    if (currentGame) {
      const idx = list.findIndex((g) => g.name.toLowerCase() === currentGame.toLowerCase());
      if (idx > 0) list = [list[idx], ...list.slice(0, idx), ...list.slice(idx + 1)];
    }
    return list;
  }, [games, needle, currentGame]);

  // "My Games": manually-added games and anything actually seen running.
  // Currently-playing first, then most-recently-played, custom last.
  const myGames = useMemo(() => {
    if (!games) return [];
    return games
      .filter((g) => g.custom || g.last_played)
      .sort((a, b) => {
        const aPlaying = currentGame && a.name.toLowerCase() === currentGame.toLowerCase();
        const bPlaying = currentGame && b.name.toLowerCase() === currentGame.toLowerCase();
        if (aPlaying !== bPlaying) return aPlaying ? -1 : 1;
        return (b.last_played ?? 0) - (a.last_played ?? 0);
      });
  }, [games, currentGame]);

  // Grid mode virtualizes *chunks* of `cols` tiles as one fixed-height row;
  // the column count tracks the scroll container's real width.
  useEffect(() => {
    const el = scrollRef.current;
    if (!el || view !== "grid") return;
    const compute = () => setCols(Math.max(3, Math.floor(el.clientWidth / 132)));
    compute();
    const ro = new ResizeObserver(compute);
    ro.observe(el);
    return () => ro.disconnect();
  }, [view, games]);

  const virtualizer = useVirtualizer({
    count: view === "grid" ? Math.ceil(filtered.length / cols) : filtered.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => (view === "grid" ? GRID_ROW_H : ROW_H),
    overscan: 6,
  });
  // Heights/indices shift when the view mode, column count, or filter
  // changes — force a remeasure.
  useEffect(() => { virtualizer.measure(); }, [view, cols, filtered]);

  // The modal's game, resolved fresh from the list every render so toggles
  // flip live inside it.
  const detailGame = detailKey && games ? games.find((g) => rowKey(g) === detailKey) : null;
  const rowHandlers = (game) => ({
    playing: !!currentGame && game.name.toLowerCase() === currentGame.toLowerCase(),
    onOpen: () => setDetailKey(rowKey(game)),
    onToggle: () => toggle(game),
  });

  return (
    <Card
      title={t("settings.games.title")}
      right={
        <div className="flex items-center gap-2">
          <button onClick={() => setAdding((v) => !v)}
            className="flex items-center gap-1 text-xs font-medium text-accent-400 transition hover:text-accent-300">
            <MdAdd size={14} /> {t("settings.games.addManual")}
          </button>
          <button onClick={sync} disabled={syncing}
            className="flex items-center gap-1.5 rounded bg-stone-800 px-2 py-1 text-xs text-stone-300 transition hover:bg-stone-700 disabled:opacity-60">
            <MdSync size={13} className={syncing ? "animate-spin" : ""} />
            {syncing
              ? (syncProgress ? `${syncProgress.done}/${syncProgress.total}` : t("settings.games.syncing"))
              : t("settings.games.sync")}
          </button>
        </div>
      }
    >
      {adding && (
        <div className="flex flex-col gap-2 py-3">
          <div className="flex items-end gap-2">
            <div className="flex min-w-0 flex-1 flex-col gap-1">
              <span className="text-[11px] font-medium text-stone-400">{t("settings.games.exeLabel")}</span>
              <input value={newExe} onChange={(e) => setNewExe(e.target.value)}
                placeholder="gta5" className={`${inputCls} w-full`} />
            </div>
            <Button onClick={browseExe} disabled={browsing}>
              {browsing ? t("common.loading") : t("settings.games.browseExe")}
            </Button>
          </div>
          <div className="flex items-end gap-2">
            <div className="flex h-9 w-9 shrink-0 items-center justify-center overflow-hidden rounded-md bg-stone-800">
              {newIcon
                ? <img src={newIcon} alt="" className="h-full w-full object-cover" />
                : <MdSportsEsports size={16} className="text-stone-600" />}
            </div>
            <div className="flex min-w-0 flex-1 flex-col gap-1">
              <span className="text-[11px] font-medium text-stone-400">{t("settings.games.nameLabel")}</span>
              <input value={newName} onChange={(e) => setNewName(e.target.value)}
                onKeyDown={(e) => { if (e.key === "Enter") addGame(); }}
                placeholder="Grand Theft Auto V" className={`${inputCls} w-full`} />
            </div>
            <Button variant="primary" onClick={addGame}>{t("settings.games.add")}</Button>
          </div>
        </div>
      )}

      {myGames.length > 0 && (
        <div className="border-b border-stone-800/50 pb-3 pt-1">
          <div className="mb-1.5 px-1 text-[11px] font-semibold uppercase tracking-wider text-stone-500">
            {t("settings.games.myGames")}
          </div>
          <div className="max-h-[300px] overflow-y-auto rounded-lg border border-stone-800 bg-stone-950/50">
            {myGames.map((game) => (
              <GameRow key={rowKey(game)} game={game} lang={lang} t={t} {...rowHandlers(game)} />
            ))}
          </div>
        </div>
      )}

      <div className="py-3">
        <div className="mb-2 px-1 text-[11px] font-semibold uppercase tracking-wider text-stone-500">
          {t("settings.games.allGames")}
        </div>
        <div className="mb-2 flex items-center gap-2">
          <div className="relative min-w-0 flex-1">
            <MdSearch size={14} className="absolute left-2.5 top-1/2 -translate-y-1/2 text-stone-600" />
            <input value={search} onChange={(e) => setSearch(e.target.value)}
              placeholder={t("settings.games.searchPlaceholder")}
              className="w-full rounded-lg border border-stone-800 bg-stone-950 py-1.5 pl-8 pr-2.5 text-xs text-stone-200 outline-none transition focus:border-accent-500 placeholder:text-stone-600" />
          </div>
          <div className="flex shrink-0 items-center rounded-lg bg-stone-950 p-0.5">
            {[["list", MdViewList], ["grid", MdGridView]].map(([mode, Icn]) => (
              <button key={mode} type="button" onClick={() => setViewMode(mode)}
                title={t(`settings.games.view.${mode}`)}
                className={`rounded-md p-1.5 transition ${
                  view === mode ? "bg-stone-700 text-stone-100" : "text-stone-600 hover:text-stone-300"
                }`}>
                <Icn size={14} />
              </button>
            ))}
          </div>
        </div>

        {games === null ? (
          // Same fixed height + frame as the loaded list below, so the
          // card doesn't jump in size once games finish loading.
          <div className="flex h-[420px] flex-col gap-2 overflow-hidden rounded-lg border border-stone-800 bg-stone-950/50 p-2">
            {Array.from({ length: 6 }).map((_, i) => (
              <div key={i} className="flex items-center gap-3 px-1" style={{ height: ROW_H - 8 }}>
                <div className="h-9 w-9 shrink-0 animate-pulse rounded-lg bg-stone-800" />
                <div className="flex min-w-0 flex-1 flex-col gap-1.5">
                  <div className="h-3 w-1/3 animate-pulse rounded bg-stone-800" />
                  <div className="h-2.5 w-1/5 animate-pulse rounded bg-stone-800/70" />
                </div>
              </div>
            ))}
          </div>
        ) : filtered.length === 0 ? (
          <div className="py-8 text-center text-xs text-stone-600">
            {games.length === 0 ? t("settings.games.emptyHint") : t("gallery.noMatch")}
          </div>
        ) : (
          <div ref={scrollRef} className="h-[420px] overflow-y-auto rounded-lg border border-stone-800 bg-stone-950/50">
            <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
              {virtualizer.getVirtualItems().map((row) => {
                if (view === "grid") {
                  // One virtual row = one chunk of `cols` tiles.
                  const slice = filtered.slice(row.index * cols, row.index * cols + cols);
                  return (
                    <div key={row.index}
                      style={{
                        position: "absolute", top: 0, left: 0, right: 0, height: row.size,
                        transform: `translateY(${row.start}px)`,
                        display: "grid", gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))`,
                        gap: "10px", padding: "5px 10px",
                      }}>
                      {slice.map((game) => (
                        <GameTile key={rowKey(game)} game={game} t={t} {...rowHandlers(game)} />
                      ))}
                    </div>
                  );
                }
                const game = filtered[row.index];
                return (
                  <div key={rowKey(game)}
                    style={{ position: "absolute", top: 0, left: 0, right: 0, height: row.size, transform: `translateY(${row.start}px)` }}>
                    <GameRow game={game} lang={lang} t={t} {...rowHandlers(game)} />
                  </div>
                );
              })}
            </div>
          </div>
        )}
        <div className="mt-2 text-[11px] text-stone-600">
          {games !== null && t("settings.games.countHint")(filtered.length)}
        </div>
      </div>

      {detailGame && (
        <GameDetailModal game={detailGame} lang={lang} t={t}
          playing={!!currentGame && detailGame.name.toLowerCase() === currentGame.toLowerCase()}
          onClose={() => setDetailKey(null)}
          onToggle={() => toggle(detailGame)}
          onToggleExe={(exe) => toggleExe(detailGame, exe)}
          onRemove={() => { setDetailKey(null); removeCustom(detailGame); }}
          onRemoveExe={(exe) => removeExe(detailGame, exe)}
          onAddExe={() => addExeToGame(detailGame)}
          addingExe={addingExeFor === detailGame.name}
          onSetOverride={(field, value) => setOverride(detailGame, field, value)}
          onResetOverrides={() => resetOverrides(detailGame)}
          folders={folders}
          encoderAvailability={encoderAvailability} />
      )}
    </Card>
  );
}
