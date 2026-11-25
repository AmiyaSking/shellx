import { useCallback, useEffect, useState } from "react";
import { api } from "../lib/api";
import {
  ACTION_LABELS,
  ENV_LABELS,
  RISK_LABELS,
  type AuditEntry,
} from "../lib/types";

interface Props {
  onClose: () => void;
}

export default function AuditDrawer({ onClose }: Props) {
  const [entries, setEntries] = useState<AuditEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState("");

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setEntries(await api.listAuditEntries(200));
      setError(null);
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const keyword = filter.trim().toLowerCase();
  const visible = entries.filter((entry) => {
    if (!keyword) return true;
    return [
      entry.connectionAlias ?? "",
      entry.command ?? "",
      entry.detail,
      entry.source,
    ]
      .join(" ")
      .toLowerCase()
      .includes(keyword);
  });

  return (
    <div className="drawer-backdrop" role="presentation" onClick={onClose}>
      <aside
        className="drawer"
        role="dialog"
        aria-modal="true"
        aria-label="审计日志"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="drawer-head">
          <div>
            <h2>审计日志</h2>
            <p className="muted">
              所有策略判定与审批结果都会写入本地 JSONL 文件。
            </p>
          </div>
          <div className="drawer-actions">
            <button className="btn btn-ghost btn-sm" onClick={() => void load()}>
              刷新
            </button>
            <button className="icon-btn" onClick={onClose} aria-label="关闭">
              ×
            </button>
          </div>
        </div>

        <input
          className="search"
          placeholder="筛选服务器 / 命令 / 结果…"
          value={filter}
          onChange={(event) => setFilter(event.target.value)}
        />

        <div className="audit-list">
          {loading && <div className="muted pad">读取中…</div>}
          {error && <div className="form-error">{error}</div>}
          {!loading && !error && visible.length === 0 && (
            <div className="empty-hint">
              <p>暂无审计记录。</p>
              <p>执行命令后，这里会展示策略判定与审批结果。</p>
            </div>
          )}
          {visible.map((entry) => (
            <div className="audit-entry" key={entry.id}>
              <div className="audit-line">
                <span className={`action-badge action-${entry.action}`}>
                  {ACTION_LABELS[entry.action]}
                </span>
                {entry.risk && (
                  <span className={`risk-badge risk-${entry.risk}`}>
                    {RISK_LABELS[entry.risk]}
                  </span>
                )}
                <span className={`source-badge source-${entry.source}`}>
                  {entry.source === "agent"
                    ? "Agent"
                    : entry.source === "user"
                      ? "用户"
                      : "系统"}
                </span>
                <span className="audit-time">
                  {new Date(entry.timestamp).toLocaleString()}
                </span>
              </div>
              <div className="audit-target">
                {entry.connectionAlias ?? "—"}
                {entry.environment && (
                  <span className={`badge badge-${entry.environment}`}>
                    {ENV_LABELS[entry.environment]}
                  </span>
                )}
                {entry.approval && (
                  <span className={`approval-badge approval-${entry.approval}`}>
                    {entry.approval === "approved" ? "已批准" : "已拒绝"}
                  </span>
                )}
              </div>
              {entry.command && <pre className="command-block sm">{entry.command}</pre>}
              <div className="audit-detail">
                {entry.detail}
                {entry.exitCode !== null && entry.exitCode !== undefined && (
                  <span className="muted"> · exit {entry.exitCode}</span>
                )}
                {entry.durationMs !== null && entry.durationMs !== undefined && (
                  <span className="muted"> · {entry.durationMs}ms</span>
                )}
              </div>
            </div>
          ))}
        </div>
      </aside>
    </div>
  );
}
