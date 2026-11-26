import { describe, expect, it } from "vitest";
import { MCP_CLIENTS, type McpLaunchInfo } from "./mcpClientConfigs";

const launch: McpLaunchInfo = {
  command: "C:\\Program Files\\ShellX\\ShellX.exe",
  args: ["--mcp-stdio"],
};

function build(id: string): string {
  const client = MCP_CLIENTS.find((candidate) => candidate.id === id);
  expect(client, `missing client ${id}`).toBeDefined();
  return client!.build(launch);
}

describe("MCP client config templates", () => {
  it("covers every client shown in the picker", () => {
    expect(MCP_CLIENTS).toHaveLength(15);
    expect(MCP_CLIENTS.map((client) => client.name)).toEqual([
      "Claude Code",
      "Cursor",
      "CodeBuddy Code CLI",
      "ZCode",
      "TRAE",
      "VS Code",
      "Windsurf",
      "Codex",
      "DeepSeek Harness",
      "OpenCode",
      "Pi",
      "Cherry Studio",
      "Qoder",
      "WorkBuddy",
      "Antigravity",
    ]);
  });

  it("builds the standard mcpServers JSON", () => {
    expect(JSON.parse(build("claude-code"))).toEqual({
      mcpServers: {
        shellx: {
          command: launch.command,
          args: ["--mcp-stdio"],
        },
      },
    });
  });

  it("builds VS Code's servers + stdio format", () => {
    expect(JSON.parse(build("vscode"))).toEqual({
      servers: {
        shellx: {
          type: "stdio",
          command: launch.command,
          args: ["--mcp-stdio"],
        },
      },
    });
  });

  it("builds Codex TOML with a quoted executable path", () => {
    const config = build("codex");
    expect(config).toContain("[mcp_servers.shellx]");
    expect(config).toContain(`command = ${JSON.stringify(launch.command)}`);
    expect(config).toContain('args = ["--mcp-stdio"]');
  });

  it("builds OpenCode's local MCP command array", () => {
    expect(JSON.parse(build("opencode"))).toEqual({
      mcp: {
        shellx: {
          type: "local",
          command: [launch.command, "--mcp-stdio"],
        },
      },
    });
  });

  it("builds Cherry Studio's manual MCP entry", () => {
    const parsed = JSON.parse(build("cherry-studio"));
    expect(parsed.mcpServers.shellx).toMatchObject({
      name: "shellx",
      command: launch.command,
      args: ["--mcp-stdio"],
      type: "stdio",
      isActive: true,
    });
  });

  it("builds the DeepSeek Harness client snippet", () => {
    const config = build("deepseek");
    expect(config).toContain("serverName: shellx");
    expect(config).toContain("transport: stdio");
    expect(config).toContain(JSON.stringify(launch.command));
    expect(config).toContain('["--mcp-stdio"]');
  });

  it("keeps the launch arguments in every client format", () => {
    for (const client of MCP_CLIENTS) {
      expect(client.build(launch), client.name).toContain("--mcp-stdio");
      expect(client.build(launch), client.name).toContain(
        "C:\\\\Program Files\\\\ShellX\\\\ShellX.exe",
      );
    }
  });
});
