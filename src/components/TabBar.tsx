import { useRef, useState } from "react";
import { Plus, Search, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import type { Connection } from "../lib/types";
import { cn } from "@/lib/utils";
import { ENV_DOT } from "./shell/env";
import WindowControls from "./shell/WindowControls";

export interface TerminalPaneState {
  id: string;
  connectionId: string;
}

export interface TerminalTab {
  id: string;
  panes: TerminalPaneState[];
  /** Flex weight per pane; same length as `panes`. */
  paneSizes: number[];
  /** Split direction for two or more panes. */
  layout: "row" | "column";
  activePaneId: string;
  /** Title reported by the remote shell via OSC 0/2, if any. */
  title?: string;
  /** User-provided title; takes precedence over the remote title. */
  customTitle?: string;
  /** A bell arrived while the tab was in the background. */
  attention?: boolean;
}

interface Props {
  tabs: TerminalTab[];
  connections: Map<string, Connection>;
  activeTabId: string | null;
  onSelect: (tabId: string) => void;
  onClose: (tabId: string) => void;
  onNew: () => void;
  onRename: (tabId: string, title: string) => void;
  onMove: (fromTabId: string, toTabId: string) => void;
  onOpenPalette: () => void;
}

export default function TabBar({
  tabs,
  connections,
  activeTabId,
  onSelect,
  onClose,
  onNew,
  onRename,
  onMove,
  onOpenPalette,
}: Props) {
  const [editingTabId, setEditingTabId] = useState<string | null>(null);
  const [draftTitle, setDraftTitle] = useState("");
  const [draggingTabId, setDraggingTabId] = useState<string | null>(null);
  const cancelEditRef = useRef(false);

  const displayTitle = (tab: TerminalTab, connection: Connection | undefined) =>
    tab.customTitle?.trim() ||
    tab.title?.trim() ||
    connection?.alias ||
    "未知连接";

  const startEditing = (tab: TerminalTab, connection: Connection | undefined) => {
    cancelEditRef.current = false;
    setEditingTabId(tab.id);
    setDraftTitle(displayTitle(tab, connection));
  };

  const commitEditing = () => {
    if (!editingTabId) return;
    if (cancelEditRef.current) {
      cancelEditRef.current = false;
      setEditingTabId(null);
      return;
    }
    const nextTitle = draftTitle.trim();
    // A blank submission should keep the existing title, not clear it.
    if (nextTitle) onRename(editingTabId, nextTitle);
    setEditingTabId(null);
  };

  return (
    <div
      className="relative z-[70] flex h-[38px] shrink-0 select-none items-center border-b border-border bg-[#0a0b0d]"
      data-tauri-drag-region="deep"
    >
      <div
        role="tablist"
        aria-label="终端标签页"
        className="flex h-full min-w-0 flex-1 items-center gap-0.5 overflow-x-auto overflow-y-hidden px-1.5"
      >
        {tabs.map((tab) => {
          const activePane =
            tab.panes.find((pane) => pane.id === tab.activePaneId) ??
            tab.panes[0];
          const connection = activePane
            ? connections.get(activePane.connectionId)
            : undefined;
          const editing = editingTabId === tab.id;
          const active = tab.id === activeTabId;
          return (
            <div
              key={tab.id}
              role="tab"
              aria-selected={active}
              title={displayTitle(tab, connection)}
              draggable={!editing}
              onDragStart={(event) => {
                setDraggingTabId(tab.id);
                event.dataTransfer.effectAllowed = "move";
                event.dataTransfer.setData("text/plain", tab.id);
              }}
              onDragEnd={() => setDraggingTabId(null)}
              onDragOver={(event) => {
                if (!draggingTabId || draggingTabId === tab.id) return;
                event.preventDefault();
                event.dataTransfer.dropEffect = "move";
              }}
              onDrop={(event) => {
                event.preventDefault();
                const from =
                  event.dataTransfer.getData("text/plain") || draggingTabId;
                if (from && from !== tab.id) onMove(from, tab.id);
                setDraggingTabId(null);
              }}
              onClick={() => onSelect(tab.id)}
              onDoubleClick={(event) => {
                event.preventDefault();
                startEditing(tab, connection);
              }}
              onAuxClick={(event) => {
                if (event.button === 1) onClose(tab.id);
              }}
              className={cn(
                "group relative flex h-full shrink-0 cursor-pointer select-none items-center gap-1.5 whitespace-nowrap px-2.5 text-xs text-muted-foreground transition-colors hover:bg-white/[0.03] hover:text-foreground",
                active && "text-foreground",
                draggingTabId === tab.id && "opacity-50",
              )}
            >
              {active && (
                <span className="absolute inset-x-2 bottom-0 h-0.5 rounded-full bg-primary" />
              )}
              <span
                className={cn(
                  "size-1.5 shrink-0 rounded-full",
                  ENV_DOT[connection?.environment ?? "dev"],
                )}
              />
              {editing ? (
                <input
                  className="w-24 rounded border border-border bg-background px-1 py-0.5 text-xs text-foreground outline-none ring-1 ring-primary/40"
                  value={draftTitle}
                  autoFocus
                  spellCheck={false}
                  onClick={(event) => event.stopPropagation()}
                  onDoubleClick={(event) => event.stopPropagation()}
                  onChange={(event) => setDraftTitle(event.target.value)}
                  onBlur={commitEditing}
                  onKeyDown={(event) => {
                    event.stopPropagation();
                    if (event.key === "Enter") {
                      event.preventDefault();
                      commitEditing();
                    } else if (event.key === "Escape") {
                      event.preventDefault();
                      cancelEditRef.current = true;
                      setEditingTabId(null);
                    }
                  }}
                />
              ) : (
                <span className="max-w-40 truncate">
                  {displayTitle(tab, connection)}
                </span>
              )}
              {tab.panes.length > 1 && (
                <span
                  className="rounded bg-white/5 px-1 font-mono text-[9px] text-muted-foreground"
                  title={`${tab.panes.length} 个窗格`}
                >
                  {tab.panes.length}
                </span>
              )}
              {tab.attention && !active && !editing && (
                <span
                  className="size-1.5 rounded-full bg-amber-400"
                  title="终端响铃 / 有新输出"
                />
              )}
              <button
                type="button"
                aria-label="关闭标签页"
                title="关闭标签页"
                onClick={(event) => {
                  event.stopPropagation();
                  onClose(tab.id);
                }}
                className={cn(
                  "ml-0.5 grid size-4 place-items-center rounded text-muted-foreground transition-opacity hover:bg-white/10 hover:text-foreground",
                  active
                    ? "opacity-60"
                    : "opacity-0 group-hover:opacity-100",
                )}
              >
                <X className="size-3" />
              </button>
            </div>
          );
        })}

        <button
          type="button"
          className="grid size-7 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-white/5 hover:text-foreground"
          title="复制当前会话（Ctrl+Shift+T）"
          aria-label="复制当前会话"
          onClick={onNew}
        >
          <Plus className="size-3.5" />
        </button>
      </div>

      <div className="flex h-full shrink-0 items-center">
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              type="button"
              variant="ghost"
              size="xs"
              onClick={onOpenPalette}
              className="mr-1 gap-1.5 px-2 text-[10px] font-normal text-muted-foreground hover:text-foreground"
            >
              <Search className="size-3" />
              命令
              <kbd className="command-kbd rounded border border-border bg-white/[0.03] px-1 font-mono text-[9px] text-muted-foreground/80">
                Ctrl+Shift+P
              </kbd>
            </Button>
          </TooltipTrigger>
          <TooltipContent side="bottom">打开命令面板</TooltipContent>
        </Tooltip>
        <WindowControls />
      </div>
    </div>
  );
}
