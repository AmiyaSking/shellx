import { useMemo } from "react";
import {
  ArrowLeftRight,
  FolderClosed,
  History,
  PanelLeft,
  Plus,
  Server,
  Settings,
  ShieldCheck,
  SquareTerminal,
  Upload,
} from "lucide-react";
import {
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandShortcut,
} from "@/components/ui/command";
import type { TerminalTab } from "@/components/TabBar";
import type { Connection } from "@/lib/types";
import { cn } from "@/lib/utils";
import { ENV_DOT } from "./env";

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  connections: Connection[];
  tabs: TerminalTab[];
  activeTabId: string | null;
  onOpenConnection: (connection: Connection) => void;
  onSelectTab: (tabId: string) => void;
  onNewConnection: () => void;
  onImport: () => void;
  onOpenFiles: () => void;
  onOpenForwards: () => void;
  onOpenMcpPolicy: () => void;
  onOpenMcpConfig: () => void;
  onOpenAudit: () => void;
  onOpenSettings: () => void;
  onToggleExplorer: () => void;
}

export default function CommandPalette({
  open,
  onOpenChange,
  connections,
  tabs,
  activeTabId,
  onOpenConnection,
  onSelectTab,
  onNewConnection,
  onImport,
  onOpenFiles,
  onOpenForwards,
  onOpenMcpPolicy,
  onOpenMcpConfig,
  onOpenAudit,
  onOpenSettings,
  onToggleExplorer,
}: Props) {
  const connectionById = useMemo(
    () => new Map(connections.map((connection) => [connection.id, connection])),
    [connections],
  );

  const run = (action: () => void) => {
    onOpenChange(false);
    action();
  };

  return (
    <CommandDialog
      open={open}
      onOpenChange={onOpenChange}
      title="命令面板"
      description="搜索连接、标签页或命令"
      className="top-[18%] sm:max-w-xl"
    >
      <CommandInput placeholder="搜索连接、标签页或命令…" autoFocus />
      <CommandList className="max-h-[420px]">
        <CommandEmpty>没有匹配项</CommandEmpty>

        {connections.length > 0 && (
          <CommandGroup heading="连接">
            {connections.map((connection) => (
              <CommandItem
                key={connection.id}
                value={`connection-${connection.id}`}
                keywords={[
                  connection.alias,
                  connection.host,
                  connection.username,
                  connection.group,
                  ...connection.tags,
                ]}
                onSelect={() => run(() => onOpenConnection(connection))}
              >
                <Server />
                <span className="truncate">{connection.alias}</span>
                <span className="ml-auto flex items-center gap-1.5 font-mono text-[10px] text-muted-foreground">
                  <span
                    className={cn(
                      "size-1.5 rounded-full",
                      ENV_DOT[connection.environment],
                    )}
                  />
                  {connection.username}@{connection.host}
                </span>
              </CommandItem>
            ))}
          </CommandGroup>
        )}

        {tabs.length > 0 && (
          <CommandGroup heading="标签页">
            {tabs.map((tab) => {
              const pane =
                tab.panes.find((item) => item.id === tab.activePaneId) ??
                tab.panes[0];
              const connection = pane
                ? connectionById.get(pane.connectionId)
                : undefined;
              const title =
                tab.customTitle?.trim() ||
                tab.title?.trim() ||
                connection?.alias ||
                "未知连接";
              return (
                <CommandItem
                  key={tab.id}
                  value={`tab-${tab.id}`}
                  keywords={[title, connection?.host ?? ""]}
                  onSelect={() => run(() => onSelectTab(tab.id))}
                >
                  <SquareTerminal />
                  <span className="truncate">{title}</span>
                  {tab.id === activeTabId && (
                    <span className="ml-auto text-[10px] text-primary">
                      当前
                    </span>
                  )}
                </CommandItem>
              );
            })}
          </CommandGroup>
        )}

        <CommandGroup heading="操作">
          <CommandItem
            value="new-connection"
            keywords={["新建", "连接", "create"]}
            onSelect={() => run(onNewConnection)}
          >
            <Plus />
            新建连接
          </CommandItem>
          <CommandItem
            value="import-ssh-config"
            keywords={["导入", "ssh", "config"]}
            onSelect={() => run(onImport)}
          >
            <Upload />
            从 SSH config 导入
          </CommandItem>
          <CommandItem
            value="open-sftp"
            keywords={["文件", "sftp", "上传", "下载"]}
            onSelect={() => run(onOpenFiles)}
          >
            <FolderClosed />
            SFTP 文件
          </CommandItem>
          <CommandItem
            value="open-forwards"
            keywords={["转发", "隧道", "tunnel"]}
            onSelect={() => run(onOpenForwards)}
          >
            <ArrowLeftRight />
            端口转发
          </CommandItem>
          <CommandItem
            value="mcp-policy"
            keywords={["mcp", "权限", "授权", "policy"]}
            onSelect={() => run(onOpenMcpPolicy)}
          >
            <ShieldCheck />
            MCP 权限设置
          </CommandItem>
          <CommandItem
            value="mcp-client-config"
            keywords={["mcp", "客户端", "配置", "client"]}
            onSelect={() => run(onOpenMcpConfig)}
          >
            <ShieldCheck />
            MCP 客户端配置
          </CommandItem>
          <CommandItem
            value="open-audit"
            keywords={["审计", "日志", "audit"]}
            onSelect={() => run(onOpenAudit)}
          >
            <History />
            审计日志
          </CommandItem>
          <CommandItem
            value="open-settings"
            keywords={["设置", "settings", "偏好"]}
            onSelect={() => run(onOpenSettings)}
          >
            <Settings />
            设置
          </CommandItem>
          <CommandItem
            value="toggle-explorer"
            keywords={["侧栏", "连接面板", "收起"]}
            onSelect={() => run(onToggleExplorer)}
          >
            <PanelLeft />
            收起 / 展开连接面板
            <CommandShortcut>Ctrl+Shift+B</CommandShortcut>
          </CommandItem>
        </CommandGroup>
      </CommandList>
    </CommandDialog>
  );
}
