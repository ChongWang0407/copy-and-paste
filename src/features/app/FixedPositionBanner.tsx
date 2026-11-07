import { invoke } from "@tauri-apps/api/core";
import { startWindowDrag } from "../../shared/lib/windowDrag";
import type { Translate } from "../../shared/types";

interface FixedPositionBannerProps {
  t: Translate;
  onDone: (saved: boolean) => void;
}

/** 固定位置: the window stays open until the user has put it where they want it. */
const FixedPositionBanner = ({ t, onDone }: FixedPositionBannerProps) => (
  <div className="fixed-banner glass-popover" onMouseDown={startWindowDrag}>
    <span className="fixed-banner-text">{t("fixed_banner_text")}</span>
    <button
      type="button"
      className="glass-button"
      onClick={() => {
        invoke("cancel_fixed_position_setup").catch(console.error);
        onDone(false);
      }}
    >
      {t("cancel")}
    </button>
    <button
      type="button"
      className="glass-button is-primary"
      onClick={() => {
        invoke("save_fixed_window_position")
          .then(() => onDone(true))
          .catch(console.error);
      }}
    >
      {t("fixed_banner_save")}
    </button>
  </div>
);

export default FixedPositionBanner;
