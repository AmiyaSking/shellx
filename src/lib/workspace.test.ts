import { beforeEach, describe, expect, it } from "vitest";
import type { TerminalTab } from "../components/TabBar";
import { loadWorkspace, saveWorkspace } from "./workspace";

function sampleTabs(): TerminalTab[] {
  return [
    {
      id: "tab1",
      panes: [
        { id: "pane1", connectionId: "conn-a" },
        { id: "pane2", connectionId: "conn-b" },
      ],
      paneSizes: [2, 1],
      layout: "row",
      activePaneId: "pane2",
      customTitle: "Ops",
    },
    {
      id: "tab2",
      panes: [{ id: "pane3", connectionId: "conn-c" }],
      paneSizes: [1],
      layout: "column",
      activePaneId: "pane3",
    },
  ];
}

describe("workspace", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it("round-trips tabs, pane sizes and custom titles", () => {
    saveWorkspace(sampleTabs(), "tab2");

    const stored = loadWorkspace();
    expect(stored).not.toBeNull();
    expect(stored?.activeTabIndex).toBe(1);
    expect(stored?.tabs).toHaveLength(2);
    expect(stored?.tabs[0].panes.map((pane) => pane.connectionId)).toEqual([
      "conn-a",
      "conn-b",
    ]);
    expect(stored?.tabs[0].panes[0].size).toBe(2);
    expect(stored?.tabs[0].layout).toBe("row");
    expect(stored?.tabs[0].activePaneIndex).toBe(1);
    expect(stored?.tabs[0].customTitle).toBe("Ops");
  });

  it("returns null for missing or corrupted storage", () => {
    expect(loadWorkspace()).toBeNull();

    window.localStorage.setItem("shellx.workspace.v1", "{not json");
    expect(loadWorkspace()).toBeNull();

    window.localStorage.setItem(
      "shellx.workspace.v1",
      JSON.stringify({ version: 2, tabs: [] }),
    );
    expect(loadWorkspace()).toBeNull();
  });

  it("drops malformed panes while keeping valid tabs", () => {
    window.localStorage.setItem(
      "shellx.workspace.v1",
      JSON.stringify({
        version: 1,
        activeTabIndex: 99,
        tabs: [
          {
            panes: [{ connectionId: "", size: 1 }, { connectionId: "ok", size: 0 }],
            layout: "row",
            activePaneIndex: 5,
          },
          { panes: null, layout: "row" },
        ],
      }),
    );

    const stored = loadWorkspace();
    expect(stored?.tabs).toHaveLength(1);
    expect(stored?.tabs[0].panes).toEqual([{ connectionId: "ok", size: 1 }]);
    expect(stored?.tabs[0].activePaneIndex).toBe(5);
    expect(stored?.activeTabIndex).toBe(99);
  });
});
