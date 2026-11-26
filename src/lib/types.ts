// Shared types mirroring `src-tauri/src/models.rs` (serde camelCase).

export type Environment = "dev" | "staging" | "prod";

export type McpAccessMode = "disabled" | "read_only" | "standard_controlled";

export type AuthKind = "password" | "private_key" | "agent";

export interface AuthMethod {
  kind: AuthKind;
  secretRef?: string | null;
  privateKeyPath?: string | null;
}

export interface Connection {
  id: string;
  alias: string;
  host: string;
  port: number;
  username: string;
  group: string;
  tags: string[];
  environment: Environment;
  auth: AuthMethod;
  mcpAccess: McpAccessMode;
  jumpHost?: string | null;
  remark: string;
  term: string;
  encoding: string;
  startupCommand: string;
  legacyAlgorithms: boolean;
  forwardAgent: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface ConnectionInput {
  id?: string | null;
  alias: string;
  host: string;
  port: number;
  username: string;
  group: string;
  tags: string[];
  environment: Environment;
  auth: AuthMethod;
  mcpAccess: McpAccessMode;
  jumpHost?: string | null;
  remark: string;
  term: string;
  encoding: string;
  startupCommand: string;
  legacyAlgorithms: boolean;
  forwardAgent: boolean;
  password?: string | null;
  passphrase?: string | null;
}

export type AuditSource = "user" | "agent" | "system";

export type RiskLevel = "low" | "medium" | "high" | "critical";

export type PolicyAction = "allow" | "require_approval" | "block";

export interface PolicyDecision {
  action: PolicyAction;
  risk: RiskLevel;
  reasons: string[];
  ruleId?: string | null;
}

export interface CommandRequest {
  connectionId: string;
  command: string;
  source: AuditSource;
  timeoutSecs?: number | null;
}

export interface CommandResult {
  id: string;
  sessionId: string;
  connectionId: string;
  connectionAlias: string;
  environment: Environment;
  command: string;
  exitCode: number | null;
  output: string;
  truncated: boolean;
  risk: RiskLevel;
  policyAction: PolicyAction;
  decisionReasons: string[];
  durationMs: number;
  simulated: boolean;
}

export interface ApprovalRequest {
  id: string;
  connectionId: string;
  connectionAlias: string;
  environment: Environment;
  command: string;
  risk: RiskLevel;
  reasons: string[];
  source: AuditSource;
  requestedAt: string;
}

export type CommandOutcome =
  | { status: "completed"; result: CommandResult }
  | { status: "pending_approval"; approval: ApprovalRequest }
  | { status: "blocked"; decision: PolicyDecision; auditId: string; message: string };

export type ApprovalDecision = "approved" | "rejected";

export interface AuditEntry {
  id: string;
  timestamp: string;
  source: AuditSource;
  connectionId?: string | null;
  connectionAlias?: string | null;
  environment?: Environment | null;
  command?: string | null;
  risk?: RiskLevel | null;
  action: PolicyAction;
  approval?: ApprovalDecision | null;
  exitCode?: number | null;
  durationMs?: number | null;
  detail: string;
}

export interface SessionInfo {
  id: string;
  connectionId: string;
  connectionAlias: string;
  createdAt: string;
  lastActiveAt: string;
  status: "active" | "closed";
  outputLines: number;
  outputBytes: number;
}

export interface McpToolInfo {
  name: string;
  description: string;
  risk: RiskLevel;
}

export interface McpServerInfo {
  transport: string;
  command: string;
  status: "scaffold" | "ready" | "running" | "stopped";
  tools: McpToolInfo[];
  authorizedConnections: number;
  activeSessions: number;
}

/* ------------------------------------------------------------------ */
/* Interactive PTY (real SSH shell)                                    */
/* ------------------------------------------------------------------ */

export interface HostKey {
  /** e.g. `ssh-ed25519` */
  algorithm: string;
  /** `SHA256:...`, the same form OpenSSH prints. */
  fingerprint: string;
  /** `true` when the key was already trusted in our known_hosts file. */
  known: boolean;
}

export interface PtyOpened {
  sessionId: string;
  alias: string;
  hostKey: HostKey;
}

export interface SshConfigHost {
  alias: string;
  host: string;
  port: number;
  username: string;
  identityFile?: string | null;
  proxyJump?: string | null;
}

export interface SshConfigPreview {
  path: string;
  hosts: SshConfigHost[];
  warnings: string[];
  defaultUser: string;
}

export type ForwardKind = "local" | "remote" | "dynamic";

export interface ForwardRule {
  id: string;
  connectionId: string;
  kind: ForwardKind;
  bindHost: string;
  bindPort: number;
  targetHost: string;
  targetPort: number;
  remark: string;
}

export interface ForwardRuleInput {
  id?: string | null;
  connectionId: string;
  kind: ForwardKind;
  bindHost: string;
  bindPort: number;
  targetHost: string;
  targetPort: number;
  remark: string;
}

export interface ForwardStatus {
  ruleId: string;
  running: boolean;
  boundPort?: number | null;
  error?: string | null;
}

export interface SftpEntry {
  name: string;
  path: string;
  isDir: boolean;
  isSymlink: boolean;
  size: number;
  modified?: string | null;
  permissions?: number | null;
}

export interface SftpListing {
  path: string;
  entries: SftpEntry[];
}

export interface SftpOpened {
  sessionId: string;
  listing: SftpListing;
}

export interface SftpProgressEvent {
  sessionId: string;
  direction: "upload" | "download" | string;
  transferred: number;
  total: number;
}

/** A fingerprint the user confirmed in the first-connect dialog. */
export interface HostKeyTrust {
  host: string;
  port: number;
  fingerprint: string;
}

export interface HostKeyPrompt {
  host: string;
  port: number;
  algorithm: string;
  fingerprint: string;
}

/** Structured rejection from `pty_open`, so the UI can show a fingerprint dialog. */
export interface PtyOpenError {
  kind: "host_key_unknown" | "host_key_changed" | "other";
  message: string;
  hostKey?: HostKeyPrompt | null;
}

/** Payload of the `pty:output` Tauri event (raw bytes, base64). */
export interface PtyOutputEvent {
  sessionId: string;
  data: string;
}

export interface PtyExitEvent {
  sessionId: string;
  exitCode: number | null;
}

export interface PtyClosedEvent {
  sessionId: string;
  reason: string;
}

export interface AuthPromptInfo {
  prompt: string;
  echo: boolean;
}

/** Payload of the global `auth:prompt` Tauri event (keyboard-interactive / 2FA). */
export interface AuthPromptEvent {
  promptId: string;
  host: string;
  name: string;
  instructions: string;
  prompts: AuthPromptInfo[];
}

export interface AuthPromptClosedEvent {
  promptId: string;
}


export const ENV_LABELS: Record<Environment, string> = {
  dev: "开发",
  staging: "预发",
  prod: "生产",
};

export const MCP_ACCESS_LABELS: Record<McpAccessMode, string> = {
  disabled: "未授权",
  read_only: "只读",
  standard_controlled: "受控",
};

export const RISK_LABELS: Record<RiskLevel, string> = {
  low: "低风险",
  medium: "中风险",
  high: "高风险",
  critical: "极高危",
};

export const ACTION_LABELS: Record<PolicyAction, string> = {
  allow: "放行",
  require_approval: "需审批",
  block: "拦截",
};
