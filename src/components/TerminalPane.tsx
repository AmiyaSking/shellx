import { useCallback, useEffect, useRef, useState } from "react";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import { api } from "../lib/api";
import {
  ENV_LABELS,
  MCP_ACCESS_LABELS,
  type ApprovalRequest,
  type CommandOutcome,
  type Connection,
} from "../lib/types";
import ApprovalDialog from "./ApprovalDialog";

interface Props {
  connection: Connection;
  active: boolean;
}

export default function TerminalPane({ connection, active }: Props) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const inputRef = useRef("");
  const agentModeRef = useRef(false);
  const connectionRef = useRef(connection);
  const promptRef = useRef<() => void>(() => {});
  const submitRef = useRef<(command: string) => void>(() => {});

  const [agentMode, setAgentMode] = useState(false);
  const [approval, setApproval] = useState<ApprovalRequest | null>(null);

  useEffect(() => {
    connectionRef.current = connection;
  }, [connection]);

  const prompt = useCallback(() => {
    const term = terminalRef.current;
    if (!term) return;
    const conn = connectionRef.current;
    if (agentModeRef.current) {
      term.write(
        `\r\n\x1b[38;5;215m[AI]\x1b[0m \x1b[38;5;245m${conn.username}@${conn.alias}\x1b[0m $ `,
      );
    } else {
      term.write(
        `\r\n\x1b[38;5;114m${conn.username}@${conn.alias}\x1b[0m:\x1b[38;5;110m~\x1b[0m$ `,
      );
    }
  }, []);

  const writeOutcome = useCallback(
    (outcome: CommandOutcome) => {
      const term = terminalRef.current;
      if (!term) return;
      if (outcome.status === "completed") {
        const result = outcome.result;
        const text = result.output.replace(/\r\n/g, "\n");
        for (const line of text.split("\n")) {
          term.writeln(line);
        }
        const simulated = result.simulated ? " · 模拟" : "";
        term.writeln(
          `\x1b[38;5;245m[exit ${result.exitCode ?? "-"} · ${result.durationMs}ms · ${result.risk}${simulated}]\x1b[0m`,
        );
        prompt();
        return;
      }
      if (outcome.status === "blocked") {
        term.writeln(`\x1b[31m✖ 已拦截：${outcome.message}\x1b[0m`);
        term.writeln(`\x1b[38;5;245maudit=${outcome.auditId}\x1b[0m`);
        prompt();
        return;
      }
      term.writeln("\x1b[33m⏸ 已挂起，等待人工审批…\x1b[0m");
      setApproval(outcome.approval);
    },
    [prompt],
  );

  const submit = useCallback(
    async (command: string) => {
      const term = terminalRef.current;
      if (!term) return;
      term.writeln("\x1b[38;5;245m• 策略判定中…\x1b[0m");
      try {
        const outcome = await api.runCommand({
          connectionId: connectionRef.current.id,
          command,
          source: agentModeRef.current ? "agent" : "user",
        });
        writeOutcome(outcome);
      } catch (err) {
        term.writeln(`\x1b[31m✖ 调用失败：${String(err)}\x1b[0m`);
        prompt();
      }
    },
    [prompt, writeOutcome],
  );

  useEffect(() => {
    promptRef.current = prompt;
  }, [prompt]);

  useEffect(() => {
    submitRef.current = submit;
  }, [submit]);

  // One xterm instance per tab. Connection metadata changes are picked up
  // through refs without resetting the scrollback.
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const term = new Terminal({
      cursorBlink: true,
      fontSize: 13,
      lineHeight: 1.35,
      scrollback: 5000,
      fontFamily:
        '"JetBrains Mono", "Cascadia Code", "SFMono-Regular", Consolas, monospace',
      theme: {
        background: "#0a0e13",
        foreground: "#d8e2ec",
        cursor: "#5eead4",
        cursorAccent: "#0a0e13",
        selectionBackground: "rgba(94, 234, 212, 0.25)",
        black: "#1b2430",
        red: "#f87171",
        green: "#5eead4",
        yellow: "#fbbf24",
        blue: "#60a5fa",
        magenta: "#c084fc",
        cyan: "#22d3ee",
        white: "#d8e2ec",
      },
    });
    const fitAddon = new FitAddon();
    term.loadAddon(fitAddon);
    term.open(container);
    terminalRef.current = term;
    fitAddonRef.current = fitAddon;

    const conn = connectionRef.current;
    term.writeln(
      "\x1b[1;38;5;121mShellX\x1b[0m \x1b[38;5;245mv0.1.0 · 安全终端脚手架\x1b[0m",
    );
    term.writeln(
      `\x1b[38;5;245m连接\x1b[0m ${conn.username}@${conn.host}:${conn.port}  ` +
        `\x1b[38;5;245m环境\x1b[0m ${ENV_LABELS[conn.environment]}  ` +
        `\x1b[38;5;245mMCP\x1b[0m ${MCP_ACCESS_LABELS[conn.mcpAccess]}`,
    );
    term.writeln(
      "\x1b[38;5;245m策略引擎已生效；SSH 通道为模拟执行，将在第一阶段接入。\x1b[0m",
    );
    promptRef.current();

    const safeFit = () => {
      try {
        fitAddon.fit();
      } catch {
        // The pane can be hidden while fitting; retried when it becomes active.
      }
    };
    const raf = requestAnimationFrame(safeFit);
    const observer = new ResizeObserver(safeFit);
    observer.observe(container);
    window.addEventListener("resize", safeFit);

    const dataSub = term.onData((data) => {
      if (data === "\r") {
        const command = inputRef.current.trim();
        inputRef.current = "";
        term.write("\r\n");
        if (!command) {
          promptRef.current();
          return;
        }
        void submitRef.current(command);
        return;
      }
      if (data === "\u007f") {
        if (inputRef.current.length > 0) {
          inputRef.current = inputRef.current.slice(0, -1);
          term.write("\b \b");
        }
        return;
      }
      if (data === "\u0003") {
        inputRef.current = "";
        term.write("^C");
        promptRef.current();
        return;
      }
      if (data === "\u000c") {
        term.clear();
        promptRef.current();
        return;
      }
      if (data.startsWith("\u001b")) return;
      if (data.includes("\r") || data.includes("\n")) return;
      inputRef.current += data;
      term.write(data);
    });

    return () => {
      cancelAnimationFrame(raf);
      observer.disconnect();
      window.removeEventListener("resize", safeFit);
      dataSub.dispose();
      term.dispose();
      terminalRef.current = null;
      fitAddonRef.current = null;
      container.innerHTML = "";
    };
  }, [connection.id]);

  useEffect(() => {
    if (!active) return;
    const timer = window.setTimeout(() => {
      try {
        fitAddonRef.current?.fit();
      } catch {
        // ignore
      }
      terminalRef.current?.focus();
    }, 30);
    return () => window.clearTimeout(timer);
  }, [active]);

  const toggleAgentMode = () => {
    const next = !agentMode;
    setAgentMode(next);
    agentModeRef.current = next;
    const term = terminalRef.current;
    if (!term) return;
    term.writeln("");
    term.writeln(
      next
        ? "\x1b[33m已切换为 AI Agent 模式：命令将按该连接的 MCP 授权与策略引擎判定。\x1b[0m"
        : "\x1b[38;5;114m已切换为手动模式。\x1b[0m",
    );
    promptRef.current();
  };

  return (
    <div className={`terminal-pane ${active ? "is-active" : ""}`}>
      <div className="terminal-toolbar">
        <div className="terminal-identity">
          <span className={`env-dot env-${connection.environment}`} />
          <strong>{connection.alias}</strong>
          <span className={`badge badge-${connection.environment}`}>
            {ENV_LABELS[connection.environment]}
          </span>
          <span className="muted small">
            {connection.username}@{connection.host}:{connection.port}
          </span>
          {connection.jumpHost && (
            <span className="mini-badge">经跳板机</span>
          )}
        </div>
        <div className="terminal-controls">
          <span className={`mini-badge mcp-${connection.mcpAccess}`}>
            MCP · {MCP_ACCESS_LABELS[connection.mcpAccess]}
          </span>
          <button
            type="button"
            className={`mode-switch ${agentMode ? "is-agent" : ""}`}
            onClick={toggleAgentMode}
            title="切换命令来源身份，用于演示 MCP 授权与审批策略"
          >
            <span className="mode-knob" />
            {agentMode ? "AI Agent 模式" : "手动模式"}
          </button>
        </div>
      </div>

      <div className="xterm-host" ref={containerRef} />

      {approval && (
        <ApprovalDialog
          approval={approval}
          onResolved={(outcome) => {
            setApproval(null);
            writeOutcome(outcome);
          }}
        />
      )}
    </div>
  );
}
