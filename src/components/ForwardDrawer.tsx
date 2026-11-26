import { useCallback, useEffect, useMemo, useState } from "react";
import { api } from "../lib/api";
import type {
  Connection,
  ForwardKind,
  ForwardRule,
  ForwardRuleInput,
  ForwardStatus,
} from "../lib/types";

interface Props {
  connections: Connection[];
  onClose: () => void;
}

interface FormState {
  connectionId: string;
  kind: ForwardKind;
  bindHost: string;
  bindPort: string;
  targetHost: string;
  targetPort: string;
  remark: string;
}

function defaultForm(connectionId: string): FormState {
  return {
    connectionId,
    kind: "local",
    bindHost: "127.0.0.1",
    bindPort: "8080",
    targetHost: "127.0.0.1",
    targetPort: "80",
    remark: "",
  };
}

export default function ForwardDrawer({ connections, onClose }: Props) {
  const [rules, setRules] = useState<ForwardRule[]>([]);
  const [statuses, setStatuses] = useState<Map<string, ForwardStatus>>(
    () => new Map(),
  );
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);

  const [formOpen, setFormOpen] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [form, setForm] = useState<FormState>(() =>
    defaultForm(connections[0]?.id ?? ""),
  );
  const [formError, setFormError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const connectionById = useMemo(
    () => new Map(connections.map((connection) => [connection.id, connection])),
    [connections],
  );

  const refresh = useCallback(async () => {
    try {
      const [nextRules, nextStatuses] = await Promise.all([
        api.listForwardRules(),
        api.listForwardStatuses(),
      ]);
      setRules(nextRules);
      setStatuses(
        new Map(nextStatuses.map((status) => [status.ruleId, status])),
      );
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
    const timer = window.setInterval(() => {
      void refresh();
    }, 4000);
    return () => window.clearInterval(timer);
  }, [refresh]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !formOpen) onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [formOpen, onClose]);

  const openNew = () => {
    setEditingId(null);
    setForm(defaultForm(connections[0]?.id ?? ""));
    setFormError(null);
    setFormOpen(true);
  };

  const openEdit = (rule: ForwardRule) => {
    setEditingId(rule.id);
    setForm({
      connectionId: rule.connectionId,
      kind: rule.kind,
      bindHost: rule.bindHost,
      bindPort: String(rule.bindPort),
      targetHost: rule.targetHost,
      targetPort: String(rule.targetPort || 80),
      remark: rule.remark,
    });
    setFormError(null);
    setFormOpen(true);
  };

  const save = async () => {
    const bindPort = Number(form.bindPort);
    const targetPort = Number(form.targetPort);
    if (!form.connectionId) {
      setFormError("请选择连接。");
      return;
    }
    if (!form.bindHost.trim()) {
      setFormError("监听地址不能为空。");
      return;
    }
    if (!Number.isInteger(bindPort) || bindPort < 0 || bindPort > 65535) {
      setFormError("监听端口必须是 0-65535 之间的整数。");
      return;
    }
    if (form.kind !== "dynamic") {
      if (!form.targetHost.trim()) {
        setFormError("该转发类型必须填写目标主机。");
        return;
      }
      if (!Number.isInteger(targetPort) || targetPort < 1 || targetPort > 65535) {
        setFormError("目标端口必须是 1-65535 之间的整数。");
        return;
      }
    }

    const input: ForwardRuleInput = {
      id: editingId,
      connectionId: form.connectionId,
      kind: form.kind,
      bindHost: form.bindHost.trim(),
      bindPort,
      targetHost: form.kind !== "dynamic" ? form.targetHost.trim() : "",
      targetPort: form.kind !== "dynamic" ? targetPort : 0,
      remark: form.remark.trim(),
    };

    setSaving(true);
    setFormError(null);
    try {
      await api.saveForwardRule(input);
      setFormOpen(false);
      setEditingId(null);
      await refresh();
    } catch (err) {
      setFormError(String(err));
    } finally {
      setSaving(false);
    }
  };

  const start = async (rule: ForwardRule) => {
    setBusyId(rule.id);
    setError(null);
    try {
      const status = await api.startForward(rule.id);
      setStatuses((current) => new Map(current).set(rule.id, status));
      await refresh();
    } catch (err) {
      setError(String(err));
      await refresh();
    } finally {
      setBusyId(null);
    }
  };

  const stop = async (rule: ForwardRule) => {
    setBusyId(rule.id);
    setError(null);
    try {
      await api.stopForward(rule.id);
      await refresh();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusyId(null);
    }
  };

  const remove = async (rule: ForwardRule) => {
    if (!window.confirm(`删除转发规则「${rule.remark || rule.id}」？`)) return;
    setBusyId(rule.id);
    setError(null);
    try {
      await api.deleteForwardRule(rule.id);
      await refresh();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusyId(null);
    }
  };

  const set = <K extends keyof FormState,>(key: K, value: FormState[K]) => {
    setForm((current) => ({ ...current, [key]: value }));
  };

  return (
    <div className="drawer-backdrop" role="presentation" onClick={onClose}>
      <aside
        className="drawer forward-drawer"
        role="dialog"
        aria-modal="true"
        aria-label="端口转发"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="drawer-head">
          <div>
            <h2>端口转发</h2>
            <p className="muted">
              本地转发（-L）、远程转发（-R）与 SOCKS5 动态转发（-D）。主机密钥需先在终端中确认。
            </p>
          </div>
          <div className="drawer-actions">
            <button className="btn btn-primary btn-sm" onClick={openNew}>
              + 新建
            </button>
            <button className="icon-btn" onClick={onClose} aria-label="关闭">
              ×
            </button>
          </div>
        </div>

        {error && <div className="form-error">{error}</div>}

        {formOpen && (
          <div className="forward-form">
            <div className="forward-form-title">
              {editingId ? "编辑转发规则" : "新建转发规则"}
            </div>
            <div className="forward-form-grid">
              <label>
                <span>连接</span>
                <select
                  value={form.connectionId}
                  onChange={(event) => set("connectionId", event.target.value)}
                >
                  {connections.length === 0 && (
                    <option value="">请先创建连接</option>
                  )}
                  {connections.map((connection) => (
                    <option key={connection.id} value={connection.id}>
                      {connection.alias}（{connection.host}）
                    </option>
                  ))}
                </select>
              </label>
              <label>
                <span>类型</span>
                <select
                  value={form.kind}
                  onChange={(event) => {
                    const kind = event.target.value as ForwardKind;
                    set("kind", kind);
                    set("bindPort", kind === "dynamic" ? "1080" : "8080");
                  }}
                >
                  <option value="local">本地转发 -L</option>
                  <option value="remote">远程转发 -R</option>
                  <option value="dynamic">SOCKS5 动态 -D</option>
                </select>
              </label>
              <label>
                <span>
                  {form.kind === "remote" ? "远端监听地址" : "监听地址"}
                </span>
                <input
                  value={form.bindHost}
                  onChange={(event) => set("bindHost", event.target.value)}
                  placeholder="127.0.0.1"
                />
              </label>
              <label>
                <span>
                  {form.kind === "remote"
                    ? "远端监听端口（0 = 随机）"
                    : "监听端口（0 = 随机）"}
                </span>
                <input
                  value={form.bindPort}
                  onChange={(event) => set("bindPort", event.target.value)}
                  inputMode="numeric"
                />
              </label>
              {form.kind !== "dynamic" && (
                <>
                  <label>
                    <span>
                      {form.kind === "remote"
                        ? "本地目标主机"
                        : "目标主机"}
                    </span>
                    <input
                      value={form.targetHost}
                      onChange={(event) =>
                        set("targetHost", event.target.value)
                      }
                      placeholder="127.0.0.1 或内网主机"
                    />
                  </label>
                  <label>
                    <span>
                      {form.kind === "remote"
                        ? "本地目标端口"
                        : "目标端口"}
                    </span>
                    <input
                      value={form.targetPort}
                      onChange={(event) =>
                        set("targetPort", event.target.value)
                      }
                      inputMode="numeric"
                    />
                  </label>
                </>
              )}
              <label className="span-2">
                <span>备注</span>
                <input
                  value={form.remark}
                  onChange={(event) => set("remark", event.target.value)}
                  placeholder="用途说明"
                />
              </label>
            </div>
            <p className="muted small">
              {form.kind === "local"
                ? "访问本机监听端口时，流量会经 SSH 转发到目标主机。"
                : form.kind === "remote"
                  ? "由远端服务器监听端口，连接会经 SSH 转发到本机目标。"
                  : "将本机作为 SOCKS5 代理（无认证），由远端解析目标地址。"}
            </p>
            {formError && <div className="form-error">{formError}</div>}
            <div className="forward-form-actions">
              <button
                className="btn btn-ghost"
                onClick={() => setFormOpen(false)}
              >
                取消
              </button>
              <button
                className="btn btn-primary"
                disabled={saving}
                onClick={() => void save()}
              >
                {saving ? "保存中…" : "保存"}
              </button>
            </div>
          </div>
        )}

        <div className="forward-list">
          {loading && <div className="muted pad">加载中…</div>}
          {!loading && rules.length === 0 && !formOpen && (
            <div className="empty-hint">
              <p>还没有端口转发规则。</p>
              <p>新建后可手动启动或停止；规则保存在本机。</p>
            </div>
          )}

          {rules.map((rule) => {
            const status = statuses.get(rule.id);
            const connection = connectionById.get(rule.connectionId);
            const running = status?.running ?? false;
            const address = `${rule.bindHost}:${
              status?.boundPort ?? rule.bindPort
            }`;
            const target = connection
              ? connection.alias
              : "连接已删除";
            return (
              <div
                key={rule.id}
                className={`forward-card ${running ? "is-running" : ""}`}
              >
                <div className="forward-card-head">
                  <span
                    className={`forward-status ${
                      status?.error
                        ? "is-error"
                        : running
                          ? "is-running"
                          : ""
                    }`}
                  />
                  <span className="mini-badge">
                    {rule.kind === "local"
                      ? "-L 本地"
                      : rule.kind === "remote"
                        ? "-R 远程"
                        : "-D SOCKS5"}
                  </span>
                  <strong>{rule.remark || address}</strong>
                  <span className="muted small">{target}</span>
                </div>
                <div className="forward-route">
                  <code>{address}</code>
                  <span>→</span>
                  <code>
                    {rule.kind !== "dynamic"
                      ? `${rule.targetHost}:${rule.targetPort}`
                      : "动态目标（SOCKS5）"}
                  </code>
                </div>
                {status?.error && (
                  <div className="forward-error">{status.error}</div>
                )}
                <div className="forward-actions">
                  {!running ? (
                    <button
                      className="btn btn-primary btn-sm"
                      disabled={busyId === rule.id}
                      onClick={() => void start(rule)}
                    >
                      {busyId === rule.id ? "启动中…" : "启动"}
                    </button>
                  ) : (
                    <button
                      className="btn btn-ghost btn-sm"
                      disabled={busyId === rule.id}
                      onClick={() => void stop(rule)}
                    >
                      停止
                    </button>
                  )}
                  <button
                    className="btn btn-ghost btn-sm"
                    onClick={() => openEdit(rule)}
                  >
                    编辑
                  </button>
                  <button
                    className="btn btn-danger btn-sm"
                    disabled={busyId === rule.id}
                    onClick={() => void remove(rule)}
                  >
                    删除
                  </button>
                </div>
              </div>
            );
          })}
        </div>

        <p className="muted small">
          每个运行中的规则使用独立 SSH 连接；停止规则会立即中断活动隧道。
        </p>
      </aside>
    </div>
  );
}
