import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Check, MousePointerClick, ShieldCheck, type LucideIcon } from "lucide-react";
import AppModal from "./AppModal";
import { isTauriRuntime } from "../lib/tauriRuntime";
import { requestMacAccessibility } from "../lib/macAccessibility";
import type { Translate } from "../types";

interface Permission {
  id: string;
  icon: LucideIcon;
  name: string;
  purpose: string;
  isGranted: () => Promise<boolean>;
  request: () => Promise<void>;
}

/** One row each. Accessibility is the only one the app needs today. */
const PERMISSIONS: Permission[] = [
  {
    id: "accessibility",
    icon: MousePointerClick,
    name: "permission_accessibility",
    purpose: "permission_accessibility_purpose",
    isGranted: () => invoke<boolean>("check_macos_permissions"),
    request: requestMacAccessibility,
  },
];

const CHECK_INTERVAL_MS = 1500;

const readGrants = async () => {
  const entries = await Promise.all(
    PERMISSIONS.map(async (permission) => [permission.id, await permission.isGranted()] as const)
  );
  return Object.fromEntries(entries) as Record<string, boolean>;
};

const allGranted = (grants: Record<string, boolean>) =>
  PERMISSIONS.every((permission) => grants[permission.id]);

/**
 * The liquid-glass dialog of a first start, listing what the app needs. Also
 * comes back when a paste needs a permission that is missing.
 */
export default function PermissionOnboarding({ ready, t, onboardingDone, onDone, pushToast }: {
  ready: boolean;
  t: Translate;
  onboardingDone: boolean;
  onDone: () => void;
  pushToast: (message: string, duration?: number) => void;
}) {
  const [open, setOpen] = useState(false);
  const [grants, setGrants] = useState<Record<string, boolean>>({});
  /** Set while the user is away granting: the window is lowered for that. */
  const authorizing = useRef(false);
  const startup = useRef({ onboardingDone, onDone });
  startup.current = { onboardingDone, onDone };

  useEffect(() => {
    if (!ready || !isTauriRuntime()) return;
    let alive = true;
    readGrants()
      .then((current) => {
        if (!alive) return;
        setGrants(current);
        if (startup.current.onboardingDone) return;
        if (allGranted(current)) startup.current.onDone();
        else setOpen(true);
      })
      .catch(console.error);
    const subscription = listen("accessibility-permission-required", () => setOpen(true));
    return () => {
      alive = false;
      void subscription.then((off) => off());
    };
  }, [ready]);

  const stopAuthorizing = useCallback(() => {
    if (!authorizing.current) return;
    authorizing.current = false;
    invoke("set_panel_lowered", { lowered: false }).catch(console.error);
  }, []);

  useEffect(() => {
    if (!open) return;
    const check = () => {
      readGrants()
        .then((current) => {
          setGrants(current);
          if (authorizing.current && allGranted(current)) {
            // Back in front of System Settings, showing the result.
            stopAuthorizing();
            invoke("focus_clipboard_window").catch(console.error);
          }
        })
        .catch(console.error);
    };
    const interval = window.setInterval(check, CHECK_INTERVAL_MS);
    return () => window.clearInterval(interval);
  }, [open, stopAuthorizing]);

  const authorize = async (permission: Permission) => {
    try {
      // The panel floats over every app, and would cover System Settings.
      authorizing.current = true;
      await invoke("set_panel_lowered", { lowered: true });
      await permission.request();
    } catch {
      pushToast(t("accessibility_settings_failed"), 4000);
    }
  };

  const finish = () => {
    stopAuthorizing();
    if (!onboardingDone) onDone();
    setOpen(false);
  };

  const everythingGranted = allGranted(grants);

  return (
    <AppModal
      open={open}
      onClose={finish}
      closeOnOverlayClick={false}
      panelClassName="modal-panel permission-panel glass-popover"
      ariaLabel={t("onboarding_title")}
    >
      <div className="permission-badge" aria-hidden="true">
        <ShieldCheck size={22} />
      </div>
      <h3 className="permission-title">{t("onboarding_title")}</h3>
      <p className="permission-intro">{t("onboarding_intro")}</p>

      <ul className="permission-list">
        {PERMISSIONS.map((permission) => {
          const Icon = permission.icon;
          const granted = grants[permission.id];
          return (
            <li key={permission.id} className="permission-row">
              <span className="permission-icon" aria-hidden="true">
                <Icon size={17} />
              </span>
              <span className="permission-text">
                <span className="permission-name">{t(permission.name)}</span>
                <span className="permission-purpose">{t(permission.purpose)}</span>
              </span>
              {granted ? (
                <span className="permission-granted">
                  <Check size={13} strokeWidth={3} />
                  {t("onboarding_granted")}
                </span>
              ) : (
                <button
                  type="button"
                  className="glass-button is-primary is-compact"
                  onClick={() => void authorize(permission)}
                >
                  {t("onboarding_authorize")}
                </button>
              )}
            </li>
          );
        })}
      </ul>

      <p className="permission-note">{t("onboarding_files_note")}</p>

      <div className="modal-actions">
        <button
          type="button"
          className={`glass-button${everythingGranted ? " is-primary" : ""}`}
          onClick={finish}
        >
          {t(everythingGranted ? "onboarding_start" : "accessibility_later")}
        </button>
      </div>
    </AppModal>
  );
}
