import { useEffect, useState } from "react";
import { api } from "../lib/api";
import { writeClipboardText } from "../lib/clipboard";
import { MCP_CLIENTS, type McpLaunchInfo } from "../lib/mcpClientConfigs";
import DrawerShell from "./shell/DrawerShell";

interface Props {
  open: boolean;
  onClose: () => void;
}

export default function McpClientConfigDrawer({ open, onClose }: Props) {
  const [launch, setLaunch] = useState<McpLaunchInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [clientId, setClientId] = useState(MCP_CLIENTS[0]?.id ?? "");
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    let cancelled = false;
    api
      .mcpLaunchInfo()
      .then((info) => {
        if (!cancelled) setLaunch(info);
      })
      .catch((err) => {
        if (!cancelled) setError(String(err));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const client =
    MCP_CLIENTS.find((candidate) => candidate.id === clientId) ??
    MCP_CLIENTS[0];
  const config = launch && client ? client.build(launch) : "";

  const copy = async () => {
    if (!config) return;
    await writeClipboardText(config);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1500);
  };

  return (
    <DrawerShell
      open={open}
      onClose={onClose}
      label="MCP 客户端配置"
      width="min(900px,100vw)"
    >
        <div className="drawer-head">
          <div>
            <h2>MCP 客户端配置</h2>
            <p className="muted">
              选择要接入的 Agent 客户端，把配置复制到对应文件。配置只包含启动
              ShellX MCP Server 的命令，不包含权限或连接范围。
            </p>
          </div>
          <button className="icon-btn" onClick={onClose} aria-label="关闭">
            ×
          </button>
        </div>

        {error && <p className="error-text">{error}</p>}

        {!launch && !error && <p className="muted">读取启动路径中…</p>}

        {launch && client && (
          <div className="mcp-config-body">
            <div className="mcp-config-clients" role="tablist" aria-label="MCP 客户端">
              {MCP_CLIENTS.map((candidate) => (
                <button
                  key={candidate.id}
                  type="button"
                  role="tab"
                  aria-selected={candidate.id === client.id}
                  className={`mcp-config-tab ${
                    candidate.id === client.id ? "is-active" : ""
                  }`}
                  onClick={() => {
                    setClientId(candidate.id);
                    setCopied(false);
                  }}
                >
                  {candidate.name}
                </button>
              ))}
            </div>

            <div className="mcp-config-path">{client.pathHint}</div>

            <div className="mcp-config-code">
              <button
                type="button"
                className="btn btn-ghost btn-sm mcp-config-copy"
                onClick={() => void copy()}
              >
                {copied ? "已复制" : "复制"}
              </button>
              <pre>{config}</pre>
            </div>

            <p className="muted small">
              启动命令：<code>{launch.command}</code>{" "}
              <code>{launch.args.join(" ")}</code>
            </p>
            <p className="muted small">
              主机密钥仍需先在桌面端终端连接中确认；权限策略、可见连接与工具开关在
              「MCP 权限设置」中统一管理。
            </p>
          </div>
        )}

        <div className="drawer-actions mcp-config-footer">
          <button className="btn btn-ghost btn-sm" onClick={onClose}>
            关闭
          </button>
        </div>
    </DrawerShell>
  );
}
