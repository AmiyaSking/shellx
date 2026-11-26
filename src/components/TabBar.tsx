import { useState } from "react";
import type { Connection } from "../lib/types";

export interface TerminalTab {
  id: string;
  connectionId: string;
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
}: Props) {
  const [editingTabId, setEditingTabId] = useState<string | null>(null);
  const [draftTitle, setDraftTitle] = useState("");
  const [draggingTabId, setDraggingTabId] = useState<string | null>(null);

  const displayTitle = (tab: TerminalTab, connection: Connection | undefined) =>
    tab.customTitle?.trim() ||
    tab.title?.trim() ||
    connection?.alias ||
    "未知连接";

  const startEditing = (tab: TerminalTab, connection: Connection | undefined) => {
    setEditingTabId(tab.id);
    setDraftTitle(displayTitle(tab, connection));
  };

  const commitEditing = () => {
    if (!editingTabId) return;
    onRename(editingTabId, draftTitle);
    setEditingTabId(null);
  };

  return (
    <div className="tabbar" role="tablist" aria-label="终端标签页">
      {tabs.map((tab) => {
        const connection = connections.get(tab.connectionId);
        const editing = editingTabId === tab.id;
        return (
          <div
            key={tab.id}
            role="tab"
            aria-selected={tab.id === activeTabId}
            className={`tab ${tab.id === activeTabId ? "is-active" : ""} ${
              draggingTabId === tab.id ? "is-dragging" : ""
            }`}
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
              const from = event.dataTransfer.getData("text/plain") || draggingTabId;
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
          >
            <span className={`env-dot env-${connection?.environment ?? "dev"}`} />
            {editing ? (
              <input
                className="tab-rename"
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
                    setEditingTabId(null);
                  }
                }}
              />
            ) : (
              <span className="tab-title">{displayTitle(tab, connection)}</span>
            )}
            {tab.attention && tab.id !== activeTabId && !editing && (
              <span className="tab-attention" title="终端响铃 / 有新输出" />
            )}
            <button
              type="button"
              className="tab-close"
              title="关闭标签页"
              onClick={(event) => {
                event.stopPropagation();
                onClose(tab.id);
              }}
            >
              ×
            </button>
          </div>
        );
      })}
      <button
        type="button"
        className="tab-new"
        title="复制当前会话（Ctrl+Shift+T）"
        onClick={onNew}
      >
        +
      </button>
      <div className="tabbar-fill" />
    </div>
  );
}
