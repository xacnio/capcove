import { MdRefresh } from "react-icons/md";
import * as Icon from "../gallery/icons.jsx";
import { Row } from "./settingsUI.jsx";

// One capability's row: label/description (+ a denied note) via the shared
// `Row`, status or an action button on the right. Used both standalone in
// Settings and inside `PermissionsModal`'s first-run explainer, so the two
// stay visually and behaviorally identical. `pending` disables the button and
// swaps its label for a spinner while `onAct` is in flight — showing the OS
// consent prompt itself can take a beat.
export default function PermissionRow({ t, capability, pending, onAct }) {
  const { kind, status } = capability;
  const copy = t(`permissions.${kind}`);
  const hint = status === "denied" ? `${copy.desc} ${t("permissions.denied")}` : copy.desc;
  return (
    <Row label={copy.label} hint={hint}>
      {status === "granted" ? (
        <span className="flex items-center gap-1.5 text-xs font-medium text-emerald-400">
          <Icon.Check size={14} />
          {t("permissions.granted")}
        </span>
      ) : (
        <button onClick={() => onAct(kind, status)} disabled={pending}
          className="flex items-center gap-1.5 rounded-lg bg-accent-500 px-3 py-1.5 text-xs font-medium text-stone-950 transition hover:bg-accent-400 disabled:opacity-70">
          {pending && <MdRefresh size={13} className="animate-spin" />}
          {status === "denied" ? t("permissions.openSettings") : t("permissions.allow")}
        </button>
      )}
    </Row>
  );
}
