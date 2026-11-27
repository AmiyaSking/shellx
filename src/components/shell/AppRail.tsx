import type { ReactNode } from "react";
import {
  ArrowLeftRight,
  FolderClosed,
  History,
  Server,
  Settings,
  ShieldCheck,
  Settings2,
} from "lucide-react";
import { cn } from "@/lib/utils";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import type { McpServerInfo } from "@/lib/types";

interface Props {
  explorerOpen: boolean;
  sftpOpen: boolean;
  forwardOpen: boolean;
  auditOpen: boolean;
  settingsOpen: boolean;
  mcpPolicyOpen: boolean;
  mcpConfigOpen: boolean;
  mcpInfo: McpServerInfo | null;
  onToggleExplorer: () => void;
  onOpenFiles: () => void;
  onOpenForwards: () => void;
  onOpenAudit: () => void;
  onOpenSettings: () => void;
  onOpenMcpPolicy: () => void;
  onOpenMcpConfig: () => void;
}

function RailButton({
  label,
  active,
  onClick,
  children,
}: {
  label: string;
  active?: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          type="button"
          aria-label={label}
          onClick={onClick}
          className={cn(
            "relative grid size-8 place-items-center rounded-lg text-muted-foreground transition-colors hover:bg-white/5 hover:text-foreground",
            active &&
              "bg-primary/15 text-primary hover:bg-primary/15 hover:text-primary",
          )}
        >
          {active && (
            <span className="absolute -left-2 top-2 bottom-2 w-0.5 rounded-r-full bg-primary" />
          )}
          {children}
        </button>
      </TooltipTrigger>
      <TooltipContent side="right">{label}</TooltipContent>
    </Tooltip>
  );
}

export default function AppRail({
  explorerOpen,
  sftpOpen,
  forwardOpen,
  auditOpen,
  settingsOpen,
  mcpPolicyOpen,
  mcpConfigOpen,
  mcpInfo,
  onToggleExplorer,
  onOpenFiles,
  onOpenForwards,
  onOpenAudit,
  onOpenSettings,
  onOpenMcpPolicy,
  onOpenMcpConfig,
}: Props) {
  const mcpActive = mcpPolicyOpen || mcpConfigOpen;
  const mcpReady = mcpInfo?.status === "running" || mcpInfo?.status === "ready";

  return (
    <nav
      aria-label="主导航"
      className="flex h-full w-12 shrink-0 flex-col items-center gap-1 border-r border-border bg-[#07080a] py-2.5"
      data-tauri-drag-region="deep"
    >
      <div
        className="mb-2 grid size-7 place-items-center rounded-lg bg-gradient-to-br from-violet-500 to-indigo-500 text-[10px] font-bold text-white shadow-[inset_0_1px_0_rgba(255,255,255,.25)]"
        title="ShellX"
      >
        SX
      </div>

      <RailButton
        label="连接"
        active={explorerOpen}
        onClick={onToggleExplorer}
      >
        <Server className="size-4" />
      </RailButton>

      <RailButton
        label="SFTP 文件"
        active={sftpOpen}
        onClick={onOpenFiles}
      >
        <FolderClosed className="size-4" />
      </RailButton>

      <RailButton
        label="端口转发"
        active={forwardOpen}
        onClick={onOpenForwards}
      >
        <ArrowLeftRight className="size-4" />
      </RailButton>

      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            aria-label="MCP"
            title="MCP 权限与客户端配置"
            className={cn(
              "relative grid size-8 place-items-center rounded-lg text-muted-foreground transition-colors hover:bg-white/5 hover:text-foreground",
              mcpActive &&
                "bg-primary/15 text-primary hover:bg-primary/15 hover:text-primary",
            )}
          >
            <ShieldCheck className="size-4" />
            {mcpReady && (
              <span className="absolute right-1.5 bottom-1.5 size-1.5 rounded-full bg-emerald-400 ring-2 ring-[#07080a]" />
            )}
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent side="right" align="start" className="w-60">
          <DropdownMenuLabel className="text-[10px] font-normal text-muted-foreground">
            本地 MCP
            {mcpInfo
              ? ` · ${mcpInfo.enabledTools}/${mcpInfo.tools.length} 工具 · ${mcpInfo.authorizedConnections} 台授权`
              : " · 状态读取中"}
          </DropdownMenuLabel>
          <DropdownMenuSeparator />
          <DropdownMenuItem onSelect={onOpenMcpPolicy}>
            <ShieldCheck />
            MCP 权限设置
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={onOpenMcpConfig}>
            <Settings2 />
            客户端配置
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>

      <RailButton label="审计日志" active={auditOpen} onClick={onOpenAudit}>
        <History className="size-4" />
      </RailButton>

      <div className="flex-1" />

      <RailButton label="设置" active={settingsOpen} onClick={onOpenSettings}>
        <Settings className="size-4" />
      </RailButton>
    </nav>
  );
}
