import { useCallback, useEffect, useMemo, useState } from "react";
import { api, isTauri } from "./lib/api";
import type { Connection, ConnectionInput, McpServerInfo } from "./lib/types";
import AuditDrawer from "./components/AuditDrawer";
import ConnectionModal from "./components/ConnectionModal";
import SettingsDrawer from "./components/SettingsDrawer";
import Sidebar from "./components/Sidebar";
import TabBar, {
  type TerminalPaneState,
  type TerminalTab,
} from "./components/TabBar";
import TerminalPane from "./components/TerminalPane";

let tabSeq = 0;
let paneSeq = 0;
const newTabId = () => `tab_${Date.now().toString(36)}_${(++tabSeq).toString(36)}`;
const newPaneId = () => `pane_${Date.now().toString(36)}_${(++paneSeq).toString(36)}`;

export default function App() {
  const [connections, setConnections] = useState<Connection[]>([]);
  const [tabs, setTabs] = useState<TerminalTab[]>([]);
  const [activeTabId, setActiveTabId] = useState<string | null>(null);
  const [auditOpen, setAuditOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [mcpInfo, setMcpInfo] = useState<McpServerInfo | null>(null);
  const [modal, setModal] = useState<
    { mode: "create" } | { mode: "edit"; connection: Connection } | null
  >(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  const connectionById = useMemo(
    () => new Map(connections.map((connection) => [connection.id, connection])),
    [connections],
  );

  const refreshConnections = useCallback(async () => {
    try {
      setConnections(await api.listConnections());
    } catch (err) {
      setError(String(err));
    }
  }, []);

  const refreshMcpInfo = useCallback(async () => {
    try {
      setMcpInfo(await api.mcpServerInfo());
    } catch {
      // MCP status is informational; ignore failures in the scaffold.
    }
  }, []);

  useEffect(() => {
    void (async () => {
      await Promise.all([refreshConnections(), refreshMcpInfo()]);
      setLoading(false);
    })();
  }, [refreshConnections, refreshMcpInfo]);

  const selectTab = useCallback((tabId: string) => {
    setActiveTabId(tabId);
    setTabs((current) =>
      current.map((tab) =>
        tab.id === tabId ? { ...tab, attention: false } : tab,
      ),
    );
  }, []);

  const focusPane = useCallback((tabId: string, paneId: string) => {
    setActiveTabId(tabId);
    setTabs((current) =>
      current.map((tab) =>
        tab.id === tabId
          ? { ...tab, activePaneId: paneId, attention: false }
          : tab,
      ),
    );
  }, []);

  const createTab = useCallback((connectionId: string) => {
    const pane: TerminalPaneState = { id: newPaneId(), connectionId };
    const tab: TerminalTab = {
      id: newTabId(),
      panes: [pane],
      layout: "row",
      activePaneId: pane.id,
    };
    setTabs((current) => [...current, tab]);
    setActiveTabId(tab.id);
    return tab.id;
  }, []);

  const openConnection = useCallback(
    (connection: Connection) => {
      const existing = tabs.find((tab) =>
        tab.panes.some((pane) => pane.connectionId === connection.id),
      );
      if (existing) {
        const pane = existing.panes.find(
          (item) => item.connectionId === connection.id,
        );
        focusPane(existing.id, pane?.id ?? existing.activePaneId);
        return;
      }
      createTab(connection.id);
    },
    [createTab, focusPane, tabs],
  );

  const closeTab = useCallback((tabId: string) => {
    setTabs((current) => {
      const index = current.findIndex((tab) => tab.id === tabId);
      const next = current.filter((tab) => tab.id !== tabId);
      setActiveTabId((active) => {
        if (active !== tabId) return active;
        if (next.length === 0) return null;
        return next[Math.max(0, index - 1)]?.id ?? next[0].id;
      });
      return next;
    });
  }, []);

  const updateTabTitle = useCallback((tabId: string, title: string) => {
    setTabs((current) =>
      current.map((tab) => (tab.id === tabId ? { ...tab, title } : tab)),
    );
  }, []);

  const markTabAttention = useCallback(
    (tabId: string) => {
      if (tabId === activeTabId) return;
      setTabs((current) =>
        current.map((tab) =>
          tab.id === tabId ? { ...tab, attention: true } : tab,
        ),
      );
    },
    [activeTabId],
  );

  const renameTab = useCallback((tabId: string, title: string) => {
    const cleaned = title.trim().slice(0, 80);
    setTabs((current) =>
      current.map((tab) =>
        tab.id === tabId
          ? { ...tab, customTitle: cleaned || undefined }
          : tab,
      ),
    );
  }, []);

  const closePane = useCallback(
    (tabId: string, paneId: string) => {
      const tab = tabs.find((item) => item.id === tabId);
      if (!tab) return;
      if (tab.panes.length <= 1) {
        closeTab(tabId);
        return;
      }
      setTabs((current) =>
        current.map((item) => {
          if (item.id !== tabId) return item;
          const index = item.panes.findIndex((pane) => pane.id === paneId);
          const panes = item.panes.filter((pane) => pane.id !== paneId);
          if (panes.length === 0) return item;
          const activePaneId =
            item.activePaneId === paneId
              ? panes[Math.max(0, index - 1)]?.id ?? panes[0].id
              : item.activePaneId;
          return { ...item, panes, activePaneId };
        }),
      );
    },
    [closeTab, tabs],
  );

  const splitPane = useCallback(
    (tabId: string, paneId: string, direction: "row" | "column") => {
      const tab = tabs.find((item) => item.id === tabId);
      const source = tab?.panes.find((pane) => pane.id === paneId);
      if (!tab || !source) return;
      if (tab.panes.length >= 4) {
        setError("每个标签最多支持 4 个终端窗格");
        return;
      }
      const pane: TerminalPaneState = {
        id: newPaneId(),
        connectionId: source.connectionId,
      };
      setTabs((current) =>
        current.map((item) =>
          item.id === tabId
            ? {
                ...item,
                panes: [...item.panes, pane],
                layout: direction,
                activePaneId: pane.id,
              }
            : item,
        ),
      );
      setActiveTabId(tabId);
    },
    [tabs],
  );

  const moveTab = useCallback((fromTabId: string, toTabId: string) => {
    if (fromTabId === toTabId) return;
    setTabs((current) => {
      const from = current.findIndex((tab) => tab.id === fromTabId);
      const to = current.findIndex((tab) => tab.id === toTabId);
      if (from < 0 || to < 0) return current;
      const next = [...current];
      const [moved] = next.splice(from, 1);
      next.splice(to, 0, moved);
      return next;
    });
  }, []);

  const handleSaved = useCallback(
    async (input: ConnectionInput) => {
      try {
        const saved = await api.saveConnection(input);
        await refreshConnections();
        await refreshMcpInfo();
        setModal(null);
        openConnection(saved);
        setError(null);
      } catch (err) {
        setError(String(err));
      }
    },
    [openConnection, refreshConnections, refreshMcpInfo],
  );

  const handleDelete = useCallback(
    async (connection: Connection) => {
      if (!window.confirm(`确认删除连接「${connection.alias}」？该操作不会影响远程服务器。`)) {
        return;
      }
      try {
        await api.deleteConnection(connection.id);
        setTabs((current) => {
          const next = current.flatMap((tab) => {
            const panes = tab.panes.filter(
              (pane) => pane.connectionId !== connection.id,
            );
            if (panes.length === 0) return [];
            const activePaneId = panes.some(
              (pane) => pane.id === tab.activePaneId,
            )
              ? tab.activePaneId
              : panes[0].id;
            return [{ ...tab, panes, activePaneId }];
          });
          setActiveTabId((active) => {
            if (active && next.some((tab) => tab.id === active)) return active;
            return next[next.length - 1]?.id ?? null;
          });
          return next;
        });
        setModal(null);
        await refreshConnections();
        await refreshMcpInfo();
      } catch (err) {
        setError(String(err));
      }
    },
    [refreshConnections, refreshMcpInfo],
  );

  const activeTab = tabs.find((tab) => tab.id === activeTabId) ?? null;
  const activePane = activeTab
    ? (activeTab.panes.find((pane) => pane.id === activeTab.activePaneId) ??
      activeTab.panes[0] ??
      null)
    : null;
  const activeConnection = activePane
    ? (connectionById.get(activePane.connectionId) ?? null)
    : null;

  const duplicateActiveTab = useCallback(() => {
    if (activeConnection) {
      createTab(activeConnection.id);
    } else {
      setModal({ mode: "create" });
    }
  }, [activeConnection, createTab]);

  const closeActivePane = useCallback(() => {
    if (activeTab && activePane) {
      closePane(activeTab.id, activePane.id);
    }
  }, [activePane, activeTab, closePane]);

  const splitActivePane = useCallback(
    (direction: "row" | "column") => {
      if (activeTab && activePane) {
        splitPane(activeTab.id, activePane.id, direction);
      }
    },
    [activePane, activeTab, splitPane],
  );

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const mod = event.ctrlKey || event.metaKey;
      if (!mod || modal || auditOpen || settingsOpen) return;
      const target = event.target as HTMLElement | null;
      const typing =
        target !== null &&
        (target.tagName === "INPUT" ||
          target.tagName === "TEXTAREA" ||
          target.isContentEditable) &&
        !target.classList.contains("xterm-helper-textarea");
      if (typing) return;

      const key = event.key.toLowerCase();
      if (event.shiftKey && key === "t") {
        event.preventDefault();
        event.stopPropagation();
        duplicateActiveTab();
        return;
      }
      if (event.shiftKey && key === "w") {
        event.preventDefault();
        event.stopPropagation();
        closeActivePane();
        return;
      }
      if (event.shiftKey && key === "d") {
        event.preventDefault();
        event.stopPropagation();
        splitActivePane("row");
        return;
      }
      if (event.shiftKey && key === "e") {
        event.preventDefault();
        event.stopPropagation();
        splitActivePane("column");
        return;
      }
      if (key === "tab" && tabs.length > 0) {
        event.preventDefault();
        event.stopPropagation();
        const index = tabs.findIndex((tab) => tab.id === activeTabId);
        const step = event.shiftKey ? -1 : 1;
        const next = tabs[(index + step + tabs.length) % tabs.length] ?? tabs[0];
        if (next) selectTab(next.id);
        return;
      }
      if (!event.shiftKey && /^[1-9]$/.test(key)) {
        const index = Number(key) - 1;
        const tab = tabs[index];
        if (tab) {
          event.preventDefault();
          event.stopPropagation();
          selectTab(tab.id);
        }
      }
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [
    activeTabId,
    auditOpen,
    closeActivePane,
    duplicateActiveTab,
    modal,
    selectTab,
    settingsOpen,
    splitActivePane,
    tabs,
  ]);

  return (
    <div className="app">
      <header className="topbar">
        <div className="brand">
          <span className="brand-mark">SX</span>
          <div>
            <div className="brand-name">ShellX</div>
            <div className="brand-sub">安全 MCP 终端 · 脚手架</div>
          </div>
        </div>
        <div className="topbar-meta">
          {!isTauri() && <span className="chip chip-mock">浏览器 Mock 模式</span>}
          <span className="chip">{connections.length} 个连接</span>
          <span className="chip">{tabs.length} 个会话页</span>
          <button
            className="btn btn-ghost"
            onClick={() => {
              setSettingsOpen(true);
              setAuditOpen(false);
            }}
          >
            设置
          </button>
          <button
            className="btn btn-ghost"
            onClick={() => {
              setAuditOpen(true);
              setSettingsOpen(false);
            }}
          >
            审计日志
          </button>
        </div>
      </header>

      {error && (
        <div className="error-banner">
          <span>{error}</span>
          <button className="btn btn-ghost btn-sm" onClick={() => setError(null)}>
            关闭
          </button>
        </div>
      )}

      <div className="app-body">
        <Sidebar
          connections={connections}
          activeConnectionId={activeConnection?.id ?? null}
          loading={loading}
          mcpInfo={mcpInfo}
          onOpen={openConnection}
          onNew={() => setModal({ mode: "create" })}
          onEdit={(connection) => setModal({ mode: "edit", connection })}
        />

        <main className="workspace">
          <TabBar
            tabs={tabs}
            connections={connectionById}
            activeTabId={activeTabId}
            onSelect={selectTab}
            onClose={closeTab}
            onNew={duplicateActiveTab}
            onRename={renameTab}
            onMove={moveTab}
          />

          <div className="terminal-area">
            {tabs.length === 0 && (
              <div className="empty-state">
                <div className="empty-icon">⌁</div>
                <h2>没有打开的终端</h2>
                <p>
                  从左侧选择一个服务器开始，或先创建一条连接。新建连接默认
                  <strong> 不对 MCP 开放</strong>。
                </p>
                <button className="btn btn-primary" onClick={() => setModal({ mode: "create" })}>
                  新建连接
                </button>
              </div>
            )}

            {tabs.map((tab) => (
              <div
                key={tab.id}
                className={`tab-panes tab-panes-${tab.layout} ${
                  tab.id === activeTabId ? "is-active" : ""
                }`}
              >
                {tab.panes.map((pane) => {
                  const connection = connectionById.get(pane.connectionId);
                  if (!connection) return null;
                  return (
                    <TerminalPane
                      key={pane.id}
                      connection={connection}
                      visible={tab.id === activeTabId}
                      focused={
                        tab.id === activeTabId &&
                        pane.id === tab.activePaneId
                      }
                      canSplit={tab.panes.length < 4}
                      onFocus={() => focusPane(tab.id, pane.id)}
                      onTitleChange={(title) => updateTabTitle(tab.id, title)}
                      onBell={() => markTabAttention(tab.id)}
                      onSplitRow={() => splitPane(tab.id, pane.id, "row")}
                      onSplitColumn={() =>
                        splitPane(tab.id, pane.id, "column")
                      }
                      onClosePane={() => closePane(tab.id, pane.id)}
                    />
                  );
                })}
              </div>
            ))}
          </div>
        </main>
      </div>

      {auditOpen && <AuditDrawer onClose={() => setAuditOpen(false)} />}
      {settingsOpen && <SettingsDrawer onClose={() => setSettingsOpen(false)} />}

      {modal && (
        <ConnectionModal
          mode={modal.mode}
          connection={modal.mode === "edit" ? modal.connection : null}
          connections={connections}
          onSave={handleSaved}
          onDelete={
            modal.mode === "edit"
              ? () => handleDelete(modal.connection)
              : undefined
          }
          onClose={() => setModal(null)}
        />
      )}
    </div>
  );
}
