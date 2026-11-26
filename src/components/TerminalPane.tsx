import { useCallback, useEffect, useRef, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { WebglAddon } from "@xterm/addon-webgl";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Terminal } from "@xterm/xterm";
import { api, isTauri } from "../lib/api";
import { readClipboardText, writeClipboardText } from "../lib/clipboard";
import {
  DEFAULT_SETTINGS,
  loadSettings,
  saveSettings,
  SETTINGS_EVENT,
  type AppSettings,
} from "../lib/settings";
import {
  ENV_LABELS,
  MCP_ACCESS_LABELS,
  type ApprovalRequest,
  type CommandOutcome,
  type Connection,
  type HostKey,
  type HostKeyPrompt,
  type HostKeyTrust,
  type PtyClosedEvent,
  type PtyExitEvent,
  type PtyOpenError,
  type PtyOutputEvent,
} from "../lib/types";
import ApprovalDialog from "./ApprovalDialog";
import HostKeyDialog from "./HostKeyDialog";
import PasteConfirmDialog from "./PasteConfirmDialog";

interface Props {
  connection: Connection;
  visible: boolean;
  focused: boolean;
  canSplit?: boolean;
  onFocus?: () => void;
  onTitleChange?: (title: string) => void;
  onBell?: () => void;
  onSplitRow?: () => void;
  onSplitColumn?: () => void;
  onClosePane?: () => void;
}

function openExternalUrl(uri: string) {
  if (!/^(https?:\/\/|mailto:)/i.test(uri)) return;
  if (isTauri()) {
    void openUrl(uri).catch(() => {});
  } else {
    window.open(uri, "_blank", "noopener,noreferrer");
  }
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

function isPtyOpenError(value: unknown): value is PtyOpenError {
  return (
    typeof value === "object" &&
    value !== null &&
    "kind" in value &&
    "message" in value
  );
}

function newRequestId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID();
  }
  return `req_${Date.now().toString(36)}_${Math.random().toString(36).slice(2)}`;
}

/** Raw bytes arrive base64-encoded so a split UTF-8 character stays intact. */
function base64ToBytes(data: string): Uint8Array {
  const binary = atob(data);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

export default function TerminalPane({
  connection,
  visible,
  focused,
  canSplit = false,
  onFocus,
  onTitleChange,
  onBell,
  onSplitRow,
  onSplitColumn,
  onClosePane,
}: Props) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const searchAddonRef = useRef<SearchAddon | null>(null);
  const searchInputRef = useRef<HTMLInputElement | null>(null);
  const onTitleChangeRef = useRef<Props["onTitleChange"]>(onTitleChange);
  const onBellRef = useRef<Props["onBell"]>(onBell);
  const settingsRef = useRef<AppSettings>(loadSettings());
  const requestPasteRef = useRef<(text: string) => void>(() => {});
  const sessionRef = useRef<string | null>(null);
  const connectionRef = useRef(connection);
  const sizeRef = useRef<{ cols: number; rows: number } | null>(null);
  const trustedHostKeysRef = useRef<HostKeyTrust[]>([]);
  const retryOpenRef = useRef<() => void>(() => {});
  const cancelRequestRef = useRef<string | null>(null);
  const cancelledRef = useRef(false);

  // Browser-demo REPL state (kept so `npm run dev` still shows something).
  const inputRef = useRef("");
  const agentModeRef = useRef(false);
  const promptRef = useRef<() => void>(() => {});
  const submitRef = useRef<(command: string) => void>(() => {});

  const [status, setStatus] = useState<PtyStatus>("idle");
  const [hostKey, setHostKey] = useState<HostKey | null>(null);
  const [errorText, setErrorText] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  const [pendingHostKey, setPendingHostKey] = useState<HostKeyPrompt | null>(null);
  const [hostKeyIssue, setHostKeyIssue] = useState<HostKeyPrompt | null>(null);
  const [searchOpen, setSearchOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [pendingPaste, setPendingPaste] = useState<string | null>(null);
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number } | null>(
    null,
  );
  const [hasSelection, setHasSelection] = useState(false);

  const [agentMode, setAgentMode] = useState(false);
  const [approval, setApproval] = useState<ApprovalRequest | null>(null);

  useEffect(() => {
    connectionRef.current = connection;
  }, [connection]);

  useEffect(() => {
    onTitleChangeRef.current = onTitleChange;
  }, [onTitleChange]);

  useEffect(() => {
    onBellRef.current = onBell;
  }, [onBell]);

  useEffect(() => {
    if (!searchOpen) return;
    const timer = window.setTimeout(() => {
      searchInputRef.current?.focus();
      searchInputRef.current?.select();
    }, 20);
    return () => window.clearTimeout(timer);
  }, [searchOpen]);

  const findInTerminal = useCallback(
    (direction: "next" | "previous", value?: string) => {
      const addon = searchAddonRef.current;
      if (!addon) return;
      const query = value ?? searchQuery;
      if (!query) {
        addon.clearDecorations();
        return;
      }
      const options = { caseSensitive: false, incremental: true };
      if (direction === "next") {
        addon.findNext(query, options);
      } else {
        addon.findPrevious(query, options);
      }
    },
    [searchQuery],
  );

  const closeSearch = useCallback(() => {
    setSearchOpen(false);
    searchAddonRef.current?.clearDecorations();
    terminalRef.current?.focus();
  }, []);

  useEffect(() => {
    const handler = (event: Event) => {
      const next = (event as CustomEvent<AppSettings>).detail;
      settingsRef.current = next;
      const term = terminalRef.current;
      if (!term) return;
      term.options.fontSize = next.fontSize;
      term.options.scrollback = next.scrollback;
      term.options.cursorBlink = next.cursorBlink;
      try {
        fitAddonRef.current?.fit();
      } catch {
        // Hidden pane; the active-tab fit will catch up.
      }
      const sessionId = sessionRef.current;
      if (sessionId) {
        void api.ptyResize(sessionId, term.cols, term.rows).catch(() => {});
      }
    };
    window.addEventListener(SETTINGS_EVENT, handler);
    return () => window.removeEventListener(SETTINGS_EVENT, handler);
  }, []);

  const pasteText = useCallback((text: string) => {
    if (!text) return;
    const term = terminalRef.current;
    if (!term) return;
    if (PTY_CAPABLE && sessionRef.current) {
      void api.ptyWrite(sessionRef.current, text).catch(() => {});
      return;
    }
    if (!PTY_CAPABLE) {
      inputRef.current += text;
      term.write(text);
    }
  }, []);

  const requestPaste = useCallback(
    (text: string) => {
      if (!text) return;
      const settings = settingsRef.current;
      if (
        settings.pasteWarning &&
        (/[\r\n]/.test(text) || text.length > 2000)
      ) {
        setPendingPaste(text);
        return;
      }
      pasteText(text);
    },
    [pasteText],
  );

  useEffect(() => {
    requestPasteRef.current = requestPaste;
  }, [requestPaste]);

  useEffect(() => {
    if (!contextMenu) return;
    const close = () => setContextMenu(null);
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setContextMenu(null);
    };
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", onKey);
    };
  }, [contextMenu]);

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
      cursorBlink: settingsRef.current.cursorBlink,
      fontSize: settingsRef.current.fontSize,
      lineHeight: 1.35,
      scrollback: settingsRef.current.scrollback,
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

    const searchAddon = new SearchAddon();
    term.loadAddon(searchAddon);
    searchAddonRef.current = searchAddon;
    term.loadAddon(new Unicode11Addon());
    term.unicode.activeVersion = "11";
    term.loadAddon(new WebLinksAddon((_event, uri) => openExternalUrl(uri)));
    try {
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => webgl.dispose());
      term.loadAddon(webgl);
    } catch {
      // WebGL is optional; xterm falls back to the DOM renderer.
    }

    const titleSub = term.onTitleChange((title) => {
      const cleaned = title.trim().slice(0, 80);
      if (cleaned) onTitleChangeRef.current?.(cleaned);
    });
    const bellSub = term.onBell(() => onBellRef.current?.());

    let selectionTimer: number | undefined;
    const selectionSub = term.onSelectionChange(() => {
      const selection = term.getSelection();
      setHasSelection(Boolean(selection));
      if (!settingsRef.current.copyOnSelect || !selection) return;
      window.clearTimeout(selectionTimer);
      selectionTimer = window.setTimeout(() => {
        void writeClipboardText(selection);
      }, 120);
    });

    // xterm routes browser paste through its hidden textarea; intercept it so
    // multi-line pastes can be reviewed before they reach the remote shell.
    const onNativePaste = (event: ClipboardEvent) => {
      const text = event.clipboardData?.getData("text/plain") ?? "";
      if (!text) return;
      event.preventDefault();
      event.stopPropagation();
      requestPasteRef.current(text);
    };
    container.addEventListener("paste", onNativePaste, true);

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

    const applyFontSize = (size: number) => {
      const clamped = Math.max(10, Math.min(24, size));
      // saveSettings broadcasts to every terminal, including this one.
      saveSettings({ ...settingsRef.current, fontSize: clamped });
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
    let listenersReady = false;
    const register = async () => {
      if (listenersReady) return;
      listenersReady = true;
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
          // Drop the backend entry so active-count and audit stay truthful.
          void api.ptyClose(payload.sessionId).catch(() => {});
        }),
      );
      unlisteners.push(
        await listen<PtyClosedEvent>("pty:closed", ({ payload }) => {
          if (disposed || payload.sessionId !== sessionId) return;
          if (shellExited) return;
          setStatus("closed");
          term.write(`\r\n\x1b[38;5;245m[${payload.reason}]\x1b[0m\r\n`);
          void api.ptyClose(payload.sessionId).catch(() => {});
        }),
      );
    };

    const startPty = async () => {
      setStatus("connecting");
      setErrorText(null);
      setHostKey(null);
      setHostKeyIssue(null);
      cancelledRef.current = false;

      try {
        await register();
        term.writeln(
          `\x1b[38;5;245m正在连接 ${conn.username}@${conn.host}:${conn.port} …\x1b[0m`,
        );
        const requestId = newRequestId();
        cancelRequestRef.current = requestId;
        const opened = await api.ptyOpen(
          conn.id,
          term.cols,
          term.rows,
          trustedHostKeysRef.current,
          requestId,
        );
        cancelRequestRef.current = null;
        if (disposed || cancelledRef.current) {
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
        cancelRequestRef.current = null;
        if (disposed) return;
        if (cancelledRef.current) {
          cancelledRef.current = false;
          setStatus("idle");
          term.writeln("\x1b[33m[连接已取消]\x1b[0m");
          return;
        }
        if (isPtyOpenError(err) && err.kind === "host_key_unknown" && err.hostKey) {
          setStatus("idle");
          setPendingHostKey(err.hostKey);
          term.writeln(
            `\x1b[33m等待确认主机密钥：${err.hostKey.fingerprint}\x1b[0m`,
          );
          return;
        }
        if (isPtyOpenError(err) && err.kind === "host_key_changed" && err.hostKey) {
          setStatus("error");
          setHostKeyIssue(err.hostKey);
          setErrorText(err.message);
          term.writeln(`\x1b[31m✖ ${err.message}\x1b[0m`);
          return;
        }
        const message = isPtyOpenError(err)
          ? err.message
          : err instanceof Error
            ? err.message
            : String(err);
        setStatus("error");
        setErrorText(message);
        term.writeln(`\x1b[31m✖ ${message}\x1b[0m`);
      }
    };
    retryOpenRef.current = () => {
      void startPty();
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
      const mod = event.ctrlKey || event.metaKey;

      if (mod && event.shiftKey && key === "f") {
        event.preventDefault();
        setSearchOpen(true);
        return false;
      }
      if (mod && event.shiftKey && (key === "+" || key === "=")) {
        event.preventDefault();
        applyFontSize((term.options.fontSize ?? DEFAULT_SETTINGS.fontSize) + 1);
        return false;
      }
      if (mod && event.shiftKey && key === "-") {
        event.preventDefault();
        applyFontSize((term.options.fontSize ?? DEFAULT_SETTINGS.fontSize) - 1);
        return false;
      }
      if (mod && event.shiftKey && key === "0") {
        event.preventDefault();
        applyFontSize(DEFAULT_SETTINGS.fontSize);
        return false;
      }

      if (mod && event.shiftKey && key === "c") {
        const selection = term.getSelection();
        if (selection) {
          void writeClipboardText(selection);
          return false;
        }
        return true;
      }
      if (mod && event.shiftKey && key === "v") {
        void readClipboardText().then((text) => requestPasteRef.current(text));
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
      titleSub.dispose();
      bellSub.dispose();
      selectionSub.dispose();
      window.clearTimeout(selectionTimer);
      container.removeEventListener("paste", onNativePaste, true);
      searchAddonRef.current = null;
      unlisteners.forEach((unlisten) => unlisten());
      unlisteners = [];
      retryOpenRef.current = () => {};
      if (cancelRequestRef.current) {
        void api.ptyCancel(cancelRequestRef.current).catch(() => {});
        cancelRequestRef.current = null;
      }
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
    if (!visible) return;
    const timer = window.setTimeout(() => {
      try {
        fitAddonRef.current?.fit();
      } catch {
        // ignore
      }
      if (focused) terminalRef.current?.focus();
    }, 30);
    return () => window.clearTimeout(timer);
  }, [focused, visible]);

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
    setPendingHostKey(null);
    setHostKeyIssue(null);
    setErrorText(null);
    setAttempt((value) => value + 1);
  };

  const cancelConnect = () => {
    cancelledRef.current = true;
    const requestId = cancelRequestRef.current;
    if (requestId) {
      void api.ptyCancel(requestId).catch(() => {});
    }
    setStatus("idle");
    terminalRef.current?.writeln("\r\n\x1b[33m已请求取消连接…\x1b[0m");
  };

  const trustPendingHostKey = () => {
    const prompt = pendingHostKey;
    if (!prompt) return;
    const sameHost = (entry: HostKeyTrust) =>
      entry.host.toLowerCase() === prompt.host.toLowerCase() && entry.port === prompt.port;
    trustedHostKeysRef.current = [
      ...trustedHostKeysRef.current.filter((entry) => !sameHost(entry)),
      { host: prompt.host, port: prompt.port, fingerprint: prompt.fingerprint },
    ];
    setPendingHostKey(null);
    retryOpenRef.current();
  };

  const dismissPendingHostKey = () => {
    setPendingHostKey(null);
    setStatus("idle");
    terminalRef.current?.writeln("\r\n\x1b[33m已取消连接（未信任该主机密钥）\x1b[0m");
  };

  const forgetHostKeyAndRetry = async () => {
    const issue = hostKeyIssue;
    if (!issue) return;
    try {
      await api.forgetHostKey(issue.host, issue.port);
      trustedHostKeysRef.current = trustedHostKeysRef.current.filter(
        (entry) =>
          !(
            entry.host.toLowerCase() === issue.host.toLowerCase() &&
            entry.port === issue.port
          ),
      );
      setHostKeyIssue(null);
      setErrorText(null);
      retryOpenRef.current();
    } catch (err) {
      setErrorText(String(err));
    }
  };

  const copySelection = () => {
    const selection = terminalRef.current?.getSelection();
    if (selection) void writeClipboardText(selection);
    setContextMenu(null);
  };

  const pasteFromClipboard = () => {
    setContextMenu(null);
    void readClipboardText().then((text) => {
      if (text) requestPaste(text);
    });
  };

  const selectAllTerminal = () => {
    terminalRef.current?.selectAll();
    setContextMenu(null);
  };

  const clearTerminal = () => {
    terminalRef.current?.clear();
    setContextMenu(null);
  };

  const openSearchFromMenu = () => {
    setContextMenu(null);
    setSearchOpen(true);
  };

  return (
    <div
      className={`terminal-pane ${focused ? "is-focused" : ""}`}
      onMouseDownCapture={() => onFocus?.()}
    >
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
          <button
            type="button"
            className="btn btn-ghost btn-sm"
            title="搜索终端输出（Ctrl+Shift+F）"
            onClick={() => setSearchOpen(true)}
          >
            搜索
          </button>
          {onSplitRow && (
            <button
              type="button"
              className="icon-btn"
              disabled={!canSplit}
              title="左右分屏（Ctrl+Shift+D）"
              onClick={onSplitRow}
            >
              ◫
            </button>
          )}
          {onSplitColumn && (
            <button
              type="button"
              className="icon-btn"
              disabled={!canSplit}
              title="上下分屏（Ctrl+Shift+E）"
              onClick={onSplitColumn}
            >
              ⬒
            </button>
          )}
          {onClosePane && (
            <button
              type="button"
              className="icon-btn"
              title="关闭窗格（Ctrl+Shift+W）"
              onClick={onClosePane}
            >
              ×
            </button>
          )}
          {PTY_CAPABLE && (
            <span className={`pty-status pty-${status}`}>{STATUS_LABELS[status]}</span>
          )}
          {PTY_CAPABLE && hostKey && (
            <span className="muted small" title={`${hostKey.algorithm} ${hostKey.fingerprint}`}>
              {hostKey.known ? "主机密钥已信任" : "首次记录主机密钥"} · {hostKey.fingerprint.slice(0, 22)}…
            </span>
          )}
          {PTY_CAPABLE && status === "connecting" && (
            <button type="button" className="btn btn-ghost btn-sm" onClick={cancelConnect}>
              取消
            </button>
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

      {searchOpen && (
        <div className="terminal-search">
          <input
            ref={searchInputRef}
            className="terminal-search-input"
            value={searchQuery}
            placeholder="搜索终端输出…"
            spellCheck={false}
            onChange={(event) => {
              setSearchQuery(event.target.value);
              findInTerminal("next", event.target.value);
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                findInTerminal(event.shiftKey ? "previous" : "next");
              } else if (event.key === "Escape") {
                event.preventDefault();
                closeSearch();
              }
            }}
          />
          <button
            type="button"
            className="icon-btn"
            title="上一个（Shift+Enter）"
            onClick={() => findInTerminal("previous")}
          >
            ↑
          </button>
          <button
            type="button"
            className="icon-btn"
            title="下一个（Enter）"
            onClick={() => findInTerminal("next")}
          >
            ↓
          </button>
          <button
            type="button"
            className="icon-btn"
            title="关闭搜索（Esc）"
            onClick={closeSearch}
          >
            ×
          </button>
        </div>
      )}

      {errorText && PTY_CAPABLE && (
        <div className="pty-error-banner" title={errorText}>
          <span>{errorText}</span>
          {hostKeyIssue && (
            <button
              type="button"
              className="btn btn-ghost btn-sm"
              onClick={() => void forgetHostKeyAndRetry()}
            >
              移除该主机密钥并重试
            </button>
          )}
        </div>
      )}

      <div
        className="xterm-host"
        ref={containerRef}
        onContextMenu={(event) => {
          event.preventDefault();
          setContextMenu({
            x: Math.min(event.clientX, window.innerWidth - 200),
            y: Math.min(event.clientY, window.innerHeight - 230),
          });
        }}
      />

      {contextMenu && (
        <div
          className="terminal-context-menu"
          style={{ left: contextMenu.x, top: contextMenu.y }}
          onMouseDown={(event) => event.stopPropagation()}
        >
          <button
            type="button"
            disabled={!hasSelection}
            onClick={copySelection}
          >
            复制
          </button>
          <button type="button" onClick={pasteFromClipboard}>
            粘贴
          </button>
          <button type="button" onClick={selectAllTerminal}>
            全选
          </button>
          <button type="button" onClick={openSearchFromMenu}>
            搜索
          </button>
          <button type="button" onClick={clearTerminal}>
            清屏
          </button>
        </div>
      )}

      {pendingPaste !== null && (
        <PasteConfirmDialog
          text={pendingPaste}
          onCancel={() => {
            setPendingPaste(null);
            terminalRef.current?.focus();
          }}
          onConfirm={() => {
            const text = pendingPaste;
            setPendingPaste(null);
            pasteText(text);
            terminalRef.current?.focus();
          }}
        />
      )}

      {pendingHostKey && (
        <HostKeyDialog
          hostKey={pendingHostKey}
          onTrust={trustPendingHostKey}
          onCancel={dismissPendingHostKey}
        />
      )}

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
