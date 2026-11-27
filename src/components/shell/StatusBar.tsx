import type { Connection, McpServerInfo } from "@/lib/types";
import { ENV_LABELS } from "@/lib/types";
import { cn } from "@/lib/utils";
import { ENV_DOT } from "./env";

interface Props {
  activeConnection: Connection | null;
  mcpInfo: McpServerInfo | null;
  connectionCount: number;
  tabCount: number;
  mockMode: boolean;
  onOpenMcpPolicy: () => void;
}

export default function StatusBar({
  activeConnection,
  mcpInfo,
  connectionCount,
  tabCount,
  mockMode,
  onOpenMcpPolicy,
}: Props) {
  const mcpReady =
    mcpInfo?.status === "running" || mcpInfo?.status === "ready";

  return (
    <footer className="flex h-[26px] shrink-0 items-center gap-3 border-t border-border bg-[#08090a] px-3 font-mono text-[9.5px] text-muted-foreground">
      <span className="flex min-w-0 items-center gap-1.5">
        {activeConnection ? (
          <>
            <span
              className={cn(
                "size-1.5 shrink-0 rounded-full",
                ENV_DOT[activeConnection.environment],
              )}
            />
            <span className="shrink-0 text-foreground/80">
              {ENV_LABELS[activeConnection.environment]}
            </span>
            <span className="text-muted-foreground/50">·</span>
            <span className="truncate text-foreground/80">
              {activeConnection.alias}
            </span>
          </>
        ) : (
          <span>未连接</span>
        )}
      </span>

      <button
        type="button"
        onClick={onOpenMcpPolicy}
        title="MCP 权限设置"
        className="flex shrink-0 items-center gap-1.5 rounded px-1 py-0.5 transition-colors hover:bg-white/5 hover:text-foreground"
      >
        <span
          className={cn(
            "size-1.5 rounded-full",
            mcpReady ? "bg-emerald-400" : "bg-white/20",
          )}
        />
        <span className="text-primary/90">MCP</span>
        <span>
          {mcpInfo
            ? `${mcpInfo.enabledTools}/${mcpInfo.tools.length}`
            : "…"}
        </span>
        {mcpInfo && (
          <span className="text-muted-foreground/60">
            · {mcpInfo.authorizedConnections} 台授权
          </span>
        )}
      </button>

      <span className="flex-1" />

      {mockMode && (
        <span className="rounded border border-amber-400/25 bg-amber-400/10 px-1 text-amber-300">
          Mock
        </span>
      )}
      <span className="shrink-0">
        {connectionCount} 连接 · {tabCount} 会话
      </span>
      {activeConnection && (
        <span className="hidden shrink-0 xl:inline">
          {activeConnection.encoding} · {activeConnection.term}
        </span>
      )}
    </footer>
  );
}
