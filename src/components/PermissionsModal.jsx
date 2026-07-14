import PermissionRow from "./PermissionRow.jsx";

// Unified "requested permissions" explainer — one list instead of a separate
// one-off modal per OS capability. Shown once automatically (see
// Settings.permissions_prompt_seen) and re-invokable any time from the
// titlebar warning icon while something still needs attention. The same
// capability rows also appear standalone in Settings (see SettingsView's
// "permissions" page) via the shared `PermissionRow`.
export default function PermissionsModal({ t, capabilities, pendingCapability, onAct, onClose }) {
  return (
    <div className="absolute inset-0 z-[100] flex items-center justify-center bg-black/85 p-4">
      <div className="relative flex flex-col w-full max-w-[460px] rounded-2xl border border-stone-700/60 bg-stone-950 shadow-2xl shadow-black/80 text-left">
        <div className="px-6 pt-6 pb-3 shrink-0">
          <h2 className="text-lg font-semibold text-stone-100">{t("permissions.title")}</h2>
          <p className="mt-1 text-sm text-stone-400 leading-relaxed">{t("permissions.body")}</p>
        </div>
        <div className="px-6 divide-y divide-stone-800/70">
          {capabilities.map((c) => (
            <PermissionRow key={c.kind} t={t} capability={c} pending={pendingCapability === c.kind} onAct={onAct} />
          ))}
        </div>
        <div className="px-6 py-4 border-t border-stone-800/60 shrink-0">
          <button onClick={onClose}
            className="w-full rounded-lg border border-stone-700 bg-stone-800/60 px-3.5 py-2 text-sm text-stone-300 hover:bg-stone-700/60 hover:text-stone-100 transition">
            {t("permissions.close")}
          </button>
        </div>
      </div>
    </div>
  );
}
