import type { Connection } from "../lib/types";

export interface TerminalTab {
  id: string;
  connectionId: string;
}

interface Props {
  tabs: TerminalTab[];
  connections: Map<string, Connection>;
  activeTabId: string | null;
  onSelect: (tabId: string) => void;
  onClose: (tabId: string) => void;
}

export default function TabBar({
  tabs,
  connections,
  activeTabId,
  onSelect,
  onClose,
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
            onClick={() => onSelect(tab.id)}
            onAuxClick={(event) => {
              if (event.button === 1) onClose(tab.id);
            }}
          >
            <span className={`env-dot env-${connection?.environment ?? "dev"}`} />
            <span className="tab-title">{connection?.alias ?? "未知连接"}</span>
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
      {tabs.length > 0 && <div className="tabbar-fill" />}
    </div>
  );
}
