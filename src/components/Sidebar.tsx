import { useMemo, useState } from "react";
import {
  ENV_LABELS,
  MCP_ACCESS_LABELS,
  type Connection,
  type McpServerInfo,
} from "../lib/types";

interface Props {
  connections: Connection[];
  activeConnectionId: string | null;
  loading: boolean;
  mcpInfo: McpServerInfo | null;
  onOpen: (connection: Connection) => void;
  onNew: () => void;
  onImport: () => void;
  onEdit: (connection: Connection) => void;
}

export default function Sidebar({
  connections,
  activeConnectionId,
  loading,
  mcpInfo,
  onOpen,
  onNew,
  onImport,
  onEdit,
}: Props) {
  const [query, setQuery] = useState("");

  const groups = useMemo(() => {
    const keyword = query.trim().toLowerCase();
    const filtered = connections.filter((connection) => {
      if (!keyword) return true;
      return [connection.alias, connection.host, connection.group, connection.username, ...connection.tags]
        .join(" ")
        .toLowerCase()
        .includes(keyword);
    });
    const map = new Map<string, Connection[]>();
    for (const connection of filtered) {
      const key = connection.group.trim() || "未分组";
      const bucket = map.get(key) ?? [];
      bucket.push(connection);
      map.set(key, bucket);
    }
    return [...map.entries()].sort((a, b) => a[0].localeCompare(b[0], "zh-CN"));
  }, [connections, query]);

  return (
    <aside className="sidebar">
      <div className="sidebar-head">
        <div>
          <div className="section-title">连接</div>
          <div className="section-sub">{connections.length} 台服务器</div>
        </div>
        <div className="sidebar-head-actions">
          <button
            className="btn btn-ghost btn-sm"
            title="从 ~/.ssh/config 导入"
            onClick={onImport}
          >
            导入
          </button>
          <button className="btn btn-primary btn-sm" onClick={onNew}>
            + 新建
          </button>
        </div>
      </div>

      <input
        className="search"
        placeholder="搜索别名 / 主机 / 标签…"
        value={query}
        onChange={(event) => setQuery(event.target.value)}
      />

      <div className="connection-list">
        {loading && <div className="muted pad">加载中…</div>}
        {!loading && connections.length === 0 && (
          <div className="empty-hint">
            <p>还没有服务器连接。</p>
            <p>新建连接后，可在此打开终端；对 MCP 的授权默认关闭。</p>
          </div>
        )}
        {!loading && connections.length > 0 && groups.length === 0 && (
          <div className="muted pad">没有匹配的连接</div>
        )}

        {groups.map(([group, items]) => (
          <div className="connection-group" key={group}>
            <div className="group-label">
              <span>{group}</span>
              <span className="group-count">{items.length}</span>
            </div>
            {items.map((connection) => (
              <div
                key={connection.id}
                className={`connection-card ${
                  connection.id === activeConnectionId ? "is-active" : ""
                }`}
                onClick={() => onOpen(connection)}
              >
                <div className="connection-main">
                  <div className="connection-line">
                    <span className={`env-dot env-${connection.environment}`} />
                    <span className="connection-alias">{connection.alias}</span>
                    <span className={`badge badge-${connection.environment}`}>
                      {ENV_LABELS[connection.environment]}
                    </span>
                  </div>
                  <div className="connection-sub">
                    {connection.username}@{connection.host}:{connection.port}
                  </div>
                  <div className="connection-tags">
                    <span
                      className={`mini-badge mcp-${connection.mcpAccess}`}
                      title="MCP 授权状态"
                    >
                      MCP · {MCP_ACCESS_LABELS[connection.mcpAccess]}
                    </span>
                    {connection.mcpPtyEnabled && (
                      <span
                        className="mini-badge mcp-pty"
                        title="MCP 交互式终端已开启（危险：绕过命令级策略）"
                      >
                        PTY ⚠
                      </span>
                    )}
                    {connection.tags.slice(0, 3).map((tag) => (
                      <span className="mini-badge" key={tag}>
                        {tag}
                      </span>
                    ))}
                  </div>
                </div>
                <button
                  type="button"
                  className="icon-btn"
                  title="编辑连接"
                  onClick={(event) => {
                    event.stopPropagation();
                    onEdit(connection);
                  }}
                >
                  ⋯
                </button>
              </div>
            ))}
          </div>
        ))}
      </div>

      <div className="mcp-card">
        <div className="mcp-card-head">
          <span className="section-title">本地 MCP</span>
          <span className={`status-dot status-${mcpInfo?.status ?? "stopped"}`} />
        </div>
        {mcpInfo ? (
          <>
            <div className="mcp-line">
              <span>传输</span>
              <code>{mcpInfo.transport}</code>
            </div>
            <div className="mcp-line">
              <span>入口</span>
              <code>{mcpInfo.command}</code>
            </div>
            <div className="mcp-line">
              <span>工具</span>
              <span>{mcpInfo.tools.length} 个</span>
            </div>
            <div className="mcp-line">
              <span>已授权连接</span>
              <span>{mcpInfo.authorizedConnections}</span>
            </div>
            <p className="mcp-note">
              {mcpInfo.status === "scaffold"
                ? "协议与工具目录已就绪；stdio 传输与真实执行将在第一阶段接入。"
                : mcpInfo.status === "running"
                  ? "MCP 服务运行中。"
                  : "stdio 传输已就绪，由 MCP 客户端按需启动 `shellx --mcp-stdio`。"}
            </p>
          </>
        ) : (
          <p className="muted">MCP 状态读取中…</p>
        )}
      </div>
    </aside>
  );
}
