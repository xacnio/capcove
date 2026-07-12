import { useLanguage } from "../lib/LanguageContext.jsx";
import Screenshot from "./Screenshot.jsx";

const IMAGES = [
  "screenshots/replay.webp",
  "screenshots/area-recording.webp",
  "screenshots/editor-timeline.webp",
  "screenshots/recording-settings.webp",
  "screenshots/youtube-live.webp",
  "screenshots/drive-sync.webp",
  "screenshots/gallery-recordings.webp",
];

function Shot({ i, items, placeholderLabel, className = "" }) {
  const f = items[i];
  return (
    <Screenshot src={IMAGES[i]} alt={f.title} placeholder={f.placeholder} note={placeholderLabel} className={className} />
  );
}

// Seven spreads, each its own layout instead of one repeated image+text
// grid — the rhythm varies with how much room each feature actually needs
// (a settings panel wants more prose, a HUD screenshot wants to be seen large).
export default function Features() {
  const { t } = useLanguage();
  const items = t("features.items");
  const placeholderLabel = t("features.placeholderLabel");

  return (
    <section id="features" className="max-w-6xl mx-auto px-6 py-24 border-t border-stone-800/80">
      <h2 className="text-3xl font-bold tracking-tight">{t("features.title")}</h2>
      <p className="mt-3 text-stone-400 max-w-lg text-lg">{t("features.desc")}</p>

      <div className="mt-16 space-y-24">
        {/* 0 — Instant Replay: full-width image, text in a narrower column below. */}
        <div>
          <Shot i={0} items={items} placeholderLabel={placeholderLabel} />
          <div className="mt-6 max-w-xl">
            <h3 className="text-xl font-semibold text-stone-100">{items[0].title}</h3>
            <p className="mt-2.5 text-stone-400 leading-relaxed">{items[0].desc}</p>
          </div>
        </div>

        {/* 1 — Area & window recording: image right, text left, 50/50. */}
        <div className="grid sm:grid-cols-2 gap-10 items-center">
          <div>
            <h3 className="text-xl font-semibold text-stone-100">{items[1].title}</h3>
            <p className="mt-2.5 text-stone-400 leading-relaxed">{items[1].desc}</p>
          </div>
          <Shot i={1} items={items} placeholderLabel={placeholderLabel} />
        </div>

        {/* 2 — Quick trim tool: mirrors #1, image left. */}
        <div className="grid sm:grid-cols-2 gap-10 items-center">
          <Shot i={2} items={items} placeholderLabel={placeholderLabel} className="sm:order-1" />
          <div className="sm:order-2">
            <h3 className="text-xl font-semibold text-stone-100">{items[2].title}</h3>
            <p className="mt-2.5 text-stone-400 leading-relaxed">{items[2].desc}</p>
          </div>
        </div>

        {/* 3 — Hardware encoding: text-forward, a power-user feature with
            more to say than to show, image kept smaller. */}
        <div className="grid sm:grid-cols-5 gap-10 items-center">
          <div className="sm:col-span-3">
            <h3 className="text-xl font-semibold text-stone-100">{items[3].title}</h3>
            <p className="mt-2.5 text-stone-400 leading-relaxed">{items[3].desc}</p>
          </div>
          <div className="sm:col-span-2">
            <Shot i={3} items={items} placeholderLabel={placeholderLabel} />
          </div>
        </div>

        {/* 4 — YouTube Live: full-width again but capped narrower, to break
            the rhythm from the two 50/50 splits above. */}
        <div className="max-w-3xl">
          <Shot i={4} items={items} placeholderLabel={placeholderLabel} />
          <div className="mt-6">
            <h3 className="text-xl font-semibold text-stone-100">{items[4].title}</h3>
            <p className="mt-2.5 text-stone-400 leading-relaxed max-w-xl">{items[4].desc}</p>
          </div>
        </div>

        {/* 5 — Google Drive sync: image left, text right, with the privacy/
            local-first point spelled out as its own line. */}
        <div className="grid sm:grid-cols-2 gap-10 items-center">
          <Shot i={5} items={items} placeholderLabel={placeholderLabel} />
          <div>
            <h3 className="text-xl font-semibold text-stone-100">{items[5].title}</h3>
            <p className="mt-2.5 text-stone-400 leading-relaxed">{items[5].desc}</p>
            <p className="mt-3 text-sm text-stone-500 leading-relaxed">{items[5].note}</p>
          </div>
        </div>

        {/* 6 — Gallery: closes the sequence at full size again. */}
        <div>
          <div className="max-w-xl mb-6">
            <h3 className="text-xl font-semibold text-stone-100">{items[6].title}</h3>
            <p className="mt-2.5 text-stone-400 leading-relaxed">{items[6].desc}</p>
          </div>
          <Shot i={6} items={items} placeholderLabel={placeholderLabel} />
        </div>
      </div>
    </section>
  );
}
