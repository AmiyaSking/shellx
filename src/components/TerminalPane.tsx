import { useCallback, useEffect, useRef, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import { api, isTauri } from "../lib/api";
import {
  ENV_LABELS,
  MCP_ACCESS_LABELS,
  type ApprovalRequest,
  type CommandOutcome,
  type Connection,
  type HostKey,
  type PtyClosedEvent,
  type PtyExitEvent,
  type PtyOutputEvent,
} from "../lib/types";
import ApprovalDialog from "./ApprovalDialog";

interface Props {
  connection: Connection;
  active: boolean;
}

type PtyStatus = "idle" | "connecting" | "live" | "closed" | "error";

/** The desktop build talks to a real SSH PTY; the browser demo does not. */
const PTY_CAPABLE = isTauri();

const STATUS_LABELS: Record<PtyStatus, string> = {
  idle: "未连接",
  connecting: "连接中…",
  live: "已连接",
  closed: "已断开",
  error: "连接失败",
};

/** Raw bytes arrive base64-encoded so a split UTF-8 character stays intact. */
function base64ToBytes(data: string): Uint8Array {
  const binary = atob(data);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

export default function TerminalPane({ connection, active }: Props) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const sessionRef = useRef<string | null>(null);
  const connectionRef = useRef(connection);
  const sizeRef = useRef<{ cols: number; rows: number } | null>(null);

  // Browser-demo REPL state (kept so `npm run dev` still shows something).
  const inputRef = useRef("");
  const agentModeRef = useRef(false);
  const promptRef = useRef<() => void>(() => {});
  const submitRef = useRef<(command: string) => void>(() => {});

  const [status, setStatus] = useState<PtyStatus>("idle");
  const [hostKey, setHostKey] = useState<HostKey | null>(null);
  const [errorText, setErrorText] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);

  const [agentMode, setAgentMode] = useState(false);
  const [approval, setApproval] = useState<ApprovalRequest | null>(null);

  useEffect(() => {
    connectionRef.current = connection;
  }, [connection]);

  /* ---------------- browser mock REPL (unchanged behaviour) ---------------- */

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

  /* --------------------------- terminal lifecycle --------------------------- */

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const term = new Terminal({
      cursorBlink: true,
      fontSize: 13,
      lineHeight: 1.35,
      scrollback: 5000,
      allowProposedApi: true,
      convertEol: false,
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

    const safeFit = () => {
      try {
        fitAddon.fit();
      } catch {
        // The pane can be hidden while fitting; retried when it becomes active.
      }
      const cols = term.cols;
      const rows = term.rows;
      const previous = sizeRef.current;
      if (previous && previous.cols === cols && previous.rows === rows) return;
      sizeRef.current = { cols, rows };
      const sessionId = sessionRef.current;
      if (sessionId) void api.ptyResize(sessionId, cols, rows).catch(() => {});
    };

    const raf = requestAnimationFrame(safeFit);
    const observer = new ResizeObserver(safeFit);
    observer.observe(container);
    window.addEventListener("resize", safeFit);

    let disposed = false;
    let sessionId: string | null = null;
    let unlisteners: UnlistenFn[] = [];
    // Bytes produced before `pty_open` resolved, keyed by session id.
    const early = new Map<string, string[]>();
    let shellExited = false;

    const writeChunk = (payload: string) => term.write(base64ToBytes(payload));

    /* ------------------------- real SSH PTY session ------------------------- */
    const startPty = async () => {
      setStatus("connecting");
      setErrorText(null);
      setHostKey(null);

      const register = async () => {
        unlisteners.push(
          await listen<PtyOutputEvent>("pty:output", ({ payload }) => {
            if (disposed) return;
            if (!sessionId) {
              const queued = early.get(payload.sessionId) ?? [];
              queued.push(payload.data);
              early.set(payload.sessionId, queued);
              return;
            }
            if (payload.sessionId !== sessionId) return;
            writeChunk(payload.data);
          }),
        );
        unlisteners.push(
          await listen<PtyExitEvent>("pty:exit", ({ payload }) => {
            if (disposed || payload.sessionId !== sessionId) return;
            shellExited = true;
            setStatus("closed");
            term.write(
              `\r\n\x1b[38;5;245m[远端 shell 已退出 · code ${payload.exitCode ?? "?"}]\x1b[0m\r\n`,
            );
          }),
        );
        unlisteners.push(
          await listen<PtyClosedEvent>("pty:closed", ({ payload }) => {
            if (disposed || payload.sessionId !== sessionId) return;
            if (shellExited) return;
            setStatus("closed");
            term.write(`\r\n\x1b[38;5;245m[${payload.reason}]\x1b[0m\r\n`);
          }),
        );
      };

      try {
        await register();
        term.writeln(
          `\x1b[38;5;245m正在连接 ${conn.username}@${conn.host}:${conn.port} …\x1b[0m`,
        );
        const opened = await api.ptyOpen(conn.id, term.cols, term.rows);
        if (disposed) {
          void api.ptyClose(opened.sessionId).catch(() => {});
          return;
        }
        sessionId = opened.sessionId;
        sessionRef.current = opened.sessionId;
        setHostKey(opened.hostKey);
        setStatus("live");
        const queued = early.get(sessionId);
        early.clear();
        if (queued) queued.forEach(writeChunk);
        safeFit();
        term.focus();
      } catch (err) {
        if (disposed) return;
        const message = String(err instanceof Error ? err.message : err);
        setStatus("error");
        setErrorText(message);
        term.writeln(`\x1b[31m✖ ${message}\x1b[0m`);
      }
    };

    /* ------------------------- browser demo REPL ------------------------- */
    const startMock = () => {
      term.writeln(
        "\x1b[1;38;5;121mShellX\x1b[0m \x1b[38;5;245mv0.1.0 · 浏览器 Mock 模式\x1b[0m",
      );
      term.writeln(
        `\x1b[38;5;245m连接\x1b[0m ${conn.username}@${conn.host}:${conn.port}  ` +
          `\x1b[38;5;245m环境\x1b[0m ${ENV_LABELS[conn.environment]}  ` +
          `\x1b[38;5;245mMCP\x1b[0m ${MCP_ACCESS_LABELS[conn.mcpAccess]}`,
      );
      term.writeln(
        "\x1b[38;5;245m这是浏览器内的模拟执行；请在桌面版中体验真实 SSH 会话。\x1b[0m",
      );
      promptRef.current();
      setStatus("live");
    };

    term.attachCustomKeyEventHandler((event) => {
      if (event.type !== "keydown") return true;
      const key = event.key.toLowerCase();
      if (event.ctrlKey && event.shiftKey && key === "c") {
        const selection = term.getSelection();
        if (selection) {
          void navigator.clipboard.writeText(selection);
          return false;
        }
        return true;
      }
      if (event.ctrlKey && event.shiftKey && key === "v") {
        void navigator.clipboard.readText().then((text) => {
          if (!text) return;
          if (sessionId) void api.ptyWrite(sessionId, text).catch(() => {});
        });
        return false;
      }
      return true;
    });

    const dataSub = term.onData((data) => {
      if (PTY_CAPABLE) {
        // Everything, including ESC sequences, Ctrl-C and IME output, goes
        // straight to the remote shell: that is what makes it a terminal.
        if (sessionId) void api.ptyWrite(sessionId, data).catch(() => {});
        return;
      }

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

    if (PTY_CAPABLE) {
      void startPty();
    } else {
      startMock();
    }

    return () => {
      disposed = true;
      cancelAnimationFrame(raf);
      observer.disconnect();
      window.removeEventListener("resize", safeFit);
      dataSub.dispose();
      unlisteners.forEach((unlisten) => unlisten());
      unlisteners = [];
      if (sessionId) void api.ptyClose(sessionId).catch(() => {});
      sessionRef.current = null;
      sizeRef.current = null;
      term.dispose();
      terminalRef.current = null;
      fitAddonRef.current = null;
      container.innerHTML = "";
    };
    // `attempt` is a manual reconnect counter; a new value rebuilds the session.
  }, [connection.id, attempt]);

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

  const reconnect = () => {
    setApproval(null);
    setAttempt((value) => value + 1);
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
          {connection.jumpHost && <span className="mini-badge">经跳板机</span>}
        </div>
        <div className="terminal-controls">
          {PTY_CAPABLE && (
            <span className={`pty-status pty-${status}`}>{STATUS_LABELS[status]}</span>
          )}
          {PTY_CAPABLE && hostKey && (
            <span className="muted small" title={`${hostKey.algorithm} ${hostKey.fingerprint}`}>
              {hostKey.known ? "主机密钥已信任" : "首次记录主机密钥"} · {hostKey.fingerprint.slice(0, 22)}…
            </span>
          )}
          {PTY_CAPABLE && (status === "error" || status === "closed") && (
            <button type="button" className="btn btn-ghost btn-sm" onClick={reconnect}>
              重新连接
            </button>
          )}
          {!PTY_CAPABLE && (
            <span className={`mini-badge mcp-${connection.mcpAccess}`}>
              MCP · {MCP_ACCESS_LABELS[connection.mcpAccess]}
            </span>
          )}
          {!PTY_CAPABLE && (
            <button
              type="button"
              className={`mode-switch ${agentMode ? "is-agent" : ""}`}
              onClick={toggleAgentMode}
              title="切换命令来源身份，用于演示 MCP 授权与审批策略"
            >
              <span className="mode-knob" />
              {agentMode ? "AI Agent 模式" : "手动模式"}
            </button>
          )}
        </div>
      </div>

      {errorText && PTY_CAPABLE && (
        <div className="pty-error-banner" title={errorText}>
          {errorText}
        </div>
      )}

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
