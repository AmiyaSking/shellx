import { useCallback, useEffect, useMemo, useState } from "react";
import { api } from "../lib/api";
import {
  ENV_LABELS,
  RISK_LABELS,
  type McpAccessLevel,
  type McpAccessSource,
  type McpConfirmLevel,
  type McpConnectionAccess,
  type McpPolicy,
  type McpPolicyRule,
  type McpPolicySnapshot,
  type McpRuleTarget,
  type McpToolState,
} from "../lib/types";
import DrawerShell from "./shell/DrawerShell";

interface Props {
  open: boolean;
  onClose: () => void;
  onSaved?: () => void;
  onOpenAudit?: (filter: string) => void;
}

const ACCESS_LEVELS: McpAccessLevel[] = ["none", "read", "write", "full"];
const CONFIRM_LEVELS: McpConfirmLevel[] = ["off", "medium", "high"];

const ACCESS_LABELS: Record<McpAccessLevel, string> = {
  none: "不可访问",
  read: "只读",
  write: "读写",
  full: "完全访问",
};

const CONFIRM_LABELS: Record<McpConfirmLevel, string> = {
  off: "关闭",
  medium: "中风险起",
  high: "高风险起",
};

const SOURCE_LABELS: Record<McpAccessSource, string> = {
  global: "全局",
  group: "分组",
  connection: "连接",
};

const RANK: Record<McpAccessLevel, number> = {
  none: 0,
  read: 1,
  write: 2,
  full: 3,
};

function minAccess(left: McpAccessLevel, right: McpAccessLevel): McpAccessLevel {
  return RANK[left] <= RANK[right] ? left : right;
}

function targetKey(target: McpRuleTarget): string {
  return target.kind === "group"
    ? `group:${target.name}`
    : `connection:${target.id}`;
}

function ruleFor(policy: McpPolicy, target: McpRuleTarget): McpPolicyRule | undefined {
  return policy.rules.find(
    (rule) => targetKey(rule.target) === targetKey(target),
  );
}

function setRuleField(
  policy: McpPolicy,
  target: McpRuleTarget,
  field: "visible" | "access" | "confirm",
  value: boolean | McpAccessLevel | McpConfirmLevel | null,
): McpPolicy {
  const key = targetKey(target);
  let found = false;
  const rules = policy.rules.map((rule) => {
    if (targetKey(rule.target) !== key) return rule;
    found = true;
    return { ...rule, [field]: value };
  });
  if (!found && value !== null) {
    rules.push({ target, [field]: value });
  }
  return {
    ...policy,
    rules: rules.filter(
      (rule) =>
        rule.visible != null || rule.access != null || rule.confirm != null,
    ),
  };
}

function setCapability(
  policy: McpPolicy,
  connectionId: string,
  field: "sftp" | "forwarding",
  value: McpAccessLevel | null,
): McpPolicy {
  const current = { ...(policy.capabilities[connectionId] ?? {}) };
  if (value === null) {
    delete current[field];
  } else {
    current[field] = value;
  }
  const capabilities = { ...policy.capabilities };
  if (current.sftp == null && current.forwarding == null) {
    delete capabilities[connectionId];
  } else {
    capabilities[connectionId] = current;
  }
  return { ...policy, capabilities };
}

function setTool(policy: McpPolicy, name: string, enabled: boolean): McpPolicy {
  return { ...policy, tools: { ...policy.tools, [name]: enabled } };
}

interface EffectiveView {
  access: McpAccessLevel;
  confirm: McpConfirmLevel;
  visible: boolean;
  source: McpAccessSource;
  confirmSource: McpAccessSource;
  sftp: McpAccessLevel;
  forwarding: McpAccessLevel;
}

function effectiveFor(
  policy: McpPolicy,
  connection: McpConnectionAccess,
): EffectiveView {
  let access = policy.defaults.access;
  let confirm = policy.defaults.confirm;
  let source: McpAccessSource = "global";
  let confirmSource: McpAccessSource = "global";
  let visibleOverride: boolean | null = null;

  const groupRule = ruleFor(policy, {
    kind: "group",
    name: connection.group.trim(),
  });
  if (groupRule) {
    if (groupRule.access) {
      access = groupRule.access;
      source = "group";
    }
    if (groupRule.confirm) {
      confirm = groupRule.confirm;
      confirmSource = "group";
    }
    if (groupRule.visible != null) visibleOverride = groupRule.visible;
  }

  const connectionRule = ruleFor(policy, {
    kind: "connection",
    id: connection.connectionId,
  });
  if (connectionRule) {
    if (connectionRule.access) {
      access = connectionRule.access;
      source = "connection";
    }
    if (connectionRule.confirm) {
      confirm = connectionRule.confirm;
      confirmSource = "connection";
    }
    if (connectionRule.visible != null) visibleOverride = connectionRule.visible;
  }

  const visible =
    (policy.visibility === "all"
      ? (visibleOverride ?? true)
      : (visibleOverride ?? false)) && access !== "none";
  const caps = policy.capabilities[connection.connectionId] ?? {};
  return {
    access,
    confirm,
    visible,
    source,
    confirmSource,
    sftp: minAccess(access, caps.sftp ?? "full"),
    forwarding: minAccess(access, caps.forwarding ?? "full"),
  };
}

const TOOL_GROUPS: Array<{ title: string; match: (name: string) => boolean }> = [
  {
    title: "命令与会话",
    match: (name) =>
      ["list_servers", "exec_command", "read_output", "send_signal", "close_session"].includes(
        name,
      ),
  },
  { title: "SFTP 文件", match: (name) => name.startsWith("sftp_") },
  { title: "端口转发", match: (name) => name.endsWith("_forward") || name === "list_forward_rules" },
  { title: "交互式终端", match: (name) => name.startsWith("terminal_") },
];

export default function McpPolicyDrawer({
  open,
  onClose,
  onSaved,
  onOpenAudit,
}: Props) {
  const [snapshot, setSnapshot] = useState<McpPolicySnapshot | null>(null);
  const [draft, setDraft] = useState<McpPolicy | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [exceptionsOnly, setExceptionsOnly] = useState(false);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const next = await api.getMcpPolicy();
      setSnapshot(next);
      setDraft(next.policy);
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

  const toggleExpanded = (id: string) => {
    setExpanded((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const keyword = query.trim().toLowerCase();
  const groups = useMemo(() => {
    if (!snapshot) return [];
    const map = new Map<string, McpConnectionAccess[]>();
    for (const connection of snapshot.connections) {
      const key = connection.group || "未分组";
      const list = map.get(key) ?? [];
      list.push(connection);
      map.set(key, list);
    }
    return [...map.entries()].sort(([left], [right]) =>
      left.localeCompare(right, "zh-CN"),
    );
  }, [snapshot]);

  const matches = (connection: McpConnectionAccess) => {
    if (keyword) {
      const haystack = `${connection.alias} ${connection.group} ${connection.environment}`.toLowerCase();
      if (!haystack.includes(keyword)) return false;
    }
    if (exceptionsOnly) {
      const hasRule = Boolean(
        ruleFor(draft ?? snapshot!.policy, {
          kind: "connection",
          id: connection.connectionId,
        }),
      );
      const hasCaps = Boolean(
        (draft ?? snapshot!.policy).capabilities[connection.connectionId],
      );
      return hasRule || hasCaps;
    }
    return true;
  };

  const enabledTools = (tools: McpToolState[]) =>
    tools.filter((tool) => (draft ?? snapshot!.policy).tools[tool.name] ?? true)
      .length;

  if (loading || !snapshot || !draft) {
    return (
      <DrawerShell
        open={open}
        onClose={onClose}
        label="MCP 权限"
        width="min(1120px,100vw)"
      >
        <div className="drawer-head">
          <h2>MCP 权限</h2>
          <button className="icon-btn" onClick={onClose} aria-label="关闭">
            ×
          </button>
        </div>
        <p className="muted">{error ?? "读取权限策略中…"}</p>
      </DrawerShell>
    );
  }

  const visibleCount = snapshot.connections.filter(
    (connection) => effectiveFor(draft, connection).visible,
  ).length;

  const update = (mutator: (policy: McpPolicy) => McpPolicy) => {
    setDraft((current) => (current ? mutator(current) : current));
    setNotice(null);
  };

  const save = async () => {
    if (saving || !draft) return;
    const pending = draft;
    setSaving(true);
    setError(null);
    try {
      const next = await api.saveMcpPolicy(pending);
      setSnapshot(next);
      setDraft((current) => (current === pending ? next.policy : current));
      setNotice("权限策略已保存，MCP 进程会在下次调用时生效。");
      onSaved?.();
    } catch (err) {
      setError(String(err));
    } finally {
      setSaving(false);
    }
  };

  const renderLevelSelect = (
    value: McpAccessLevel | null | undefined,
    onChange: (value: McpAccessLevel | null) => void,
  ) => (
    <select
      className="mcp-policy-select"
      value={value ?? ""}
      onChange={(event) =>
        onChange(
          event.target.value ? (event.target.value as McpAccessLevel) : null,
        )
      }
    >
      <option value="">继承</option>
      {ACCESS_LEVELS.map((level) => (
        <option key={level} value={level}>
          {ACCESS_LABELS[level]}
        </option>
      ))}
    </select>
  );

  const renderConfirmSelect = (
    value: McpConfirmLevel | null | undefined,
    onChange: (value: McpConfirmLevel | null) => void,
  ) => (
    <select
      className="mcp-policy-select"
      value={value ?? ""}
      onChange={(event) =>
        onChange(
          event.target.value ? (event.target.value as McpConfirmLevel) : null,
        )
      }
    >
      <option value="">继承</option>
      {CONFIRM_LEVELS.map((level) => (
        <option key={level} value={level}>
          {CONFIRM_LABELS[level]}
        </option>
      ))}
    </select>
  );

  return (
    <DrawerShell
      open={open}
      onClose={onClose}
      label="MCP 权限"
      width="min(1120px,100vw)"
    >
        <div className="drawer-head">
          <div>
            <h2>MCP 权限</h2>
            <p className="muted">
              配置可见连接、访问级别、确认阈值与工具开关。命令级策略、SFTP、端口转发和交互终端共用这份策略。
            </p>
          </div>
          <button className="icon-btn" onClick={onClose} aria-label="关闭">
            ×
          </button>
        </div>

        <div className="mcp-policy-summary">
          <span>{visibleCount} / {snapshot.connections.length} 个连接可见</span>
          <span>默认 {ACCESS_LABELS[draft.defaults.access]}</span>
          <span>确认 {CONFIRM_LABELS[draft.defaults.confirm]}</span>
          <span>
            工具 {enabledTools(snapshot.tools)} / {snapshot.tools.length}
          </span>
        </div>

        <div className="mcp-policy-body">
          <section className="mcp-policy-block">
            <div className="mcp-policy-block-head">
              <div>
                <h3>全局默认</h3>
                <p className="muted small">
                  没有单独设置的连接继承这里的访问级别和确认阈值。
                </p>
              </div>
            </div>
            <div className="mcp-policy-defaults">
              <label>
                <span>访问级别</span>
                <select
                  className="mcp-policy-select"
                  value={draft.defaults.access}
                  onChange={(event) =>
                    update((policy) => ({
                      ...policy,
                      defaults: {
                        ...policy.defaults,
                        access: event.target.value as McpAccessLevel,
                      },
                    }))
                  }
                >
                  {ACCESS_LEVELS.map((level) => (
                    <option key={level} value={level}>
                      {ACCESS_LABELS[level]}
                    </option>
                  ))}
                </select>
              </label>
              <label>
                <span>调用确认</span>
                <select
                  className="mcp-policy-select"
                  value={draft.defaults.confirm}
                  onChange={(event) =>
                    update((policy) => ({
                      ...policy,
                      defaults: {
                        ...policy.defaults,
                        confirm: event.target.value as McpConfirmLevel,
                      },
                    }))
                  }
                >
                  {CONFIRM_LEVELS.map((level) => (
                    <option key={level} value={level}>
                      {CONFIRM_LABELS[level]}
                    </option>
                  ))}
                </select>
              </label>
              <label>
                <span>可见范围</span>
                <select
                  className="mcp-policy-select"
                  value={draft.visibility}
                  onChange={(event) =>
                    update((policy) => ({
                      ...policy,
                      visibility: event.target.value as McpPolicy["visibility"],
                    }))
                  }
                >
                  <option value="all">全部连接</option>
                  <option value="selected">仅选中的连接</option>
                </select>
              </label>
            </div>
            <p className="muted small">
              风险映射：低风险 → 只读；中风险 → 读写；高风险 → 完全访问；极高危始终拦截。
              确认阈值「中风险起」会确认读写及以上的调用，「高风险起」只确认完全访问。
            </p>
            <p className="muted small">
              「仅选中的连接」需要在下方资源树里勾选连接或分组。确认阈值不影响硬保护：极高危命令、非回环地址转发、未确认主机密钥始终拒绝；stdio 模式下需要人工确认的调用会自动拒绝。
            </p>
            <p className="muted small">
              这份策略只影响 MCP / Agent 调用；手动终端仍按环境与风险规则判定，极高危命令始终拦截。
            </p>
          </section>

          <section className="mcp-policy-block">
            <div className="mcp-policy-block-head">
              <div>
                <h3>访问控制</h3>
                <p className="muted small">
                  生效列显示最终级别与来源。连接可以单独收紧 SFTP 或端口转发。
                </p>
              </div>
            </div>
            <div className="mcp-policy-toolbar">
              <input
                className="mcp-policy-search"
                placeholder="搜索连接或分组…"
                value={query}
                onChange={(event) => setQuery(event.target.value)}
              />
              <label className="mcp-policy-check">
                <input
                  type="checkbox"
                  checked={exceptionsOnly}
                  onChange={(event) => setExceptionsOnly(event.target.checked)}
                />
                仅显示单独设置
              </label>
            </div>

            <div className="mcp-policy-table">
              <div className="mcp-policy-row mcp-policy-row-head">
                <span>资源</span>
                <span>可见</span>
                <span>访问级别</span>
                <span>确认</span>
                <span>生效</span>
              </div>
              {groups.map(([groupName, connections]) => {
                const visibleConnections = connections.filter(matches);
                if (visibleConnections.length === 0) return null;
                const groupTarget: McpRuleTarget = {
                  kind: "group",
                  name: connections[0].group.trim(),
                };
                const groupRule = ruleFor(draft, groupTarget);
                const groupEffective = groupRule?.access
                  ? ACCESS_LABELS[groupRule.access]
                  : `继承 · ${ACCESS_LABELS[draft.defaults.access]}`;
                return (
                  <div key={groupName} className="mcp-policy-group">
                    <div className="mcp-policy-row">
                      <span className="mcp-policy-resource">
                        <span className="mcp-policy-resource-text">
                          <strong>{groupName}</strong>
                          <small className="muted">
                            {connections.length} 个连接
                          </small>
                        </span>
                      </span>
                      <span>
                        {draft.visibility === "selected" ? (
                          <input
                            type="checkbox"
                            checked={groupRule?.visible ?? false}
                            onChange={(event) =>
                              update((policy) =>
                                setRuleField(
                                  policy,
                                  groupTarget,
                                  "visible",
                                  event.target.checked ? true : null,
                                ),
                              )
                            }
                          />
                        ) : (
                          <span className="muted">—</span>
                        )}
                      </span>
                      <span>
                        {renderLevelSelect(groupRule?.access, (value) =>
                          update((policy) =>
                            setRuleField(policy, groupTarget, "access", value),
                          ),
                        )}
                      </span>
                      <span>
                        {renderConfirmSelect(groupRule?.confirm, (value) =>
                          update((policy) =>
                            setRuleField(policy, groupTarget, "confirm", value),
                          ),
                        )}
                      </span>
                      <span className="muted">{groupEffective}</span>
                    </div>
                    {visibleConnections.map((connection) => {
                      const connectionTarget: McpRuleTarget = {
                        kind: "connection",
                        id: connection.connectionId,
                      };
                      const connectionRule = ruleFor(draft, connectionTarget);
                      const effective = effectiveFor(draft, connection);
                      const isExpanded = expanded.has(connection.connectionId);
                      const caps = draft.capabilities[connection.connectionId];
                      return (
                        <div key={connection.connectionId}>
                          <div className="mcp-policy-row mcp-policy-row-child">
                            <span className="mcp-policy-resource">
                              <button
                                className="icon-btn"
                                onClick={() => toggleExpanded(connection.connectionId)}
                                aria-label="展开能力例外"
                              >
                                {isExpanded ? "▾" : "▸"}
                              </button>
                              <span className="mcp-policy-resource-text">
                                <strong>{connection.alias}</strong>
                                <small className="muted">
                                  {connection.group || "未分组"} ·{" "}
                                  {ENV_LABELS[connection.environment]}
                                </small>
                              </span>
                            </span>
                            <span>
                              {draft.visibility === "selected" ? (
                                <input
                                  type="checkbox"
                                  checked={connectionRule?.visible ?? effective.visible}
                                  onChange={(event) =>
                                    update((policy) =>
                                      setRuleField(
                                        policy,
                                        connectionTarget,
                                        "visible",
                                        event.target.checked ? true : false,
                                      ),
                                    )
                                  }
                                />
                              ) : (
                                <input
                                  type="checkbox"
                                  checked={connectionRule?.visible ?? effective.visible}
                                  onChange={(event) =>
                                    update((policy) =>
                                      setRuleField(
                                        policy,
                                        connectionTarget,
                                        "visible",
                                        event.target.checked,
                                      ),
                                    )
                                  }
                                />
                              )}
                            </span>
                            <span>
                              {renderLevelSelect(connectionRule?.access, (value) =>
                                update((policy) =>
                                  setRuleField(
                                    policy,
                                    connectionTarget,
                                    "access",
                                    value,
                                  ),
                                ),
                              )}
                            </span>
                            <span>
                              {renderConfirmSelect(connectionRule?.confirm, (value) =>
                                update((policy) =>
                                  setRuleField(
                                    policy,
                                    connectionTarget,
                                    "confirm",
                                    value,
                                  ),
                                ),
                              )}
                            </span>
                            <span>
                              {effective.visible ? (
                                <>
                                  <strong>{ACCESS_LABELS[effective.access]}</strong>
                                  <small className="muted">
                                    {" "}
                                    · {SOURCE_LABELS[effective.source]}
                                  </small>
                                </>
                              ) : (
                                <span className="muted">不可见</span>
                              )}
                              {onOpenAudit && (
                                <button
                                  className="btn btn-ghost btn-sm mcp-policy-audit"
                                  onClick={() => onOpenAudit(connection.alias)}
                                >
                                  审计
                                </button>
                              )}
                            </span>
                          </div>
                          {isExpanded && (
                            <div className="mcp-policy-capabilities">
                              <label>
                                <span>SFTP 最高级别</span>
                                {renderLevelSelect(caps?.sftp, (value) =>
                                  update((policy) =>
                                    setCapability(
                                      policy,
                                      connection.connectionId,
                                      "sftp",
                                      value,
                                    ),
                                  ),
                                )}
                                <small className="muted">
                                  生效：{ACCESS_LABELS[effective.sftp]}
                                </small>
                              </label>
                              <label>
                                <span>端口转发最高级别</span>
                                {renderLevelSelect(caps?.forwarding, (value) =>
                                  update((policy) =>
                                    setCapability(
                                      policy,
                                      connection.connectionId,
                                      "forwarding",
                                      value,
                                    ),
                                  ),
                                )}
                                <small className="muted">
                                  生效：{ACCESS_LABELS[effective.forwarding]}
                                </small>
                              </label>
                              <span className="mcp-policy-pty">
                                PTY：
                                {connection.ptyEnabled
                                  ? effective.access === "full" && effective.visible
                                    ? "已开启（危险）"
                                    : "已开启，但访问级别不足"
                                  : "未开启"}
                                <small className="muted">
                                  {" "}
                                  在连接编辑器中开启/关闭
                                </small>
                              </span>
                            </div>
                          )}
                        </div>
                      );
                    })}
                  </div>
                );
              })}
              {snapshot.connections.length === 0 && (
                <p className="muted pad">还没有连接。</p>
              )}
            </div>
          </section>

          <section className="mcp-policy-block">
            <div className="mcp-policy-block-head">
              <div>
                <h3>工具开关</h3>
                <p className="muted small">
                  关闭的工具不会出现在 tools/list 中，调用也会被拒绝。
                </p>
              </div>
            </div>
            <div className="mcp-policy-tools">
              {TOOL_GROUPS.map((group) => {
                const tools = snapshot.tools.filter((tool) =>
                  group.match(tool.name),
                );
                if (tools.length === 0) return null;
                return (
                  <div key={group.title} className="mcp-policy-tool-group">
                    <h4>{group.title}</h4>
                    {tools.map((tool) => (
                      <label key={tool.name} className="mcp-policy-tool">
                        <input
                          type="checkbox"
                          checked={draft.tools[tool.name] ?? true}
                          onChange={(event) =>
                            update((policy) =>
                              setTool(policy, tool.name, event.target.checked),
                            )
                          }
                        />
                        <code>{tool.name}</code>
                        <span className={`risk-badge risk-${tool.risk}`}>
                          {RISK_LABELS[tool.risk]}
                        </span>
                        <span className="muted small">{tool.description}</span>
                      </label>
                    ))}
                  </div>
                );
              })}
            </div>
          </section>
        </div>

        <div className="mcp-policy-footer">
          <div className="mcp-policy-messages">
            {error && <span className="error-text">{error}</span>}
            {notice && <span className="ok-text">{notice}</span>}
          </div>
          <div className="drawer-actions">
            <button
              className="btn btn-ghost btn-sm"
              onClick={() => {
                setDraft(snapshot.policy);
                setNotice(null);
              }}
            >
              还原
            </button>
            <button
              className="btn btn-primary btn-sm"
              disabled={saving}
              onClick={() => void save()}
            >
              {saving ? "保存中…" : "应用"}
            </button>
          </div>
        </div>
    </DrawerShell>
  );
}
