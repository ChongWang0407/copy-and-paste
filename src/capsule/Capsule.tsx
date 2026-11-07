import { useEffect, useState } from "react";
import type { ReactNode } from "react";
import { listen } from "@tauri-apps/api/event";
import { convertFileSrc } from "@tauri-apps/api/core";
import { AnimatePresence, motion } from "framer-motion";
import { Check, CodeXml, File, Files, FileText, Film, Image, Link, Sparkles } from "lucide-react";
import { applyAccent, mirroredAccent } from "../shared/lib/accent";
import { capsuleLocale, capsuleSubtitle, capsuleTitle, type CapsulePayload } from "./capsuleText";

// The pill itself (its glass or black fill, its size) is the native window,
// grown by Rust. This page only draws what was copied, and fades it in once
// the pill has mostly grown, then out before it shrinks.

/** Content waits for the pill to grow most of the way. */
const FRESH_DELAY_S = 0.16;

const kindIcon = (kind: string, fileCount: number): ReactNode => {
  const props = { size: 18, strokeWidth: 2 };
  switch (kind) {
    case "image":
      return <Image {...props} />;
    case "video":
      return <Film {...props} />;
    case "file":
      return fileCount > 1 ? <Files {...props} /> : <File {...props} />;
    case "url":
      return <Link {...props} />;
    case "code":
      return <CodeXml {...props} />;
    case "sample":
      return <Sparkles {...props} />;
    default:
      return <FileText {...props} />;
  }
};

const Capsule = () => {
  const [payload, setPayload] = useState<CapsulePayload | null>(null);
  const [shown, setShown] = useState(false);

  useEffect(() => {
    const show = listen<CapsulePayload>("capsule-show", (event) => {
      // The main window mirrors 主题色 here; it may have changed since.
      applyAccent(mirroredAccent());
      setPayload(event.payload);
      setShown(true);
    });
    const hide = listen("capsule-hide", () => setShown(false));
    return () => {
      void show.then((off) => off());
      void hide.then((off) => off());
    };
  }, []);

  if (!payload) return null;

  const locale = capsuleLocale(payload.language);
  const title = capsuleTitle(payload, locale);
  const subtitle = capsuleSubtitle(payload, locale);
  const dark = payload.style !== "glass";
  const image = payload.imagePath ? convertFileSrc(payload.imagePath) : null;

  return (
    <div
      className={`capsule ${dark ? "is-island" : "is-glass"}`}
      lang={locale === "zh" ? "zh-CN" : "en"}
      style={{ width: payload.width, height: payload.height }}
    >
      <motion.div
        className="capsule-body"
        style={{ top: payload.topInset, height: payload.height - payload.topInset }}
        initial={false}
        animate={shown ? { opacity: 1, scale: 1 } : { opacity: 0, scale: 0.97 }}
        transition={{
          duration: shown ? 0.24 : 0.12,
          delay: shown && payload.fresh ? FRESH_DELAY_S : 0,
          ease: [0.2, 0.8, 0.2, 1],
        }}
      >
        <AnimatePresence mode="popLayout" initial={false}>
          <motion.div
            key={payload.seq}
            className="capsule-row"
            initial={payload.fresh ? false : { opacity: 0, y: 8 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -8 }}
            transition={{ duration: 0.22, ease: [0.2, 0.8, 0.2, 1] }}
          >
            <div className="capsule-icon">
              {payload.appIcon ? <img src={payload.appIcon} alt="" draggable={false} /> : kindIcon(payload.kind, payload.fileCount)}
            </div>
            <div className="capsule-text">
              <div className="capsule-title">
                <span>{title}</span>
                {payload.count > 1 && (
                  <motion.span
                    key={payload.count}
                    className="capsule-count"
                    initial={{ scale: 1.2, opacity: 0 }}
                    animate={{ scale: 1, opacity: 1 }}
                    transition={{ duration: 0.2, ease: [0.2, 0.8, 0.2, 1] }}
                  >
                    ×{payload.count}
                  </motion.span>
                )}
              </div>
              {subtitle && <div className="capsule-subtitle">{subtitle}</div>}
            </div>
            {image ? (
              <img className="capsule-thumb" src={image} alt="" draggable={false} />
            ) : (
              <motion.div
                className="capsule-check"
                initial={{ scale: 0.6, opacity: 0 }}
                animate={{ scale: 1, opacity: 1 }}
                transition={{ duration: 0.22, ease: [0.2, 0.8, 0.2, 1], delay: payload.fresh ? FRESH_DELAY_S + 0.06 : 0.04 }}
              >
                <Check size={13} strokeWidth={3.2} />
              </motion.div>
            )}
          </motion.div>
        </AnimatePresence>
      </motion.div>
    </div>
  );
};

export default Capsule;
