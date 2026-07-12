import { useEffect, useState } from "react";
import { invoke, listen } from "../lib/tauri.js";
import { Radio, Button, inputCls } from "./settingsUI.jsx";
import { MdClose, MdOpenInNew, MdContentCopy } from "react-icons/md";
import { SiYoutube } from "react-icons/si";

function fmtBytes(bytes) {
  if (!bytes) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  let v = bytes, i = 0;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  return `${v.toFixed(i === 0 ? 0 : 2)} ${units[i]}`;
}

const PRIVACY_OPTIONS = ["public", "unlisted", "private"];

export default function YouTubeUploadModal({ t, path, defaultTitle, connected, onOpenSettings, onClose }) {
  const [title, setTitle] = useState(defaultTitle ?? "");
  const [description, setDescription] = useState("");
  const [privacy, setPrivacy] = useState("unlisted");
  const [status, setStatus] = useState("idle"); // idle | uploading | done | error
  const [progress, setProgress] = useState(null); // { sent, total, bps }
  const [resultUrl, setResultUrl] = useState("");
  const [error, setError] = useState("");

  useEffect(() => {
    let unlisten;
    (async () => {
      unlisten = await listen("youtube-upload-progress", (event) => setProgress(event.payload));
    })();
    return () => unlisten?.();
  }, []);

  const upload = async () => {
    setStatus("uploading");
    setError("");
    setProgress(null);
    try {
      const url = await invoke("upload_video_to_youtube", {
        path, title: title.trim() || defaultTitle, description, privacy,
      });
      setResultUrl(url);
      setStatus("done");
    } catch (e) {
      setError(String(e));
      setStatus("error");
    }
  };

  const busy = status === "uploading";

  return (
    <div className="fixed inset-0 z-50 bg-black/60 flex items-center justify-center p-6" onClick={busy ? undefined : onClose}>
      <div className="w-full max-w-md rounded-xl bg-stone-900 border border-stone-800 p-4" onClick={(e) => e.stopPropagation()}>
        <div className="flex items-center justify-between mb-3">
          <h3 className="text-sm font-semibold text-stone-200 flex items-center gap-2">
            <SiYoutube size={16} className="text-red-500" /> {t("videoEditor.youtubeModal.title")}
          </h3>
          {!busy && (
            <button onClick={onClose} className="text-stone-500 hover:text-stone-200"><MdClose size={16} /></button>
          )}
        </div>

        {!connected ? (
          <div className="flex flex-col gap-3">
            <p className="text-xs text-stone-400">{t("videoEditor.youtubeModal.notConnected")}</p>
            <div className="flex justify-end gap-2">
              <Button onClick={onClose}>{t("videoEditor.youtubeModal.cancel")}</Button>
              <Button variant="primary" onClick={onOpenSettings}>{t("videoEditor.youtubeModal.openSettings")}</Button>
            </div>
          </div>
        ) : status === "done" ? (
          <div className="flex flex-col gap-3">
            <p className="text-xs text-emerald-400">{t("videoEditor.youtubeModal.success")}</p>
            <div className="flex gap-2">
              <Button className="flex-1 flex items-center justify-center gap-1.5" onClick={() => invoke("open_url", { url: resultUrl }).catch(() => window.open(resultUrl, "_blank"))}>
                <MdOpenInNew size={14} /> {t("videoEditor.youtubeModal.openVideo")}
              </Button>
              <Button className="flex items-center justify-center gap-1.5" onClick={() => invoke("copy_text", { text: resultUrl }).catch(() => navigator.clipboard?.writeText(resultUrl))}>
                <MdContentCopy size={14} /> {t("videoEditor.youtubeModal.copyLink")}
              </Button>
            </div>
            <Button variant="primary" onClick={onClose}>{t("videoEditor.youtubeModal.close")}</Button>
          </div>
        ) : (
          <div className="flex flex-col gap-3">
            <div>
              <label className="text-xs text-stone-500 mb-1 block">{t("videoEditor.youtubeModal.titleLabel")}</label>
              <input value={title} onChange={(e) => setTitle(e.target.value)} disabled={busy}
                placeholder={t("videoEditor.youtubeModal.titlePlaceholder")}
                className={`${inputCls} w-full`} />
            </div>
            <div>
              <label className="text-xs text-stone-500 mb-1 block">{t("videoEditor.youtubeModal.descriptionLabel")}</label>
              <textarea value={description} onChange={(e) => setDescription(e.target.value)} disabled={busy}
                placeholder={t("videoEditor.youtubeModal.descriptionPlaceholder")} rows={3}
                className={`${inputCls} w-full resize-none`} />
            </div>
            <div>
              <label className="text-xs text-stone-500 mb-1.5 block">{t("videoEditor.youtubeModal.privacyLabel")}</label>
              <div className="flex flex-col gap-1.5">
                {PRIVACY_OPTIONS.map((opt) => (
                  <button key={opt} type="button" disabled={busy} onClick={() => setPrivacy(opt)}
                    className="flex items-center gap-2 text-left disabled:opacity-50">
                    <Radio checked={privacy === opt} onChange={() => setPrivacy(opt)} />
                    <span className="text-xs text-stone-300">{t(`videoEditor.youtubeModal.${opt}`)}</span>
                  </button>
                ))}
              </div>
            </div>

            {status === "error" && (
              <div className="text-xs text-red-400">{t("videoEditor.youtubeModal.error")}{error}</div>
            )}

            {busy && progress && progress.total > 0 && (
              <div className="flex flex-col gap-1">
                <div className="h-1.5 rounded-full bg-stone-800 overflow-hidden">
                  <div className="h-full bg-accent-400 transition-all" style={{ width: `${Math.min(100, (progress.sent / progress.total) * 100)}%` }} />
                </div>
                <div className="text-[10px] text-stone-500">{fmtBytes(progress.sent)} / {fmtBytes(progress.total)}</div>
              </div>
            )}

            <div className="flex justify-end gap-2 mt-1">
              {!busy && <Button onClick={onClose}>{t("videoEditor.youtubeModal.cancel")}</Button>}
              <Button variant="primary" disabled={busy} onClick={upload}
                className="flex items-center gap-1.5">
                {busy ? t("videoEditor.youtubeModal.uploading") : t("videoEditor.youtubeModal.upload")}
              </Button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
