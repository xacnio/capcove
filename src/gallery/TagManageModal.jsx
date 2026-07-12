import { useRef, useState } from "react";
import { inputCls } from "../components/settingsUI.jsx";
import * as Icon from "./icons.jsx";

const COLORS = ["#ef4444", "#f97316", "#eab308", "#22c55e", "#06b6d4", "#3b82f6", "#8b5cf6", "#ec4899"];

function uuid() {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}`;
}

function ColorSwatches({ value, onChange }) {
  return (
    <div className="flex flex-wrap gap-1.5">
      {COLORS.map((c) => (
        <button key={c} type="button" onClick={() => onChange(c)}
          className={`h-5 w-5 shrink-0 rounded-full transition ${
            value === c ? "ring-2 ring-offset-2 ring-offset-stone-900 ring-stone-300" : "hover:scale-110"
          }`}
          style={{ backgroundColor: c }} />
      ))}
    </div>
  );
}

// Every change here saves for real, right away — there's no draft to discard,
// so the modal only ever needs a way to close, never a Cancel that would
// imply otherwise. Renames debounce like every other text field in Settings;
// everything else (add/remove/recolor) is a single discrete action, saved
// the instant it happens.
export default function TagManageModal({ t, tags, onSave, onClose }) {
  const [list, setList] = useState(tags);
  const [draftName, setDraftName] = useState("");
  const [draftColor, setDraftColor] = useState(COLORS[0]);
  const [editingColorId, setEditingColorId] = useState(null);
  const saveTimer = useRef(null);

  const saveNow = (next) => {
    clearTimeout(saveTimer.current);
    setList(next);
    onSave(next);
  };
  const saveDebounced = (next) => {
    setList(next);
    clearTimeout(saveTimer.current);
    saveTimer.current = setTimeout(() => onSave(next), 400);
  };

  const addTag = () => {
    const name = draftName.trim();
    if (!name) return;
    saveNow([...list, { id: uuid(), name, color: draftColor }]);
    setDraftName("");
  };

  const removeTag = (id) => saveNow(list.filter((tag) => tag.id !== id));
  const renameTag = (id, name) => saveDebounced(list.map((tag) => (tag.id === id ? { ...tag, name } : tag)));
  const recolorTag = (id, color) => saveNow(list.map((tag) => (tag.id === id ? { ...tag, color } : tag)));

  const close = () => {
    // A rename's debounce timer may still be pending — flush it so the last
    // keystrokes aren't lost on close.
    if (saveTimer.current) { clearTimeout(saveTimer.current); onSave(list); }
    onClose();
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-6" onClick={close}>
      <div className="w-full max-w-sm rounded-xl border border-stone-800 bg-stone-900 p-5" onClick={(e) => e.stopPropagation()}>
        <div className="mb-4 flex items-center justify-between">
          <h3 className="text-sm font-semibold text-stone-100">{t("gallery.tags.manage")}</h3>
          <button onClick={close} className="rounded p-1 text-stone-500 transition hover:bg-stone-800 hover:text-stone-200">
            <Icon.X size={16} />
          </button>
        </div>

        <div className="mb-4 flex max-h-64 flex-col gap-0.5 overflow-y-auto">
          {list.length === 0 && (
            <div className="rounded-lg px-2 py-3 text-center text-xs text-stone-600">{t("gallery.tags.noTags")}</div>
          )}
          {list.map((tag) => (
            <div key={tag.id} className="rounded-lg transition hover:bg-stone-800/60">
              <div className="flex items-center gap-2.5 px-2 py-2">
                <button type="button" onClick={() => setEditingColorId((id) => (id === tag.id ? null : tag.id))}
                  title={t("gallery.tags.tagColor")}
                  className="h-3.5 w-3.5 shrink-0 rounded-full ring-1 ring-inset ring-white/10 transition hover:scale-110"
                  style={{ backgroundColor: tag.color }} />
                <input
                  value={tag.name}
                  onChange={(e) => renameTag(tag.id, e.target.value)}
                  className="min-w-0 flex-1 bg-transparent text-sm text-stone-200 outline-none"
                />
                <button onClick={() => removeTag(tag.id)}
                  className="shrink-0 rounded p-1 text-stone-600 transition hover:bg-stone-800 hover:text-red-400">
                  <Icon.Trash size={13} />
                </button>
              </div>
              {editingColorId === tag.id && (
                <div className="px-2 pb-2.5 pl-8">
                  <ColorSwatches value={tag.color} onChange={(c) => recolorTag(tag.id, c)} />
                </div>
              )}
            </div>
          ))}
        </div>

        <div className="rounded-lg border border-stone-800 bg-stone-950/40 p-3">
          <div className="mb-2.5 flex items-center gap-2">
            <input
              autoFocus
              value={draftName}
              onChange={(e) => setDraftName(e.target.value)}
              onKeyDown={(e) => { if (e.key === "Enter") addTag(); }}
              placeholder={t("gallery.tags.tagName")}
              className={`${inputCls} min-w-0 flex-1`}
            />
            <button onClick={addTag} disabled={!draftName.trim()}
              title={t("gallery.tags.createTag")}
              className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-accent-500/15 text-accent-300 transition hover:bg-accent-500/25 disabled:opacity-40">
              <Icon.Tag size={14} />
            </button>
          </div>
          <ColorSwatches value={draftColor} onChange={setDraftColor} />
        </div>
      </div>
    </div>
  );
}
