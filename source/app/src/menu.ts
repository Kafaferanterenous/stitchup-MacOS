//! Native macOS menu bar, built from the command registry with the same menus
//! as the Windows build (File · Export · Edit · Pages · Annotate · View · Tabs
//! · Help) plus the standard macOS app menu. Accelerators live on the menu
//! items, so the JS key handler skips them while the native menu is active.

import { invoke } from "@tauri-apps/api/core";
import { Menu, Submenu, MenuItem, PredefinedMenuItem, CheckMenuItem } from "@tauri-apps/api/menu";
import { COMMANDS, MENUS, byId, runCommand } from "./commands";
import { store, subscribe, getState, RIBBON_TABS } from "./store";
import { THEMES, applyTheme, currentTheme } from "./themes/theme";

let native = false;
export const nativeMenuActive = () => native;

type Item = MenuItem | Submenu | PredefinedMenuItem | CheckMenuItem;

export async function installMenu(): Promise<void> {
  try {
    const items = new Map<string, MenuItem>();
    const make = async (id: string): Promise<MenuItem> => {
      const c = byId.get(id)!;
      const text = (c.menu ?? c.label) + (c.soon ? " (coming soon)" : "");
      const mi = await MenuItem.new({ id, text, accelerator: c.accel, action: () => void runCommand(id) });
      items.set(id, mi);
      return mi;
    };
    const sep = () => PredefinedMenuItem.new({ item: "Separator" });

    const schemeItems = await Promise.all(
      THEMES.map((t) =>
        CheckMenuItem.new({ id: `scheme:${t.name}`, text: t.label, checked: currentTheme() === t.name, action: () => void applyTheme(t.name).then(syncSchemes) }),
      ),
    );
    async function syncSchemes() {
      for (const [i, t] of THEMES.entries()) await schemeItems[i].setChecked(currentTheme() === t.name);
    }
    const tabItems = await Promise.all(
      RIBBON_TABS.map((t) => MenuItem.new({ id: `tab:${t}`, text: t, action: () => store.setRibbonTab(t) })),
    );

    const submenus: Record<string, () => Promise<Item[]>> = {
      clipboard: async () => [
        await PredefinedMenuItem.new({ item: "Cut" }),
        await PredefinedMenuItem.new({ item: "Copy" }),
        await PredefinedMenuItem.new({ item: "Paste" }),
      ],
      schemes: async () => [
        await Submenu.new({ text: "Colour Scheme", items: [...schemeItems, await sep(), await make("darkMode")] }),
      ],
      ribbonTabs: async () => tabItems,
    };

    const bar: Submenu[] = [
      await Submenu.new({
        text: "Stitchup",
        items: [
          await make("about"),
          await sep(),
          await PredefinedMenuItem.new({ item: "Services" }),
          await sep(),
          await PredefinedMenuItem.new({ item: "Hide" }),
          await PredefinedMenuItem.new({ item: "HideOthers" }),
          await PredefinedMenuItem.new({ item: "ShowAll" }),
          await sep(),
          await make("exit"),
        ],
      }),
    ];
    for (const m of MENUS) {
      const its: Item[] = [];
      for (const it of m.items) {
        if (it === "-") its.push(await sep());
        else if (typeof it === "string") its.push(await make(it));
        else its.push(...(await submenus[it.submenu]()));
      }
      if (m.title === "View") {
        // Standard macOS full-screen entry.
        its.push(await sep(), await PredefinedMenuItem.new({ item: "Fullscreen" }));
      }
      if (m.title === "Help") bar.push(await Submenu.new({ text: "Window", items: [await PredefinedMenuItem.new({ item: "Minimize" }), await PredefinedMenuItem.new({ item: "Maximize" })] }));
      bar.push(await Submenu.new({ text: m.title, items: its }));
    }
    const menu = await Menu.new({ items: bar });
    await menu.setAsAppMenu();
    native = true;
    void invoke("log_message", { msg: `native menu installed (${bar.length} menus, ${items.size} commands)` }).catch(() => {});

    // Grey out items that can't run right now (only when the answer changes).
    const last = new Map<string, boolean>();
    const refresh = () => {
      const s = getState();
      for (const c of COMMANDS) {
        const mi = items.get(c.id);
        if (!mi) continue;
        const on = !c.soon && (!c.enabled || c.enabled(s));
        if (last.get(c.id) !== on) {
          last.set(c.id, on);
          void mi.setEnabled(on);
        }
      }
    };
    refresh();
    subscribe(refresh);
  } catch (e) {
    void invoke("log_message", { msg: `native menu unavailable: ${String((e as Error)?.message ?? JSON.stringify(e))}` }).catch(() => {});
    // Not running inside Tauri (browser / tests): the in-window ribbon and the
    // JS key handler provide everything.
    native = false;
  }
}
