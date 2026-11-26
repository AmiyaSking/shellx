import type { Connection } from "../lib/types";

export interface TerminalTab {
  id: string;
  connectionId: string;
  /** Title reported by the remote shell via OSC 0/2, if any. */
  title?: string;
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
}

export default function TabBar({
  tabs,
  connections,
  activeTabId,
  onSelect,
  onClose,
  onNew,
}: Props) {
  return (
    <div className="tabbar" role="tablist" aria-label="终端标签页">
      {tabs.map((tab) => {
        const connection = connections.get(tab.connectionId);
        return (
          <div
            key={tab.id}
            role="tab"
            aria-selected={tab.id === activeTabId}
            className={`tab ${tab.id === activeTabId ? "is-active" : ""}`}
            title={tab.title}
            onClick={() => onSelect(tab.id)}
            onAuxClick={(event) => {
              if (event.button === 1) onClose(tab.id);
            }}
          >
            <span className={`env-dot env-${connection?.environment ?? "dev"}`} />
            <span className="tab-title">
              {tab.title?.trim() || connection?.alias || "未知连接"}
            </span>
            {tab.attention && tab.id !== activeTabId && (
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
