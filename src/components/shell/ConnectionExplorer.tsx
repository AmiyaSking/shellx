import { useMemo, useState, type ReactNode } from "react";
import {
  MoreHorizontal,
  PanelLeftClose,
  Pencil,
  Plus,
  Search,
  ShieldCheck,
  Upload,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  ENV_LABELS,
  MCP_ACCESS_LABELS,
  type Connection,
} from "@/lib/types";
import { cn } from "@/lib/utils";
import { ENV_BADGE, ENV_DOT } from "./env";

interface Props {
  open: boolean;
  connections: Connection[];
  activeConnectionId: string | null;
  loading: boolean;
  onOpen: (connection: Connection) => void;
  onNew: () => void;
  onImport: () => void;
  onEdit: (connection: Connection) => void;
  onCollapse: () => void;
}

function IconAction({
  label,
  onClick,
  children,
}: {
  label: string;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          aria-label={label}
          onClick={onClick}
          className="text-muted-foreground hover:text-foreground"
        >
          {children}
        </Button>
      </TooltipTrigger>
      <TooltipContent side="bottom">{label}</TooltipContent>
    </Tooltip>
  );
}

export default function ConnectionExplorer({
  open,
  connections,
  activeConnectionId,
  loading,
  onOpen,
  onNew,
  onImport,
  onEdit,
  onCollapse,
}: Props) {
  const [query, setQuery] = useState("");

  const groups = useMemo(() => {
    const keyword = query.trim().toLowerCase();
    const filtered = connections.filter((connection) => {
      if (!keyword) return true;
      return [
        connection.alias,
        connection.host,
        connection.group,
        connection.username,
        ...connection.tags,
      ]
        .join(" ")
        .toLowerCase()
        .includes(keyword);
    });
    const map = new Map<string, Connection[]>();
    for (const connection of filtered) {
      const key = connection.group.trim() || "未分组";
      const bucket = map.get(key) ?? [];
      bucket.push(connection);
      map.set(key, bucket);
    }
    return [...map.entries()].sort((a, b) =>
      a[0].localeCompare(b[0], "zh-CN"),
    );
  }, [connections, query]);

  return (
    <aside
      aria-hidden={!open}
      className={cn(
        "h-full shrink-0 overflow-hidden border-r border-border bg-sidebar transition-[width,border-color] duration-300 ease-[cubic-bezier(0.22,0.61,0.36,1)] will-change-[width]",
        open ? "w-[264px]" : "pointer-events-none w-0 border-r-transparent",
      )}
    >
      <div className="flex h-full w-[264px] flex-col" inert={!open}>
        <div
          className="flex h-12 shrink-0 select-none items-center justify-between px-3"
          data-tauri-drag-region="deep"
        >
          <div className="text-xs font-semibold tracking-wide text-foreground">
            连接
            <span className="ml-1.5 font-mono text-[10px] font-normal text-muted-foreground">
              {connections.length}
            </span>
          </div>
          <div className="flex items-center gap-0.5">
            <IconAction label="从 ~/.ssh/config 导入" onClick={onImport}>
              <Upload className="size-3.5" />
            </IconAction>
            <IconAction label="新建连接" onClick={onNew}>
              <Plus className="size-3.5" />
            </IconAction>
          </div>
        </div>

        <div className="relative shrink-0 px-3 pb-2">
          <Search className="pointer-events-none absolute left-5 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="搜索别名 / 主机 / 标签"
            spellCheck={false}
            className="h-8 border-border/70 bg-card pl-7 text-[11.5px] placeholder:text-muted-foreground/70 focus-visible:ring-1"
          />
        </div>

        <ScrollArea className="min-h-0 flex-1">
          <div className="px-2 pb-3">
            {loading && (
              <div className="px-2 py-4 text-[11px] text-muted-foreground">
                加载中…
              </div>
            )}

            {!loading && connections.length === 0 && (
              <div className="px-2 py-6 text-[11px] leading-relaxed text-muted-foreground">
                <p className="m-0 text-foreground">还没有服务器连接</p>
                <p className="mb-0 mt-1">
                  新建连接后即可在此打开终端；对 MCP 的授权默认关闭。
                </p>
              </div>
            )}

            {!loading && connections.length > 0 && groups.length === 0 && (
              <div className="px-2 py-4 text-[11px] text-muted-foreground">
                没有匹配的连接
              </div>
            )}

            {groups.map(([group, items]) => (
              <div key={group}>
                <div className="flex items-center justify-between px-2 pb-1 pt-3 text-[10px] font-semibold uppercase tracking-[0.07em] text-muted-foreground/80">
                  <span>{group}</span>
                  <span className="font-mono font-normal tracking-normal">
                    {items.length}
                  </span>
                </div>
                {items.map((connection) => {
                  const active = connection.id === activeConnectionId;
                  return (
                    <div
                      key={connection.id}
                      role="button"
                      tabIndex={0}
                      onClick={() => onOpen(connection)}
                      onKeyDown={(event) => {
                        if (event.key === "Enter" || event.key === " ") {
                          event.preventDefault();
                          onOpen(connection);
                        }
                      }}
                      className={cn(
                        "group relative flex w-full cursor-pointer items-start gap-2 rounded-lg px-2 py-2 text-left transition-colors hover:bg-white/[0.04] focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring",
                        active &&
                          "bg-[#16181e] shadow-[inset_2px_0_0_var(--primary)] hover:bg-[#16181e]",
                      )}
                    >
                      <span
                        className={cn(
                          "mt-[7px] size-1.5 shrink-0 rounded-full",
                          ENV_DOT[connection.environment],
                        )}
                      />
                      <span className="min-w-0 flex-1">
                        <span className="flex items-center gap-1.5">
                          <span className="truncate text-[12.5px] font-medium text-foreground">
                            {connection.alias}
                          </span>
                          <span
                            className={cn(
                              "shrink-0 rounded px-1 py-px text-[9px] font-semibold",
                              ENV_BADGE[connection.environment],
                            )}
                          >
                            {ENV_LABELS[connection.environment]}
                          </span>
                          <span className="ml-auto flex shrink-0 items-center gap-0.5 text-[9px] text-muted-foreground">
                            <ShieldCheck className="size-2.5" />
                            {MCP_ACCESS_LABELS[connection.mcpAccess]}
                          </span>
                        </span>
                        <span className="mt-0.5 block truncate font-mono text-[10px] text-muted-foreground">
                          {connection.username}@{connection.host}:
                          {connection.port}
                        </span>
                        {connection.tags.length > 0 && (
                          <span className="mt-1 block truncate text-[9.5px] text-muted-foreground/70">
                            {connection.tags.slice(0, 3).join(" · ")}
                          </span>
                        )}
                      </span>

                      <DropdownMenu>
                        <DropdownMenuTrigger asChild>
                          <button
                            type="button"
                            aria-label={`${connection.alias} 的更多操作`}
                            onClick={(event) => event.stopPropagation()}
                            onKeyDown={(event) => event.stopPropagation()}
                            className={cn(
                              "mt-0.5 grid size-6 shrink-0 place-items-center rounded text-muted-foreground transition-opacity hover:bg-white/10 hover:text-foreground",
                              active
                                ? "opacity-70"
                                : "opacity-0 group-hover:opacity-60",
                            )}
                          >
                            <MoreHorizontal className="size-3.5" />
                          </button>
                        </DropdownMenuTrigger>
                        <DropdownMenuContent align="end" className="w-40">
                          <DropdownMenuItem onSelect={() => onEdit(connection)}>
                            <Pencil />
                            编辑连接
                          </DropdownMenuItem>
                          <DropdownMenuItem onSelect={() => onOpen(connection)}>
                            打开终端
                          </DropdownMenuItem>
                        </DropdownMenuContent>
                      </DropdownMenu>
                    </div>
                  );
                })}
              </div>
            ))}
          </div>
        </ScrollArea>

        <div className="shrink-0 border-t border-border p-2">
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={onCollapse}
            className="w-full justify-start gap-2 px-2 text-[10.5px] font-normal text-muted-foreground hover:text-foreground"
          >
            <PanelLeftClose className="size-3.5" />
            收起连接面板
            <kbd className="ml-auto rounded border border-border bg-white/[0.03] px-1 py-px font-mono text-[9px] text-muted-foreground/80">
              Ctrl+Shift+B
            </kbd>
          </Button>
        </div>
      </div>
    </aside>
  );
}
