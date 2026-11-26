// Per-client MCP configuration snippets. Each client gets the same launch
// command (the ShellX executable with `--mcp-stdio`) in the format it expects.

export interface McpLaunchInfo {
  command: string;
  args: string[];
}

export interface McpClientConfig {
  id: string;
  name: string;
  format: "json" | "toml" | "yaml";
  pathHint: string;
  build: (launch: McpLaunchInfo) => string;
}

function stdioEntry(launch: McpLaunchInfo): Record<string, unknown> {
  const entry: Record<string, unknown> = { command: launch.command };
  if (launch.args.length > 0) entry.args = [...launch.args];
  return entry;
}

function json(value: unknown): string {
  return JSON.stringify(value, null, 2);
}

function quotedArray(values: readonly string[]): string {
  return `[${values.map((value) => JSON.stringify(value)).join(", ")}]`;
}

function standard(launch: McpLaunchInfo): string {
  return json({ mcpServers: { shellx: stdioEntry(launch) } });
}

function vsCode(launch: McpLaunchInfo): string {
  return json({
    servers: {
      shellx: { type: "stdio", ...stdioEntry(launch) },
    },
  });
}

function cherryStudio(launch: McpLaunchInfo): string {
  return json({
    mcpServers: {
      shellx: {
        name: "shellx",
        description: "",
        baseUrl: "",
        command: launch.command,
        args: [...launch.args],
        env: {},
        isActive: true,
        type: "stdio",
      },
    },
  });
}

function codex(launch: McpLaunchInfo): string {
  const lines = [
    "[mcp_servers.shellx]",
    `command = ${JSON.stringify(launch.command)}`,
  ];
  if (launch.args.length > 0) {
    lines.push(`args = ${quotedArray(launch.args)}`);
  }
  return lines.join("\n");
}

function deepSeekHarness(launch: McpLaunchInfo): string {
  const lines = [
    "- insert:",
    "    - id: mcp-shellx",
    "      name: '@deepseek-ai/dsh-mcp-client'",
    "      config:",
    "        serverName: shellx",
    "        transport: stdio",
    `        command: ${JSON.stringify(launch.command)}`,
  ];
  if (launch.args.length > 0) {
    lines.push(`        args: ${quotedArray(launch.args)}`);
  }
  return lines.join("\n");
}

function openCode(launch: McpLaunchInfo): string {
  return json({
    mcp: {
      shellx: {
        type: "local",
        command: [launch.command, ...launch.args],
      },
    },
  });
}

export const MCP_CLIENTS: McpClientConfig[] = [
  {
    id: "claude-code",
    name: "Claude Code",
    format: "json",
    pathHint: "Claude Code：~/.claude.json 或项目 .mcp.json",
    build: standard,
  },
  {
    id: "cursor",
    name: "Cursor",
    format: "json",
    pathHint: "Cursor：~/.cursor/mcp.json 或项目 .cursor/mcp.json",
    build: standard,
  },
  {
    id: "codebuddy",
    name: "CodeBuddy Code CLI",
    format: "json",
    pathHint: "CodeBuddy Code CLI 的 MCP 配置文件",
    build: standard,
  },
  {
    id: "zcode",
    name: "ZCode",
    format: "json",
    pathHint: "ZCode 的 MCP 配置文件",
    build: standard,
  },
  {
    id: "trae",
    name: "TRAE",
    format: "json",
    pathHint: "TRAE 的 MCP 配置文件",
    build: standard,
  },
  {
    id: "vscode",
    name: "VS Code",
    format: "json",
    pathHint: "VS Code：项目 .vscode/mcp.json 或用户设置中的 MCP Servers",
    build: vsCode,
  },
  {
    id: "windsurf",
    name: "Windsurf",
    format: "json",
    pathHint: "Windsurf：~/.codeium/windsurf/mcp_config.json",
    build: standard,
  },
  {
    id: "codex",
    name: "Codex",
    format: "toml",
    pathHint: "Codex：~/.codex/config.toml",
    build: codex,
  },
  {
    id: "deepseek",
    name: "DeepSeek Harness",
    format: "yaml",
    pathHint: "DeepSeek Harness 的 MCP 客户端配置",
    build: deepSeekHarness,
  },
  {
    id: "opencode",
    name: "OpenCode",
    format: "json",
    pathHint: "OpenCode：opencode.json",
    build: openCode,
  },
  {
    id: "pi",
    name: "Pi",
    format: "json",
    pathHint: "Pi 的 MCP 配置文件",
    build: standard,
  },
  {
    id: "cherry-studio",
    name: "Cherry Studio",
    format: "json",
    pathHint: "Cherry Studio：设置 → MCP 服务器 → 手动添加/JSON",
    build: cherryStudio,
  },
  {
    id: "qoder",
    name: "Qoder",
    format: "json",
    pathHint: "Qoder 的 MCP 配置文件",
    build: standard,
  },
  {
    id: "workbuddy",
    name: "WorkBuddy",
    format: "json",
    pathHint: "WorkBuddy 的 MCP 配置文件",
    build: standard,
  },
  {
    id: "antigravity",
    name: "Antigravity",
    format: "json",
    pathHint: "Antigravity 的 MCP 配置文件",
    build: standard,
  },
];
