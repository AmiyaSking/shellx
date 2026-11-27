import type { TerminalTab } from "../components/TabBar";

const KEY = "shellx.workspace.v1";

export interface StoredPane {
  connectionId: string;
  size: number;
}

export interface StoredTab {
  panes: StoredPane[];
  layout: "row" | "column";
  activePaneIndex: number;
  customTitle?: string;
}

export interface StoredWorkspace {
  version: 1;
  savedAt: string;
  activeTabIndex: number;
  tabs: StoredTab[];
}

/** Persist only layout metadata: no output, secrets or remote titles. */
export function saveWorkspace(
  tabs: TerminalTab[],
  activeTabId: string | null,
): void {
  const activeTabIndex = Math.max(
    0,
    tabs.findIndex((tab) => tab.id === activeTabId),
  );
  const data: StoredWorkspace = {
    version: 1,
    savedAt: new Date().toISOString(),
    activeTabIndex,
    tabs: tabs.map((tab) => ({
      panes: tab.panes.map((pane, index) => ({
        connectionId: pane.connectionId,
        size: tab.paneSizes[index] ?? 1,
      })),
      layout: tab.layout,
      activePaneIndex: Math.max(
        0,
        tab.panes.findIndex((pane) => pane.id === tab.activePaneId),
      ),
      customTitle: tab.customTitle,
    })),
  };
  try {
    window.localStorage.setItem(KEY, JSON.stringify(data));
  } catch {
    // Workspace persistence is best-effort.
  }
}

export function loadWorkspace(): StoredWorkspace | null {
  try {
    const raw = window.localStorage.getItem(KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<StoredWorkspace> | null;
    if (!parsed || parsed.version !== 1 || !Array.isArray(parsed.tabs)) {
      return null;
    }
    const tabs: StoredTab[] = [];
    for (const tab of parsed.tabs) {
      if (!tab || !Array.isArray(tab.panes) || tab.panes.length === 0) {
        continue;
      }
      const panes = tab.panes.filter(
        (pane) =>
          pane &&
          typeof pane.connectionId === "string" &&
          pane.connectionId.length > 0,
      );
      if (panes.length === 0) continue;
      tabs.push({
        panes: panes.map((pane) => ({
          connectionId: pane.connectionId,
          size:
            Number.isFinite(pane.size) && pane.size > 0 ? pane.size : 1,
        })),
        layout: tab.layout === "column" ? "column" : "row",
        activePaneIndex:
          Number.isInteger(tab.activePaneIndex) && tab.activePaneIndex >= 0
            ? Math.min(tab.activePaneIndex, panes.length - 1)
            : 0,
        customTitle:
          typeof tab.customTitle === "string" ? tab.customTitle : undefined,
      });
    }
    if (tabs.length === 0) return null;
    const activeTabIndex = parsed.activeTabIndex;
    return {
      version: 1,
      savedAt: typeof parsed.savedAt === "string" ? parsed.savedAt : "",
      activeTabIndex:
        typeof activeTabIndex === "number" &&
        Number.isInteger(activeTabIndex) &&
        activeTabIndex >= 0
          ? Math.min(activeTabIndex, tabs.length - 1)
          : 0,
      tabs,
    };
  } catch {
    return null;
  }
}
