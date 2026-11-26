import { invoke } from "@tauri-apps/api/core";
import type {
  ApprovalRequest,
  AuditEntry,
  CommandOutcome,
  CommandRequest,
  CommandResult,
  Connection,
  ConnectionInput,
  ForwardRule,
  ForwardRuleInput,
  ForwardStatus,
  HostKeyTrust,
  McpServerInfo,
  PolicyAction,
  PolicyDecision,
  PtyOpened,
  RiskLevel,
  SftpEntry,
  SftpListing,
  SftpOpened,
  SshConfigPreview,
} from "./types";

export const isTauri = (): boolean =>
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export const api = {
  listConnections: (): Promise<Connection[]> =>
    isTauri() ? invoke<Connection[]>("list_connections") : mock.listConnections(),

  saveConnection: (input: ConnectionInput): Promise<Connection> =>
    isTauri()
      ? invoke<Connection>("save_connection", { input })
      : mock.saveConnection(input),

  deleteConnection: (id: string): Promise<void> =>
    isTauri()
      ? invoke<void>("delete_connection", { id })
      : mock.deleteConnection(id),

  runCommand: (request: CommandRequest): Promise<CommandOutcome> =>
    isTauri()
      ? invoke<CommandOutcome>("run_command", { request })
      : mock.runCommand(request),

  resolveApproval: (
    approvalId: string,
    approve: boolean,
    commandOverride?: string | null,
  ): Promise<CommandOutcome> =>
    isTauri()
      ? invoke<CommandOutcome>("resolve_approval", {
          approvalId,
          approve,
          commandOverride: commandOverride ?? null,
        })
      : mock.resolveApproval(approvalId, approve, commandOverride),

  listAuditEntries: (limit = 100): Promise<AuditEntry[]> =>
    isTauri()
      ? invoke<AuditEntry[]>("list_audit_entries", { limit })
      : mock.listAuditEntries(limit),

  mcpServerInfo: (): Promise<McpServerInfo> =>
    isTauri() ? invoke<McpServerInfo>("mcp_server_info") : mock.mcpServerInfo(),

  /* ---- interactive PTY (real SSH shell, desktop build only) ---- */

  ptyOpen: (
    connectionId: string,
    cols: number,
    rows: number,
    trustedHostKeys: HostKeyTrust[],
    requestId: string,
  ): Promise<PtyOpened> =>
    isTauri()
      ? invoke<PtyOpened>("pty_open", {
          connectionId,
          cols,
          rows,
          requestId,
          trustedHostKeys,
        })
      : Promise.reject(new Error("浏览器 Mock 模式不提供真实 SSH 会话，请使用桌面版")),

  ptyCancel: (requestId: string): Promise<boolean> =>
    isTauri()
      ? invoke<boolean>("pty_cancel", { requestId })
      : Promise.resolve(false),

  authRespond: (
    promptId: string,
    responses: string[],
  ): Promise<boolean> =>
    isTauri()
      ? invoke<boolean>("auth_respond", { promptId, responses })
      : Promise.resolve(true),

  authCancel: (promptId: string): Promise<boolean> =>
    isTauri()
      ? invoke<boolean>("auth_cancel", { promptId })
      : Promise.resolve(true),

  forgetHostKey: (host: string, port: number): Promise<boolean> =>
    isTauri()
      ? invoke<boolean>("forget_host_key", { host, port })
      : Promise.resolve(false),

  ptyWrite: (sessionId: string, data: string): Promise<void> =>
    isTauri()
      ? invoke<void>("pty_write", { sessionId, data })
      : Promise.reject(new Error("浏览器 Mock 模式不提供真实 SSH 会话")),

  /** Return consumed output bytes to the backend's flow-control window. */
  ptyAck: (sessionId: string, bytes: number): Promise<boolean> =>
    isTauri()
      ? invoke<boolean>("pty_ack", { sessionId, bytes })
      : Promise.resolve(false),

  ptyResize: (sessionId: string, cols: number, rows: number): Promise<void> =>
    isTauri()
      ? invoke<void>("pty_resize", { sessionId, cols, rows })
      : Promise.reject(new Error("浏览器 Mock 模式不提供真实 SSH 会话")),

  ptySignal: (sessionId: string, signal: string): Promise<void> =>
    isTauri()
      ? invoke<void>("pty_signal", { sessionId, signal })
      : Promise.reject(new Error("浏览器 Mock 模式不提供真实 SSH 会话")),

  ptyClose: (sessionId: string): Promise<boolean> =>
    isTauri()
      ? invoke<boolean>("pty_close", { sessionId })
      : Promise.resolve(false),

  ptyActiveSessions: (): Promise<number> =>
    isTauri() ? invoke<number>("pty_active_sessions") : Promise.resolve(0),

  setSessionLogging: (enabled: boolean): Promise<void> =>
    isTauri()
      ? invoke<void>("set_session_logging", { enabled })
      : Promise.resolve(),

  previewSshConfig: (): Promise<SshConfigPreview> =>
    isTauri()
      ? invoke<SshConfigPreview>("preview_ssh_config")
      : mock.previewSshConfig(),

  listForwardRules: (): Promise<ForwardRule[]> =>
    isTauri()
      ? invoke<ForwardRule[]>("list_forward_rules")
      : mock.listForwardRules(),

  saveForwardRule: (input: ForwardRuleInput): Promise<ForwardRule> =>
    isTauri()
      ? invoke<ForwardRule>("save_forward_rule", { input })
      : mock.saveForwardRule(input),

  deleteForwardRule: (id: string): Promise<void> =>
    isTauri()
      ? invoke<void>("delete_forward_rule", { id })
      : mock.deleteForwardRule(id),

  startForward: (id: string): Promise<ForwardStatus> =>
    isTauri()
      ? invoke<ForwardStatus>("start_forward", { id })
      : mock.startForward(id),

  stopForward: (id: string): Promise<boolean> =>
    isTauri()
      ? invoke<boolean>("stop_forward", { id })
      : mock.stopForward(id),

  listForwardStatuses: (): Promise<ForwardStatus[]> =>
    isTauri()
      ? invoke<ForwardStatus[]>("list_forward_statuses")
      : mock.listForwardStatuses(),

  sftpOpen: (connectionId: string): Promise<SftpOpened> =>
    isTauri()
      ? invoke<SftpOpened>("sftp_open", { connectionId })
      : mock.sftpOpen(connectionId),

  sftpList: (sessionId: string, path: string): Promise<SftpListing> =>
    isTauri()
      ? invoke<SftpListing>("sftp_list", { sessionId, path })
      : mock.sftpList(sessionId, path),

  sftpDownload: (
    sessionId: string,
    remotePath: string,
    localPath: string,
  ): Promise<number> =>
    isTauri()
      ? invoke<number>("sftp_download", {
          sessionId,
          remotePath,
          localPath,
        })
      : mock.sftpDownload(sessionId, remotePath, localPath),

  sftpUpload: (
    sessionId: string,
    localPath: string,
    remotePath: string,
  ): Promise<number> =>
    isTauri()
      ? invoke<number>("sftp_upload", {
          sessionId,
          localPath,
          remotePath,
        })
      : mock.sftpUpload(sessionId, localPath, remotePath),

  sftpMkdir: (sessionId: string, path: string): Promise<void> =>
    isTauri()
      ? invoke<void>("sftp_mkdir", { sessionId, path })
      : mock.sftpMkdir(path),

  sftpRemove: (
    sessionId: string,
    path: string,
    isDir: boolean,
  ): Promise<void> =>
    isTauri()
      ? invoke<void>("sftp_remove", { sessionId, path, isDir })
      : mock.sftpRemove(path, isDir),

  sftpCancel: (sessionId: string): Promise<boolean> =>
    isTauri()
      ? invoke<boolean>("sftp_cancel", { sessionId })
      : Promise.resolve(false),

  sftpRename: (
    sessionId: string,
    oldPath: string,
    newPath: string,
  ): Promise<void> =>
    isTauri()
      ? invoke<void>("sftp_rename", { sessionId, oldPath, newPath })
      : mock.sftpRename(oldPath, newPath),

  sftpClose: (sessionId: string): Promise<boolean> =>
    isTauri()
      ? invoke<boolean>("sftp_close", { sessionId })
      : Promise.resolve(true),
};

/* ------------------------------------------------------------------ */
/* Browser-only mock backend.                                          */
/* Mirrors the Rust policy engine well enough to demo the UX without   */
/* a Tauri/Rust toolchain. Real enforcement always lives in Rust.      */
/* ------------------------------------------------------------------ */

let seq = 0;
const nextId = (prefix: string) => `${prefix}_${Date.now().toString(36)}_${(++seq).toString(36)}`;

const now = () => new Date().toISOString();

const demoConnections: Connection[] = [
  {
    id: "conn_dev_box",
    alias: "dev-box",
    host: "10.10.0.21",
    port: 22,
    username: "amiya",
    group: "研发环境",
    tags: ["daily", "docker"],
    environment: "dev",
    auth: { kind: "agent", secretRef: null, privateKeyPath: null },
    mcpAccess: "standard_controlled",
    mcpPtyEnabled: false,
    jumpHost: null,
    remark: "本地开发与调试用机器",
    term: "xterm-256color",
    encoding: "utf-8",
    startupCommand: "",
    legacyAlgorithms: false,
    forwardAgent: false,
    createdAt: now(),
    updatedAt: now(),
  },
  {
    id: "conn_staging_api",
    alias: "staging-api",
    host: "10.20.0.11",
    port: 22,
    username: "deploy",
    group: "预发环境",
    tags: ["api"],
    environment: "staging",
    auth: { kind: "private_key", secretRef: null, privateKeyPath: "~/.ssh/id_ed25519" },
    mcpAccess: "read_only",
    mcpPtyEnabled: false,
    jumpHost: null,
    remark: "预发 API 集群入口",
    term: "xterm-256color",
    encoding: "utf-8",
    startupCommand: "cd /srv/api",
    legacyAlgorithms: false,
    forwardAgent: false,
    createdAt: now(),
    updatedAt: now(),
  },
  {
    id: "conn_prod_web",
    alias: "prod-web-01",
    host: "10.30.0.8",
    port: 22,
    username: "ops",
    group: "生产环境",
    tags: ["nginx", "critical"],
    environment: "prod",
    auth: { kind: "password", secretRef: "vault://prod-web-01", privateKeyPath: null },
    mcpAccess: "read_only",
    mcpPtyEnabled: false,
    jumpHost: "conn_staging_api",
    remark: "生产 Web 入口，默认只读授权",
    term: "xterm-256color",
    encoding: "utf-8",
    startupCommand: "",
    legacyAlgorithms: false,
    forwardAgent: false,
    createdAt: now(),
    updatedAt: now(),
  },
];

const mockState = {
  connections: demoConnections,
  approvals: new Map<
    string,
    { request: ApprovalRequest; command: string; source: CommandRequest["source"] }
  >(),
  audit: [] as AuditEntry[],
  forwardRules: [
    {
      id: "fwd_demo_local",
      connectionId: "conn_dev_box",
      kind: "local",
      bindHost: "127.0.0.1",
      bindPort: 8080,
      targetHost: "127.0.0.1",
      targetPort: 80,
      remark: "本地 Web 调试",
    },
    {
      id: "fwd_demo_socks",
      connectionId: "conn_staging_api",
      kind: "dynamic",
      bindHost: "127.0.0.1",
      bindPort: 1080,
      targetHost: "",
      targetPort: 0,
      remark: "SOCKS5 代理",
    },
  ] as ForwardRule[],
  forwardStatuses: new Map<string, ForwardStatus>(),
};

const HIGH_RISK = [
  ["rm ", "包含删除操作"],
  ["systemctl restart", "重启系统服务"],
  ["systemctl stop", "停止系统服务"],
  ["shutdown", "影响主机运行状态"],
  ["reboot", "重启主机"],
  ["kill", "终止进程"],
  ["chmod", "变更文件权限"],
  ["chown", "变更文件所有者"],
  ["iptables", "变更防火墙规则"],
  ["ufw ", "变更防火墙规则"],
  ["useradd", "变更系统用户"],
  ["passwd", "变更账户凭据"],
  ["docker rm", "删除容器"],
  ["kubectl delete", "删除集群资源"],
  ["git push --force", "强制推送远端历史"],
  ["sed -i", "原地修改文件"],
];

const LOW_RISK_PREFIX = [
  "ls", "ll", "pwd", "whoami", "hostname", "date", "uptime", "uname",
  "df", "du", "free", "ps", "top", "htop", "cat", "head", "tail", "less",
  "grep", "wc", "find", "systemctl status", "journalctl", "docker ps",
  "docker logs", "kubectl get", "git status", "git log", "git diff", "echo",
];

function classify(command: string): { risk: RiskLevel; reasons: string[] } {
  const cmd = command.trim().toLowerCase();
  if (!cmd) return { risk: "medium", reasons: ["空命令"] };
  if (/(rm\s+-[a-z]*[rf][a-z]*\s+\/(\s|$|\*))|mkfs|dd\s+.*of=\/dev\/|:\(\)\s*\{/.test(cmd)) {
    return { risk: "critical", reasons: ["命中极高危破坏性命令规则"] };
  }
  const high = HIGH_RISK.find(([pattern]) => cmd.includes(pattern));
  if (high) return { risk: "high", reasons: [high[1]] };
  if (cmd.startsWith("sudo ")) return { risk: "high", reasons: ["请求提权执行"] };
  if (LOW_RISK_PREFIX.some((prefix) => cmd === prefix || cmd.startsWith(`${prefix} `))) {
    return { risk: "low", reasons: ["只读检查类命令"] };
  }
  return { risk: "medium", reasons: ["未命中白名单，按中风险处理"] };
}

function decide(
  connection: Connection,
  command: string,
  source: CommandRequest["source"],
): PolicyDecision {
  const base = classify(command);
  if (base.risk === "critical") {
    return { action: "block", risk: base.risk, reasons: base.reasons, ruleId: "critical-destructive" };
  }
  if (source === "agent" && connection.mcpAccess === "disabled") {
    return { action: "block", risk: base.risk, reasons: ["该连接未授权给 MCP"], ruleId: "mcp-not-authorized" };
  }
  if (source === "agent" && connection.mcpAccess === "read_only" && base.risk !== "low") {
    return { action: "block", risk: base.risk, reasons: ["该连接对 MCP 为只读模式，仅允许低风险命令"], ruleId: "mcp-read-only" };
  }
  const matrix: Record<Connection["environment"], Record<RiskLevel, PolicyAction>> = {
    dev: { low: "allow", medium: "allow", high: "require_approval", critical: "block" },
    staging: { low: "allow", medium: "require_approval", high: "require_approval", critical: "block" },
    prod: { low: "allow", medium: "require_approval", high: "require_approval", critical: "block" },
  };
  return {
    action: matrix[connection.environment][base.risk],
    risk: base.risk,
    reasons: base.reasons,
    ruleId: `env-${connection.environment}`,
  };
}

function simulate(connection: Connection, command: string): { exitCode: number; output: string } {
  const cmd = command.trim();
  const first = cmd.toLowerCase();
  const output = (() => {
    if (first.startsWith("echo ")) return cmd.slice(5);
    if (first === "whoami") return connection.username;
    if (first === "hostname") return connection.alias;
    if (first === "pwd") return `/home/${connection.username}`;
    if (first === "date") return new Date().toString();
    if (first === "uptime") return " 14:32:01 up 42 days,  3:17,  2 users,  load average: 0.08, 0.12, 0.09";
    if (first === "ls" || first.startsWith("ls ") || first === "ll") {
      return "app  config  logs  scripts  README.md";
    }
    if (first.startsWith("df")) return "Filesystem      Size  Used Avail Use% Mounted on\n/dev/vda1        80G   31G   45G  41% /";
    if (first.startsWith("systemctl status")) return `● ${cmd.split(/\s+/).slice(2).join(" ")}.service - simulated\n     Loaded: loaded\n     Active: active (running)`;
    return [
      "[shellx] 脚手架模拟执行：命令已通过策略引擎，但尚未接入真实 SSH 通道。",
      `[shellx] target=${connection.username}@${connection.host}:${connection.port}  alias=${connection.alias}`,
      "[shellx] TODO(phase-1): 使用 russh 建立连接并回传真实输出。",
    ].join("\r\n");
  })();
  return { exitCode: 0, output };
}

function audit(entry: Omit<AuditEntry, "id" | "timestamp">): string {
  const id = nextId("audit");
  mockState.audit.unshift({ ...entry, id, timestamp: now() });
  mockState.audit = mockState.audit.slice(0, 500);
  return id;
}

function execute(
  connection: Connection,
  command: string,
  source: CommandRequest["source"],
  decision: PolicyDecision,
  approval: AuditEntry["approval"] = null,
): CommandOutcome {
  const started = Date.now();
  const { exitCode, output } = simulate(connection, command);
  const result: CommandResult = {
    id: nextId("cmd"),
    sessionId: `session_${connection.id}`,
    connectionId: connection.id,
    connectionAlias: connection.alias,
    environment: connection.environment,
    command,
    exitCode,
    output,
    truncated: false,
    risk: decision.risk,
    policyAction: decision.action,
    decisionReasons: decision.reasons,
    durationMs: Date.now() - started,
    simulated: true,
  };
  audit({
    source,
    connectionId: connection.id,
    connectionAlias: connection.alias,
    environment: connection.environment,
    command,
    risk: decision.risk,
    action: decision.action,
    approval,
    exitCode,
    durationMs: result.durationMs,
    detail: "命令执行完成（浏览器 Mock）",
  });
  return { status: "completed", result };
}

const mockSftpOverrides = new Map<string, SftpEntry[]>();

function normalizeMockPath(path: string): string {
  return path.replace(/\/+$/, "") || "/";
}

function mockSftpParent(path: string): string {
  const normalized = normalizeMockPath(path);
  if (normalized === "/") return "/";
  const index = normalized.lastIndexOf("/");
  return index <= 0 ? "/" : normalized.slice(0, index);
}

function mockSftpName(path: string): string {
  const normalized = normalizeMockPath(path);
  return normalized.slice(normalized.lastIndexOf("/") + 1);
}

function mockSftpBaseListing(path: string): SftpListing {
  const normalized = path.replace(/\/+$/, "") || "/";
  const now = new Date().toISOString();
  const entry = (
    name: string,
    isDir: boolean,
    size: number,
    extra: Partial<SftpEntry> = {},
  ): SftpEntry => ({
    name,
    path: normalized === "/" ? `/${name}` : `${normalized}/${name}`,
    isDir,
    isSymlink: false,
    size,
    modified: now,
    permissions: isDir ? 0o755 : 0o644,
    ...extra,
  });

  if (normalized === "/home/amiya") {
    return {
      path: normalized,
      entries: [
        entry("app", true, 4096),
        entry("logs", true, 4096),
        entry("config", true, 4096),
        entry("README.md", false, 2048),
        entry("deploy.sh", false, 1024),
        entry(".bashrc", false, 571),
      ],
    };
  }
  if (normalized === "/home/amiya/app") {
    return {
      path: normalized,
      entries: [
        entry("releases", true, 4096),
        entry("current", false, 0, {
          isSymlink: true,
          permissions: 0o777,
        }),
        entry("app.jar", false, 18_432_000),
        entry("config.yml", false, 1024),
      ],
    };
  }
  if (normalized === "/home/amiya/logs") {
    return {
      path: normalized,
      entries: [
        entry("app.log", false, 524_288),
        entry("error.log", false, 8192),
      ],
    };
  }
  if (normalized === "/home") {
    return {
      path: normalized,
      entries: [entry("amiya", true, 4096), entry("deploy", true, 4096)],
    };
  }
  if (normalized === "/") {
    return {
      path: normalized,
      entries: [
        entry("home", true, 4096),
        entry("etc", true, 4096),
        entry("var", true, 4096),
      ],
    };
  }
  return { path: normalized, entries: [] };
}

function mockSftpListing(path: string): SftpListing {
  const normalized = normalizeMockPath(path);
  const override = mockSftpOverrides.get(normalized);
  if (override) {
    return { path: normalized, entries: override.map((entry) => ({ ...entry })) };
  }
  return mockSftpBaseListing(normalized);
}

function mockSftpEntries(path: string): SftpEntry[] {
  return mockSftpListing(path).entries;
}

function mockSftpStore(path: string, entries: SftpEntry[]): void {
  mockSftpOverrides.set(normalizeMockPath(path), entries);
}

const mock = {
  async listConnections() {
    return [...mockState.connections];
  },

  async saveConnection(input: ConnectionInput): Promise<Connection> {
    const existing = input.id
      ? mockState.connections.find((c) => c.id === input.id)
      : undefined;
    const connection: Connection = {
      id: existing?.id ?? nextId("conn"),
      alias: input.alias,
      host: input.host,
      port: input.port,
      username: input.username,
      group: input.group,
      tags: input.tags,
      environment: input.environment,
      auth: input.auth,
      mcpAccess: input.mcpAccess,
      mcpPtyEnabled: input.mcpPtyEnabled ?? false,
      jumpHost: input.jumpHost ?? null,
      remark: input.remark,
      term: input.term || "xterm-256color",
      encoding: input.encoding || "utf-8",
      startupCommand: input.startupCommand || "",
      legacyAlgorithms: input.legacyAlgorithms,
      forwardAgent: input.forwardAgent,
      createdAt: existing?.createdAt ?? now(),
      updatedAt: now(),
    };
    mockState.connections = existing
      ? mockState.connections.map((c) => (c.id === connection.id ? connection : c))
      : [...mockState.connections, connection];
    return connection;
  },

  async deleteConnection(id: string) {
    mockState.connections = mockState.connections.filter((c) => c.id !== id);
  },

  async runCommand(request: CommandRequest): Promise<CommandOutcome> {
    const connection = mockState.connections.find((c) => c.id === request.connectionId);
    if (!connection) throw new Error("连接不存在");
    const decision = decide(connection, request.command, request.source);
    if (decision.action === "block") {
      const auditId = audit({
        source: request.source,
        connectionId: connection.id,
        connectionAlias: connection.alias,
        environment: connection.environment,
        command: request.command,
        risk: decision.risk,
        action: "block",
        detail: decision.reasons.join("；"),
      });
      return { status: "blocked", decision, auditId, message: decision.reasons.join("；") };
    }
    if (decision.action === "require_approval") {
      const approval: ApprovalRequest = {
        id: nextId("approval"),
        connectionId: connection.id,
        connectionAlias: connection.alias,
        environment: connection.environment,
        command: request.command,
        risk: decision.risk,
        reasons: decision.reasons,
        source: request.source,
        requestedAt: now(),
      };
      mockState.approvals.set(approval.id, {
        request: approval,
        command: request.command,
        source: request.source,
      });
      audit({
        source: request.source,
        connectionId: connection.id,
        connectionAlias: connection.alias,
        environment: connection.environment,
        command: request.command,
        risk: decision.risk,
        action: "require_approval",
        detail: "等待人工审批（浏览器 Mock）",
      });
      return { status: "pending_approval", approval };
    }
    return execute(connection, request.command, request.source, decision);
  },

  async resolveApproval(
    approvalId: string,
    approve: boolean,
    commandOverride?: string | null,
  ): Promise<CommandOutcome> {
    const pending = mockState.approvals.get(approvalId);
    if (!pending) throw new Error("审批请求不存在或已处理");
    mockState.approvals.delete(approvalId);
    const connection = mockState.connections.find((c) => c.id === pending.request.connectionId);
    if (!connection) throw new Error("连接不存在");
    const command = commandOverride?.trim() || pending.command;
    if (!approve) {
      audit({
        source: pending.source,
        connectionId: connection.id,
        connectionAlias: connection.alias,
        environment: connection.environment,
        command,
        risk: pending.request.risk,
        action: "block",
        approval: "rejected",
        detail: "用户拒绝了该命令",
      });
      return {
        status: "blocked",
        decision: {
          action: "block",
          risk: pending.request.risk,
          reasons: ["用户拒绝了该命令"],
          ruleId: "approval-rejected",
        },
        auditId: nextId("audit"),
        message: "用户拒绝了该命令",
      };
    }
    const decision = decide(connection, command, pending.source);
    if (decision.risk === "critical") {
      const auditId = audit({
        source: pending.source,
        connectionId: connection.id,
        connectionAlias: connection.alias,
        environment: connection.environment,
        command,
        risk: decision.risk,
        action: "block",
        approval: "approved",
        detail: "修改后的命令命中极高危规则，拒绝执行",
      });
      return { status: "blocked", decision, auditId, message: "修改后的命令仍命中极高危规则" };
    }
    return execute(connection, command, pending.source, { ...decision, action: "allow" }, "approved");
  },

  async listAuditEntries(limit: number) {
    return mockState.audit.slice(0, limit);
  },

  async listForwardRules(): Promise<ForwardRule[]> {
    return [...mockState.forwardRules];
  },

  async saveForwardRule(input: ForwardRuleInput): Promise<ForwardRule> {
    const existing = input.id
      ? mockState.forwardRules.find((rule) => rule.id === input.id)
      : undefined;
    const rule: ForwardRule = {
      id: existing?.id ?? nextId("fwd"),
      connectionId: input.connectionId,
      kind: input.kind,
      bindHost: input.bindHost,
      bindPort: input.bindPort,
      targetHost: input.kind !== "dynamic" ? input.targetHost : "",
      targetPort: input.kind !== "dynamic" ? input.targetPort : 0,
      remark: input.remark,
    };
    mockState.forwardRules = existing
      ? mockState.forwardRules.map((item) =>
          item.id === rule.id ? rule : item,
        )
      : [...mockState.forwardRules, rule];
    return rule;
  },

  async deleteForwardRule(id: string) {
    mockState.forwardRules = mockState.forwardRules.filter(
      (rule) => rule.id !== id,
    );
    mockState.forwardStatuses.delete(id);
  },

  async startForward(id: string): Promise<ForwardStatus> {
    const rule = mockState.forwardRules.find((item) => item.id === id);
    if (!rule) throw new Error("转发规则不存在");
    const status: ForwardStatus = {
      ruleId: id,
      running: true,
      boundPort: rule.bindPort === 0 ? 18080 : rule.bindPort,
      error: null,
    };
    mockState.forwardStatuses.set(id, status);
    return status;
  },

  async stopForward(id: string) {
    const had = mockState.forwardStatuses.has(id);
    mockState.forwardStatuses.delete(id);
    return had;
  },

  async listForwardStatuses(): Promise<ForwardStatus[]> {
    return mockState.forwardRules.map(
      (rule) =>
        mockState.forwardStatuses.get(rule.id) ?? {
          ruleId: rule.id,
          running: false,
          boundPort: null,
          error: null,
        },
    );
  },

  async sftpOpen(_connectionId: string): Promise<SftpOpened> {
    return {
      sessionId: nextId("sftp"),
      listing: mockSftpListing("/home/amiya"),
    };
  },

  async sftpList(_sessionId: string, path: string): Promise<SftpListing> {
    return mockSftpListing(path);
  },

  async sftpDownload(
    _sessionId: string,
    _remotePath: string,
    _localPath: string,
  ): Promise<number> {
    return 2048;
  },

  async sftpUpload(
    _sessionId: string,
    _localPath: string,
    remotePath: string,
  ): Promise<number> {
    const parent = mockSftpParent(remotePath);
    const name = mockSftpName(remotePath);
    const entries = mockSftpEntries(parent).filter(
      (entry) => entry.name !== name,
    );
    entries.push({
      name,
      path: remotePath,
      isDir: false,
      isSymlink: false,
      size: 2048,
      modified: new Date().toISOString(),
      permissions: 0o644,
    });
    mockSftpStore(parent, entries);
    return 2048;
  },

  async sftpMkdir(path: string): Promise<void> {
    const parent = mockSftpParent(path);
    const name = mockSftpName(path);
    const entries = mockSftpEntries(parent);
    if (entries.some((entry) => entry.name === name)) {
      throw new Error("目录已存在");
    }
    entries.push({
      name,
      path,
      isDir: true,
      isSymlink: false,
      size: 4096,
      modified: new Date().toISOString(),
      permissions: 0o755,
    });
    mockSftpStore(parent, entries);
    mockSftpStore(path, []);
  },

  async sftpRemove(path: string, isDir: boolean): Promise<void> {
    const parent = mockSftpParent(path);
    const name = mockSftpName(path);
    mockSftpStore(
      parent,
      mockSftpEntries(parent).filter((entry) => entry.name !== name),
    );
    if (isDir) {
      for (const key of [...mockSftpOverrides.keys()]) {
        if (key === path || key.startsWith(`${path}/`)) {
          mockSftpOverrides.delete(key);
        }
      }
    } else {
      mockSftpOverrides.delete(path);
    }
  },

  async sftpRename(oldPath: string, newPath: string): Promise<void> {
    const oldParent = mockSftpParent(oldPath);
    const newParent = mockSftpParent(newPath);
    const newName = mockSftpName(newPath);
    const oldEntries = mockSftpEntries(oldParent);
    const index = oldEntries.findIndex(
      (entry) => entry.name === mockSftpName(oldPath),
    );
    if (index < 0) throw new Error("源文件不存在");
    const [entry] = oldEntries.splice(index, 1);
    mockSftpStore(oldParent, oldEntries);
    const newEntries = mockSftpEntries(newParent).filter(
      (item) => item.name !== newName,
    );
    newEntries.push({ ...entry, name: newName, path: newPath });
    mockSftpStore(newParent, newEntries);
    if (entry.isDir) {
      const moved = [...mockSftpOverrides.keys()]
        .filter((key) => key.startsWith(`${oldPath}/`))
        .map((key) => ({
          oldKey: key,
          newKey: `${newPath}${key.slice(oldPath.length)}`,
          entries: mockSftpOverrides.get(key)!,
        }));
      for (const item of moved) {
        mockSftpOverrides.delete(item.oldKey);
        mockSftpOverrides.set(item.newKey, item.entries);
      }
    }
  },

  async previewSshConfig(): Promise<SshConfigPreview> {
    return {
      path: "~/.ssh/config",
      defaultUser: "amiya",
      warnings: ["Include 指令未展开，如需导入请手动处理被包含的文件"],
      hosts: [
        {
          alias: "prod-db",
          host: "10.30.0.20",
          port: 22,
          username: "dba",
          identityFile: "~/.ssh/id_ed25519",
          proxyJump: "bastion",
        },
        {
          alias: "staging-cache",
          host: "10.20.0.30",
          port: 2222,
          username: "deploy",
          identityFile: null,
          proxyJump: null,
        },
      ],
    };
  },

  async mcpServerInfo(): Promise<McpServerInfo> {
    return {
      transport: "stdio",
      command: "shellx --mcp-stdio",
      status: "ready",
      tools: [
        { name: "list_servers", description: "列出已授权给 MCP 的服务器元信息", risk: "low" },
        { name: "exec_command", description: "在已授权服务器上执行命令（受策略引擎约束）", risk: "medium" },
        { name: "read_output", description: "分页读取会话输出", risk: "low" },
        { name: "send_signal", description: "向会话发送中断信号", risk: "medium" },
        { name: "close_session", description: "关闭指定会话", risk: "medium" },
        { name: "sftp_open", description: "打开 SFTP 文件会话", risk: "low" },
        { name: "sftp_close", description: "关闭 SFTP 文件会话", risk: "low" },
        { name: "sftp_list", description: "列出远端目录", risk: "low" },
        { name: "sftp_stat", description: "读取远端路径元信息", risk: "low" },
        { name: "sftp_read_file", description: "读取远端文件内容", risk: "low" },
        { name: "sftp_write_file", description: "创建或覆盖远端文件", risk: "medium" },
        { name: "sftp_mkdir", description: "创建远端目录", risk: "medium" },
        { name: "sftp_remove", description: "删除远端文件或目录", risk: "medium" },
        { name: "sftp_rename", description: "重命名远端路径", risk: "medium" },
        { name: "list_forward_rules", description: "列出端口转发规则与状态", risk: "low" },
        { name: "start_forward", description: "启动预定义转发规则", risk: "medium" },
        { name: "stop_forward", description: "停止转发规则", risk: "medium" },
        { name: "terminal_open", description: "打开交互式 PTY（危险，需显式开启）", risk: "high" },
        { name: "terminal_write", description: "向交互式终端发送输入", risk: "high" },
        { name: "terminal_read", description: "读取交互式终端输出", risk: "medium" },
        { name: "terminal_resize", description: "调整终端窗口大小", risk: "low" },
        { name: "terminal_signal", description: "向终端前台进程发送信号", risk: "medium" },
        { name: "terminal_close", description: "关闭交互式终端", risk: "low" },
      ],
      authorizedConnections: mockState.connections.filter((c) => c.mcpAccess !== "disabled").length,
      activeSessions: 0,
    };
  },
};
