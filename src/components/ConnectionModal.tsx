import { useMemo, useState, type FormEvent } from "react";
import {
  ENV_LABELS,
  MCP_ACCESS_LABELS,
  type AuthKind,
  type Connection,
  type ConnectionInput,
  type Environment,
  type McpAccessMode,
} from "../lib/types";

interface Props {
  mode: "create" | "edit";
  connection: Connection | null;
  connections: Connection[];
  onSave: (input: ConnectionInput) => Promise<void>;
  onDelete?: () => void;
  onClose: () => void;
}

interface FormState {
  alias: string;
  host: string;
  port: string;
  username: string;
  group: string;
  tags: string;
  environment: Environment;
  authKind: AuthKind;
  privateKeyPath: string;
  password: string;
  mcpAccess: McpAccessMode;
  jumpHost: string;
  remark: string;
}

function initialForm(connection: Connection | null): FormState {
  return {
    alias: connection?.alias ?? "",
    host: connection?.host ?? "",
    port: String(connection?.port ?? 22),
    username: connection?.username ?? "",
    group: connection?.group ?? "",
    tags: connection?.tags.join(", ") ?? "",
    environment: connection?.environment ?? "dev",
    authKind: connection?.auth.kind ?? "agent",
    privateKeyPath: connection?.auth.privateKeyPath ?? "",
    password: "",
    mcpAccess: connection?.mcpAccess ?? "disabled",
    jumpHost: connection?.jumpHost ?? "",
    remark: connection?.remark ?? "",
  };
}

export default function ConnectionModal({
  mode,
  connection,
  connections,
  onSave,
  onDelete,
  onClose,
}: Props) {
  const [form, setForm] = useState<FormState>(() => initialForm(connection));
  const [saving, setSaving] = useState(false);
  const [localError, setLocalError] = useState<string | null>(null);

  const groups = useMemo(
    () => [...new Set(connections.map((item) => item.group).filter(Boolean))],
    [connections],
  );
  const jumpOptions = useMemo(
    () => connections.filter((item) => item.id !== connection?.id),
    [connections, connection?.id],
  );

  const set = <K extends keyof FormState,>(key: K, value: FormState[K]) => {
    setForm((current) => ({ ...current, [key]: value }));
  };

  const handleSubmit = async (event: FormEvent) => {
    event.preventDefault();
    const port = Number(form.port);
    if (!form.alias.trim() || !form.host.trim() || !form.username.trim()) {
      setLocalError("别名、主机地址与用户名不能为空。");
      return;
    }
    if (!Number.isInteger(port) || port < 1 || port > 65535) {
      setLocalError("端口必须是 1-65535 之间的整数。");
      return;
    }
    setLocalError(null);
    setSaving(true);
    try {
      await onSave({
        id: connection?.id ?? null,
        alias: form.alias.trim(),
        host: form.host.trim(),
        port,
        username: form.username.trim(),
        group: form.group.trim(),
        tags: form.tags
          .split(",")
          .map((tag) => tag.trim())
          .filter(Boolean),
        environment: form.environment,
        auth: {
          kind: form.authKind,
          secretRef: connection?.auth.secretRef ?? null,
          privateKeyPath: form.privateKeyPath.trim() || null,
        },
        mcpAccess: form.mcpAccess,
        jumpHost: form.jumpHost || null,
        remark: form.remark.trim(),
        password: form.password || null,
      });
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="modal-backdrop" role="presentation">
      <div className="modal" role="dialog" aria-modal="true" aria-label="连接配置">
        <div className="modal-head">
          <div>
            <h2>{mode === "create" ? "新建连接" : "编辑连接"}</h2>
            <p className="muted">凭据不会写入连接配置文件；脚手架阶段仅保存在内存中。</p>
          </div>
          <button className="icon-btn" onClick={onClose} aria-label="关闭">
            ×
          </button>
        </div>

        <form className="modal-body" onSubmit={handleSubmit}>
          <div className="form-grid">
            <label>
              <span>别名 *</span>
              <input
                value={form.alias}
                onChange={(event) => set("alias", event.target.value)}
                placeholder="prod-web-01"
                autoFocus
              />
            </label>
            <label>
              <span>分组</span>
              <input
                value={form.group}
                onChange={(event) => set("group", event.target.value)}
                placeholder="生产环境"
                list="group-options"
              />
              <datalist id="group-options">
                {groups.map((group) => (
                  <option key={group} value={group} />
                ))}
              </datalist>
            </label>
            <label>
              <span>主机地址 *</span>
              <input
                value={form.host}
                onChange={(event) => set("host", event.target.value)}
                placeholder="10.30.0.8"
              />
            </label>
            <label>
              <span>端口 *</span>
              <input
                value={form.port}
                onChange={(event) => set("port", event.target.value)}
                inputMode="numeric"
              />
            </label>
            <label>
              <span>用户名 *</span>
              <input
                value={form.username}
                onChange={(event) => set("username", event.target.value)}
                placeholder="ops"
              />
            </label>
            <label>
              <span>环境分级</span>
              <select
                value={form.environment}
                onChange={(event) => set("environment", event.target.value as Environment)}
              >
                {(Object.keys(ENV_LABELS) as Environment[]).map((environment) => (
                  <option key={environment} value={environment}>
                    {ENV_LABELS[environment]}（{environment}）
                  </option>
                ))}
              </select>
            </label>
            <label>
              <span>认证方式</span>
              <select
                value={form.authKind}
                onChange={(event) => set("authKind", event.target.value as AuthKind)}
              >
                <option value="agent">SSH Agent</option>
                <option value="private_key">私钥</option>
                <option value="password">密码</option>
              </select>
            </label>
            <label>
              <span>MCP 授权</span>
              <select
                value={form.mcpAccess}
                onChange={(event) => set("mcpAccess", event.target.value as McpAccessMode)}
              >
                {(Object.keys(MCP_ACCESS_LABELS) as McpAccessMode[]).map((access) => (
                  <option key={access} value={access}>
                    {MCP_ACCESS_LABELS[access]}（{access}）
                  </option>
                ))}
              </select>
            </label>

            {form.authKind === "private_key" && (
              <label>
                <span>私钥路径</span>
                <input
                  value={form.privateKeyPath}
                  onChange={(event) => set("privateKeyPath", event.target.value)}
                  placeholder="~/.ssh/id_ed25519"
                />
              </label>
            )}
            {form.authKind === "password" && (
              <label>
                <span>登录密码</span>
                <input
                  type="password"
                  value={form.password}
                  onChange={(event) => set("password", event.target.value)}
                  placeholder="仅保存在内存（脚手架）"
                />
              </label>
            )}
            <label>
              <span>跳板机</span>
              <select
                value={form.jumpHost}
                onChange={(event) => set("jumpHost", event.target.value)}
              >
                <option value="">无</option>
                {jumpOptions.map((item) => (
                  <option key={item.id} value={item.id}>
                    {item.alias}（{item.host}）
                  </option>
                ))}
              </select>
            </label>
            <label className="span-2">
              <span>标签（逗号分隔）</span>
              <input
                value={form.tags}
                onChange={(event) => set("tags", event.target.value)}
                placeholder="nginx, critical"
              />
            </label>
            <label className="span-2">
              <span>备注</span>
              <textarea
                value={form.remark}
                onChange={(event) => set("remark", event.target.value)}
                rows={2}
                placeholder="用途、联系人、注意事项…"
              />
            </label>
          </div>

          {localError && <div className="form-error">{localError}</div>}

          <div className="modal-foot">
            <div>
              {onDelete && (
                <button type="button" className="btn btn-danger btn-sm" onClick={onDelete}>
                  删除连接
                </button>
              )}
            </div>
            <div className="modal-foot-right">
              <button type="button" className="btn btn-ghost" onClick={onClose}>
                取消
              </button>
              <button type="submit" className="btn btn-primary" disabled={saving}>
                {saving ? "保存中…" : mode === "create" ? "创建连接" : "保存修改"}
              </button>
            </div>
          </div>
        </form>
      </div>
    </div>
  );
}
