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
  passphrase: string;
  mcpAccess: McpAccessMode;
  mcpPtyEnabled: boolean;
  jumpHost: string;
  remark: string;
  term: string;
  encoding: string;
  startupCommand: string;
  legacyAlgorithms: boolean;
  forwardAgent: boolean;
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
    passphrase: "",
    mcpAccess: connection?.mcpAccess ?? "disabled",
    mcpPtyEnabled: connection?.mcpPtyEnabled ?? false,
    jumpHost: connection?.jumpHost ?? "",
    remark: connection?.remark ?? "",
    term: connection?.term ?? "xterm-256color",
    encoding: connection?.encoding ?? "utf-8",
    startupCommand: connection?.startupCommand ?? "",
    legacyAlgorithms: connection?.legacyAlgorithms ?? false,
    forwardAgent: connection?.forwardAgent ?? false,
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

  const toggleMcpPty = (enabled: boolean) => {
    if (!enabled) {
      set("mcpPtyEnabled", false);
      return;
    }
    const confirmed = window.confirm(
      "危险：开启后 MCP Agent 可以获得完整的交互式 shell，绕过命令级策略、风险分级与审批。\n\n仅在完全信任接入的 Agent 时开启。确定开启吗？",
    );
    if (confirmed) set("mcpPtyEnabled", true);
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
        mcpPtyEnabled:
          form.mcpAccess === "standard_controlled" && form.mcpPtyEnabled,
        jumpHost: form.jumpHost || null,
        remark: form.remark.trim(),
        term: form.term.trim() || "xterm-256color",
        encoding: form.encoding,
        startupCommand: form.startupCommand.trim(),
        legacyAlgorithms: form.legacyAlgorithms,
        forwardAgent: form.forwardAgent,
        password: form.password || null,
        passphrase: form.passphrase || null,
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
            <p className="muted">
              凭据写入系统凭据存储（Windows Credential Manager / macOS Keychain），
              不会出现在 connections.json 中。
            </p>
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

            <p className="muted small span-2">
              「只读」允许 MCP 读取命令与 SFTP 文件；「受控」在 dev 环境还允许
              SFTP 写入/删除，staging/prod 的写操作需要人工审批（stdio 模式会自动拒绝）。
              这里是快捷预设；更细的可见范围、访问级别、确认阈值与工具开关请在侧边栏
              「MCP 权限设置」中配置。
            </p>

            <label className="span-2 checkbox-field">
              <input
                type="checkbox"
                checked={
                  form.mcpAccess === "standard_controlled" &&
                  form.mcpPtyEnabled
                }
                disabled={form.mcpAccess !== "standard_controlled"}
                onChange={(event) => toggleMcpPty(event.target.checked)}
              />
              <span>
                允许 MCP 交互式终端（危险，默认关闭；需将 MCP 授权设为「受控」）。开启后
                Agent 可获得完整 shell，绕过命令级策略与审批。
              </span>
            </label>
            {form.mcpPtyEnabled && (
              <p className="muted small span-2">
                ⚠ 已开启：任何获得该连接 MCP 权限的 Agent
                都可以执行任意交互命令，请仅在完全信任时使用。
              </p>
            )}

            {form.authKind === "agent" && (
              <p className="muted small span-2">
                Agent 模式使用本机 OpenSSH Agent 或 Pageant；若未运行，请改用密码或私钥。
              </p>
            )}
            {form.authKind === "private_key" && (
              <>
                <label>
                  <span>私钥路径</span>
                  <input
                    value={form.privateKeyPath}
                    onChange={(event) => set("privateKeyPath", event.target.value)}
                    placeholder="~/.ssh/id_ed25519"
                  />
                </label>
                <label>
                  <span>私钥口令（可选）</span>
                  <input
                    type="password"
                    value={form.passphrase}
                    onChange={(event) => set("passphrase", event.target.value)}
                    placeholder="留空表示私钥未加密或沿用已保存口令"
                  />
                </label>
              </>
            )}
            {form.authKind === "password" && (
              <label>
                <span>登录密码</span>
                <input
                  type="password"
                  value={form.password}
                  onChange={(event) => set("password", event.target.value)}
                  placeholder="保存到系统凭据存储"
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
            <label>
              <span>终端类型（TERM）</span>
              <input
                value={form.term}
                onChange={(event) => set("term", event.target.value)}
                list="term-options"
                placeholder="xterm-256color"
              />
              <datalist id="term-options">
                <option value="xterm-256color" />
                <option value="xterm" />
                <option value="screen-256color" />
                <option value="vt100" />
                <option value="ansi" />
              </datalist>
            </label>
            <label>
              <span>字符编码</span>
              <select
                value={form.encoding}
                onChange={(event) => set("encoding", event.target.value)}
              >
                <option value="utf-8">UTF-8</option>
                <option value="gbk">GBK / GB2312</option>
                <option value="gb18030">GB18030</option>
                <option value="big5">Big5</option>
              </select>
            </label>
            <label className="span-2">
              <span>登录后启动命令</span>
              <input
                value={form.startupCommand}
                onChange={(event) => set("startupCommand", event.target.value)}
                placeholder="例如 cd /srv/app（连接后自动执行，留空则不执行）"
              />
            </label>
            <label className="span-2 checkbox-field">
              <input
                type="checkbox"
                checked={form.legacyAlgorithms}
                onChange={(event) =>
                  set("legacyAlgorithms", event.target.checked)
                }
              />
              <span>
                兼容旧算法（ssh-rsa、SHA-1 KEX/MAC、CBC/3DES）。仅用于老旧设备，会降低连接安全性。
              </span>
            </label>
            <label className="span-2 checkbox-field">
              <input
                type="checkbox"
                checked={form.forwardAgent}
                onChange={(event) => set("forwardAgent", event.target.checked)}
              />
              <span>
                转发 SSH Agent（OpenSSH Agent / SSH_AUTH_SOCK）。远端可在会话期间使用本机密钥，请谨慎开启。
              </span>
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
