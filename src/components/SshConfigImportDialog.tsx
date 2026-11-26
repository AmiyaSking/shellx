import { useEffect, useMemo, useState } from "react";
import { api } from "../lib/api";
import type {
  Connection,
  ConnectionInput,
  SshConfigHost,
  SshConfigPreview,
} from "../lib/types";

interface Props {
  existingConnections: Connection[];
  onImported: (imported: number) => void | Promise<void>;
  onClose: () => void;
}

interface ImportResult {
  imported: number;
  updated: number;
  errors: string[];
}

function buildInput(
  host: SshConfigHost,
  username: string,
): ConnectionInput {
  return {
    id: null,
    alias: host.alias,
    host: host.host,
    port: host.port,
    username,
    group: "SSH Config",
    tags: ["ssh-config"],
    environment: "dev",
    auth: host.identityFile
      ? {
          kind: "private_key",
          secretRef: null,
          privateKeyPath: host.identityFile,
        }
      : { kind: "agent", secretRef: null, privateKeyPath: null },
    mcpAccess: "disabled",
    jumpHost: null,
    remark: "从 OpenSSH config 导入",
    term: "xterm-256color",
    encoding: "utf-8",
    startupCommand: "",
    legacyAlgorithms: false,
    password: null,
    passphrase: null,
  };
}

function parseJumpSpec(
  spec: string,
): { host: string; port: number; username?: string } | null {
  const first = spec.split(",")[0]?.trim();
  if (!first || first.toLowerCase() === "none") return null;

  let username: string | undefined;
  let rest = first;
  const at = first.lastIndexOf("@");
  if (at >= 0) {
    username = first.slice(0, at);
    rest = first.slice(at + 1);
  }

  let host = rest;
  let port = 22;
  if (rest.startsWith("[")) {
    const end = rest.indexOf("]");
    if (end > 0) {
      host = rest.slice(1, end);
      const tail = rest.slice(end + 1);
      if (tail.startsWith(":")) {
        const parsed = Number(tail.slice(1));
        if (Number.isInteger(parsed) && parsed > 0) port = parsed;
      }
    }
  } else {
    const colon = rest.lastIndexOf(":");
    if (colon > 0 && rest.indexOf(":") === colon) {
      const parsed = Number(rest.slice(colon + 1));
      if (Number.isInteger(parsed) && parsed > 0) {
        host = rest.slice(0, colon);
        port = parsed;
      }
    }
  }

  return host ? { host, port, username } : null;
}

function resolveJumpTarget(
  spec: string,
  imported: Map<string, Connection>,
  existing: Connection[],
): Connection | null {
  const target = parseJumpSpec(spec);
  if (!target) return null;
  const candidates = [...imported.values(), ...existing];
  return (
    candidates.find(
      (connection) =>
        connection.alias.toLowerCase() === target.host.toLowerCase(),
    ) ??
    candidates.find(
      (connection) =>
        connection.host === target.host &&
        connection.port === target.port &&
        (!target.username || connection.username === target.username),
    ) ??
    null
  );
}

export default function SshConfigImportDialog({
  existingConnections,
  onImported,
  onClose,
}: Props) {
  const [preview, setPreview] = useState<SshConfigPreview | null>(null);
  const [selected, setSelected] = useState<Set<string>>(() => new Set());
  const [loading, setLoading] = useState(true);
  const [importing, setImporting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<ImportResult | null>(null);

  const isExisting = (host: SshConfigHost) =>
    existingConnections.some(
      (connection) =>
        connection.alias.toLowerCase() === host.alias.toLowerCase() ||
        (connection.host === host.host &&
          connection.port === host.port &&
          connection.username === host.username),
    );

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const data = await api.previewSshConfig();
        if (cancelled) return;
        setPreview(data);
        setSelected(
          new Set(
            data.hosts
              .filter((host) => {
                const exists = existingConnections.some(
                  (connection) =>
                    connection.alias.toLowerCase() ===
                      host.alias.toLowerCase() ||
                    (connection.host === host.host &&
                      connection.port === host.port &&
                      connection.username === host.username),
                );
                return !exists;
              })
              .map((host) => host.alias),
          ),
        );
      } catch (err) {
        if (!cancelled) setError(String(err));
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [existingConnections]);

  const hosts = useMemo(() => preview?.hosts ?? [], [preview]);
  const existingCount = useMemo(
    () => hosts.filter((host) => isExisting(host)).length,
    [hosts, existingConnections],
  );

  const toggle = (alias: string) => {
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(alias)) next.delete(alias);
      else next.add(alias);
      return next;
    });
  };

  const importSelected = async () => {
    if (!preview) return;
    const chosen = hosts.filter((host) => selected.has(host.alias));
    if (chosen.length === 0) return;

    setImporting(true);
    setError(null);
    const savedByAlias = new Map<string, Connection>();
    const inputs = new Map<string, ConnectionInput>();
    const errors: string[] = [];
    let imported = 0;
    let updated = 0;

    for (const host of chosen) {
      const username = (host.username || preview.defaultUser).trim();
      if (!username) {
        errors.push(`${host.alias}: 缺少用户名，已跳过`);
        continue;
      }
      const input = buildInput(host, username);
      try {
        const saved = await api.saveConnection(input);
        savedByAlias.set(host.alias, saved);
        inputs.set(host.alias, input);
        imported += 1;
      } catch (err) {
        errors.push(`${host.alias}: ${String(err)}`);
      }
    }

    for (const host of chosen) {
      const saved = savedByAlias.get(host.alias);
      const input = inputs.get(host.alias);
      if (!saved || !input || !host.proxyJump) continue;
      if (host.proxyJump.includes(",")) {
        errors.push(`${host.alias}: 多级 ProxyJump 暂不支持，已按直连导入`);
        continue;
      }
      const jump = resolveJumpTarget(
        host.proxyJump,
        savedByAlias,
        existingConnections,
      );
      if (!jump) {
        errors.push(
          `${host.alias}: 未找到跳板机「${host.proxyJump}」，已按直连导入`,
        );
        continue;
      }
      try {
        const updatedConnection = await api.saveConnection({
          ...input,
          id: saved.id,
          jumpHost: jump.id,
        });
        savedByAlias.set(host.alias, updatedConnection);
        updated += 1;
      } catch (err) {
        errors.push(`${host.alias}: 设置跳板机失败：${String(err)}`);
      }
    }

    setImporting(false);
    setResult({ imported, updated, errors });
  };

  return (
    <div className="modal-backdrop" role="presentation">
      <div
        className="modal import-modal"
        role="dialog"
        aria-modal="true"
        aria-label="导入 SSH Config"
      >
        <div className="modal-head">
          <div>
            <h2>导入 OpenSSH Config</h2>
            <p className="muted">
              从 <code>{preview?.path ?? "~/.ssh/config"}</code> 读取 Host
              条目；导入后仍是普通 ShellX 连接，可继续编辑。
            </p>
          </div>
          <button className="icon-btn" onClick={onClose} aria-label="关闭">
            ×
          </button>
        </div>

        <div className="import-body">
          {loading && <div className="muted pad">读取配置中…</div>}
          {error && <div className="form-error">{error}</div>}

          {preview && !result && (
            <>
              {preview.warnings.length > 0 && (
                <div className="import-warnings">
                  {preview.warnings.map((warning) => (
                    <div key={warning}>· {warning}</div>
                  ))}
                </div>
              )}

              <div className="import-toolbar">
                <label className="check">
                  <input
                    type="checkbox"
                    checked={
                      hosts.length > 0 && selected.size === hosts.length
                    }
                    onChange={(event) => {
                      setSelected(
                        event.target.checked
                          ? new Set(hosts.map((host) => host.alias))
                          : new Set(),
                      );
                    }}
                  />
                  全选
                </label>
                <span className="muted small">
                  共 {hosts.length} 条，已存在 {existingCount} 条，已选{" "}
                  {selected.size} 条
                </span>
              </div>

              <div className="import-list">
                {hosts.length === 0 && (
                  <div className="empty-hint">
                    <p>没有找到可导入的 Host。</p>
                  </div>
                )}
                {hosts.map((host) => {
                  const exists = isExisting(host);
                  return (
                    <label
                      key={host.alias}
                      className={`import-row ${exists ? "is-existing" : ""}`}
                    >
                      <input
                        type="checkbox"
                        checked={selected.has(host.alias)}
                        onChange={() => toggle(host.alias)}
                      />
                      <div className="import-main">
                        <div className="import-line">
                          <strong>{host.alias}</strong>
                          {exists && (
                            <span className="mini-badge">已存在</span>
                          )}
                          <span className="mini-badge">
                            {host.identityFile ? "私钥" : "Agent"}
                          </span>
                          {host.proxyJump && (
                            <span className="mini-badge">
                              跳板 {host.proxyJump}
                            </span>
                          )}
                        </div>
                        <div className="import-sub">
                          {(host.username || preview.defaultUser || "缺少用户名") +
                            "@" +
                            host.host +
                            ":" +
                            host.port}
                        </div>
                        {host.identityFile && (
                          <div className="muted small">
                            {host.identityFile}
                          </div>
                        )}
                      </div>
                    </label>
                  );
                })}
              </div>
            </>
          )}

          {result && (
            <div className="import-result">
              <div className="import-result-head">
                已导入 {result.imported} 条
                {result.updated > 0 && `，设置跳板机 ${result.updated} 条`}
              </div>
              {result.errors.length > 0 && (
                <div className="form-error">
                  {result.errors.map((message) => (
                    <div key={message}>{message}</div>
                  ))}
                </div>
              )}
            </div>
          )}
        </div>

        <div className="modal-foot">
          <div className="muted small">
            认证方式由 IdentityFile 推断，缺少时使用本机 SSH Agent。
          </div>
          <div className="modal-foot-right">
            {result ? (
              <button
                type="button"
                className="btn btn-primary"
                onClick={() => void onImported(result.imported)}
              >
                完成
              </button>
            ) : (
              <>
                <button type="button" className="btn btn-ghost" onClick={onClose}>
                  取消
                </button>
                <button
                  type="button"
                  className="btn btn-primary"
                  disabled={
                    importing || loading || selected.size === 0 || !preview
                  }
                  onClick={() => void importSelected()}
                >
                  {importing ? "导入中…" : `导入 ${selected.size} 条`}
                </button>
              </>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
