import { useCallback, useEffect, useMemo, useState } from "react";
import { api, isTauri } from "./lib/api";
import type { Connection, ConnectionInput, McpServerInfo } from "./lib/types";
import AuditDrawer from "./components/AuditDrawer";
import ConnectionModal from "./components/ConnectionModal";
import Sidebar from "./components/Sidebar";
import TabBar, { type TerminalTab } from "./components/TabBar";
import TerminalPane from "./components/TerminalPane";

let tabSeq = 0;
const newTabId = () => `tab_${Date.now().toString(36)}_${(++tabSeq).toString(36)}`;

export default function App() {
  const [connections, setConnections] = useState<Connection[]>([]);
  const [tabs, setTabs] = useState<TerminalTab[]>([]);
  const [activeTabId, setActiveTabId] = useState<string | null>(null);
  const [auditOpen, setAuditOpen] = useState(false);
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

  const openConnection = useCallback((connection: Connection) => {
    setTabs((current) => {
      const existing = current.find((tab) => tab.connectionId === connection.id);
      if (existing) {
        setActiveTabId(existing.id);
        return current;
      }
      const tab = { id: newTabId(), connectionId: connection.id };
      setActiveTabId(tab.id);
      return [...current, tab];
    });
  }, []);

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
        setTabs((current) => current.filter((tab) => tab.connectionId !== connection.id));
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
  const activeConnection = activeTab
    ? connectionById.get(activeTab.connectionId) ?? null
    : null;

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
          <button className="btn btn-ghost" onClick={() => setAuditOpen(true)}>
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
            onSelect={setActiveTabId}
            onClose={closeTab}
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

            {tabs.map((tab) => {
              const connection = connectionById.get(tab.connectionId);
              if (!connection) return null;
              return (
                <TerminalPane
                  key={tab.id}
                  connection={connection}
                  active={tab.id === activeTabId}
                />
              );
            })}
          </div>
        </main>
      </div>

      {auditOpen && <AuditDrawer onClose={() => setAuditOpen(false)} />}

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
