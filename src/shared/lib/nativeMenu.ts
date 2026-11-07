import { CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu } from "@tauri-apps/api/menu";
import { LogicalPosition } from "@tauri-apps/api/dpi";
import { acquireBlurGuard, releaseBlurGuard } from "./focus";

/**
 * Right-click menus are the system's own: real Liquid Glass that refracts
 * what's behind it, which no web layer can do.
 */
export interface NativeMenuItem {
  text?: string;
  /** Shows a check mark. */
  checked?: boolean;
  action?: () => void;
  children?: NativeMenuItem[];
  separator?: boolean;
}

let menuSequence = 0;
/**
 * The menu shown last. A click can arrive just after a menu closes, so it stays
 * alive until the next one opens.
 */
let previousMenu: Menu | null = null;
/** Menus open strictly one after another. */
let queue: Promise<void> = Promise.resolve();

type Choose = (action: () => void) => () => void;

type BuiltItem = MenuItem | CheckMenuItem | PredefinedMenuItem | Submenu;

const buildItem = async (item: NativeMenuItem, choose: Choose): Promise<BuiltItem> => {
  if (item.separator) return PredefinedMenuItem.new({ item: "Separator" });
  if (item.children) {
    const items = await Promise.all(item.children.map((child) => buildItem(child, choose)));
    return Submenu.new({ text: item.text ?? "", items });
  }
  const action = item.action ? choose(item.action) : undefined;
  if (item.checked !== undefined) {
    return CheckMenuItem.new({ text: item.text ?? "", checked: item.checked, action });
  }
  return MenuItem.new({ text: item.text ?? "", action });
};

/**
 * The non-activating panel would otherwise hide itself as the menu takes key
 * focus; the chosen action runs only once that protection is lifted again.
 *
 * Tauri keeps its resource table locked for as long as a menu is on screen, and
 * the main thread, which runs the menu, can't wait for that lock. Any call that
 * touches a menu resource during that time, like a delayed close of the last
 * menu, froze the whole app. So all of them happen here, before the popup.
 */
const present = async (items: NativeMenuItem[], at?: { x: number; y: number }) => {
  const owner = `native-menu:${++menuSequence}`;
  let finished = false;
  let chosen: (() => void) | null = null;
  const choose: Choose = (action) => () => {
    if (finished) action();
    else chosen = action;
  };

  await acquireBlurGuard(owner);
  try {
    await previousMenu?.close().catch(() => {});
    previousMenu = null;
    const built = await Promise.all(items.map((item) => buildItem(item, choose)));
    const menu = await Menu.new({ items: built });
    previousMenu = menu;
    await menu.popup(at ? new LogicalPosition(at.x, at.y) : undefined);
  } catch (error) {
    console.error("Menu failed", error);
  } finally {
    await releaseBlurGuard(owner).catch(() => {});
    finished = true;
    const run = chosen as (() => void) | null;
    run?.();
  }
};

/** Shows the menu at the pointer, or at `at`, in window points. */
export const showNativeMenu = (items: NativeMenuItem[], at?: { x: number; y: number }) => {
  const shown = queue.then(() => present(items, at));
  queue = shown.catch(() => {});
  return shown;
};
