import { useEffect, useState } from "react";
import { invoke } from "../lib/tauri.js";
import { OverridesPanel } from "../components/GamesCard.jsx";
import { useAppIcon } from "./appIcons.js";
import * as Icon from "./icons.jsx";

// Lightweight per-game settings modal reached by right-clicking a game tile
// in the folder browser — same overrides editor as Settings > Games'
// detail modal, but scoped to just the overrides (rename/exe/removal stay
// in Settings, since this is a quick-access shortcut, not a replacement).
export default function GameSettingsModal({ t, name, folders, onClose }) {
  const [overrides, setOverrides] = useState(null); // null = loading
  const [encoderAvailability, setEncoderAvailability] = useState({});
  const icon = useAppIcon(name);

  useEffect(() => {
    invoke("get_game_overrides", { name }).then(setOverrides).catch(() => setOverrides({}));
  }, [name]);

  useEffect(() => {
    invoke("list_available_encoders")
      .then((list) => setEncoderAvailability(Object.fromEntries(list.map((e) => [e.kind, e.available]))))
      .catch(() => {});
  }, []);

  const setOverride = (field, value) => {
    setOverrides((cur) => {
      const next = { ...(cur ?? {}), [field]: value };
      invoke("set_game_overrides", { name, overrides: next }).catch(() => {});
      return next;
    });
  };

  const resetOverrides = () => {
    setOverrides({});
    invoke("set_game_overrides", { name, overrides: {} }).catch(() => {});
  };

  // Same rule as the Settings > Games detail modal: the per-game default
  // folder can point at any global folder or one of this game's own.
  const availableFolders = folders.filter((f) => !f.game || f.game === name);

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-6" onClick={onClose}>
      <div className="w-full max-w-md rounded-xl border border-stone-800 bg-stone-900 p-4" onClick={(e) => e.stopPropagation()}>
        <div className="mb-3 flex items-center gap-2.5">
          {icon
            ? <img src={icon} alt="" className="h-8 w-8 shrink-0 rounded-lg object-cover" />
            : <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-stone-800 text-stone-600"><Icon.Monitor size={16} /></span>}
          <span className="min-w-0 flex-1 truncate text-sm font-semibold text-stone-100">{name}</span>
          <button onClick={onClose} className="rounded p-1 text-stone-500 transition hover:bg-stone-800 hover:text-stone-200">
            <Icon.X size={16} />
          </button>
        </div>
        {overrides === null ? (
          <div className="py-6 text-center text-xs text-stone-600">{t("common.loading")}</div>
        ) : (
          <OverridesPanel game={{ overrides }} t={t} onSet={setOverride} onReset={resetOverrides}
            folders={availableFolders} encoderAvailability={encoderAvailability} />
        )}
      </div>
    </div>
  );
}
