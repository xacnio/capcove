import { useState } from "react";
import { invoke } from "../lib/tauri.js";
import { Toggle, Button, inputCls } from "../components/settingsUI.jsx";
import * as Icon from "./icons.jsx";

const AUTO_DELETE_DAY_OPTIONS = [1, 3, 7, 14, 30, 60, 90];

// Folder create/edit modal, opened from a tile in the gallery grid rather
// than a separate Settings page. Scope (`game`) is implicit from where the
// "+ New folder" tile was clicked; edit mode reuses the folder's own scope.
export default function FolderEditModal({ mode, game, folder, t, onClose, onSaved }) {
  const isCreate = mode === "create";
  const [name, setName] = useState(folder?.name ?? "");
  const [autoDeleteDays, setAutoDeleteDays] = useState(folder?.auto_delete_days ?? null);
  const [neverUpload, setNeverUpload] = useState(!!folder?.never_upload_to_drive);
  const [alwaysKeep, setAlwaysKeep] = useState(!!folder?.always_keep);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");

  const scopeGame = isCreate ? game : folder?.game;

  const save = async () => {
    const trimmed = name.trim();
    if (!trimmed) return;
    setSaving(true);
    setError("");
    try {
      if (isCreate) {
        await invoke("create_recording_folder", { name: trimmed, game: scopeGame ?? null });
      } else {
        if (trimmed !== folder.name) {
          await invoke("rename_recording_folder", { id: folder.id, newName: trimmed });
        }
        await invoke("update_recording_folder_rules", {
          id: folder.id,
          autoDeleteDays,
          neverUploadToDrive: neverUpload,
          alwaysKeep,
        });
      }
      onSaved?.();
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const remove = async () => {
    setSaving(true);
    try {
      await invoke("delete_recording_folder", { id: folder.id });
      onSaved?.();
      onClose();
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-6" onClick={onClose}>
      <div className="w-full max-w-sm rounded-xl border border-stone-800 bg-stone-900 p-5" onClick={(e) => e.stopPropagation()}>
        <div className="mb-1 flex items-center justify-between">
          <h3 className="text-sm font-semibold text-stone-100">
            {isCreate ? t("gallery.folderModal.createTitle") : t("gallery.folderModal.editTitle")}
          </h3>
          <button onClick={onClose} className="rounded p-1 text-stone-500 transition hover:bg-stone-800 hover:text-stone-200">
            <Icon.X size={16} />
          </button>
        </div>

        <div className="mb-3 flex items-center gap-1.5 text-xs text-stone-500">
          {scopeGame ? (<><Icon.Monitor size={12} /> {t("gallery.folderModal.scopedTo")(scopeGame)}</>) : (
            <><Icon.Folder size={12} /> {t("gallery.folderModal.global")}</>
          )}
        </div>

        <input
          autoFocus value={name} onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && save()}
          placeholder={t("settings.folders.newNamePlaceholder")}
          className={`${inputCls} mb-3 w-full`}
        />

        {!isCreate && (
          <div className="mb-3 flex flex-col gap-2.5 border-t border-stone-800 pt-3">
            <div className="flex items-center justify-between gap-3">
              <div className="min-w-0">
                <div className="text-xs font-medium text-stone-300">{t("settings.folders.autoDeleteLabel")}</div>
                <div className="text-[11px] text-stone-500">{t("settings.folders.autoDeleteHint")}</div>
              </div>
              <select value={autoDeleteDays ?? ""} disabled={alwaysKeep}
                onChange={(e) => setAutoDeleteDays(e.target.value === "" ? null : Number(e.target.value))}
                className={`${inputCls} shrink-0 cursor-pointer py-1 text-xs disabled:opacity-40`}>
                <option value="">{t("settings.folders.autoDeleteNever")}</option>
                {AUTO_DELETE_DAY_OPTIONS.map((d) => <option key={d} value={d}>{t("settings.folders.autoDeleteDays")(d)}</option>)}
              </select>
            </div>
            <div className="flex items-center justify-between gap-3">
              <div className="min-w-0">
                <div className="text-xs font-medium text-stone-300">{t("settings.folders.neverUploadLabel")}</div>
                <div className="text-[11px] text-stone-500">{t("settings.folders.neverUploadHint")}</div>
              </div>
              <Toggle checked={neverUpload} onChange={setNeverUpload} />
            </div>
            <div className="flex items-center justify-between gap-3">
              <div className="min-w-0">
                <div className="text-xs font-medium text-stone-300">{t("settings.folders.alwaysKeepLabel")}</div>
                <div className="text-[11px] text-stone-500">{t("settings.folders.alwaysKeepHint")}</div>
              </div>
              <Toggle checked={alwaysKeep} onChange={setAlwaysKeep} />
            </div>
          </div>
        )}

        {error && <div className="mb-2 text-xs text-red-400">{error}</div>}

        <div className="flex items-center justify-between gap-2 pt-1">
          {!isCreate ? (
            <button onClick={remove} disabled={saving}
              className="text-xs font-medium text-stone-500 transition hover:text-red-400 disabled:opacity-50">
              {t("common.delete")}
            </button>
          ) : <span />}
          <div className="flex gap-2">
            <button onClick={onClose} className="rounded-lg px-3.5 py-1.5 text-[13px] font-medium text-stone-300 transition hover:bg-stone-800">
              {t("gallery.confirm.cancel")}
            </button>
            <Button variant="primary" disabled={saving || !name.trim()} onClick={save}>
              {isCreate ? t("settings.folders.createButton") : t("common.save")}
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
}
