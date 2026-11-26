import { invoke } from "@tauri-apps/api/core";
import type {
  ApprovalRequest,
  AuditEntry,
  CommandOutcome,
  CommandRequest,
  CommandResult,
  Connection,
  ConnectionInput,
  HostKeyTrust,
  McpServerInfo,
  PolicyAction,
  PolicyDecision,
  PtyOpened,
  RiskLevel,
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

  forgetHostKey: (host: string, port: number): Promise<boolean> =>
    isTauri()
      ? invoke<boolean>("forget_host_key", { host, port })
      : Promise.resolve(false),

  ptyWrite: (sessionId: string, data: string): Promise<void> =>
    isTauri()
      ? invoke<void>("pty_write", { sessionId, data })
      : Promise.reject(new Error("浏览器 Mock 模式不提供真实 SSH 会话")),

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
    jumpHost: null,
    remark: "本地开发与调试用机器",
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
    jumpHost: null,
    remark: "预发 API 集群入口",
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
    jumpHost: "conn_staging_api",
    remark: "生产 Web 入口，默认只读授权",
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
      jumpHost: input.jumpHost ?? null,
      remark: input.remark,
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
      status: "scaffold",
      tools: [
        { name: "list_servers", description: "列出已授权给 MCP 的服务器元信息", risk: "low" },
        { name: "exec_command", description: "在已授权服务器上执行命令（受策略引擎约束）", risk: "medium" },
        { name: "read_output", description: "分页读取会话输出", risk: "low" },
        { name: "send_signal", description: "向会话发送中断信号", risk: "medium" },
        { name: "close_session", description: "关闭指定会话", risk: "medium" },
      ],
      authorizedConnections: mockState.connections.filter((c) => c.mcpAccess !== "disabled").length,
      activeSessions: 0,
    };
  },
};
