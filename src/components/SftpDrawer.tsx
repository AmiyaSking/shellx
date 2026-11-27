import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { api, isTauri } from "../lib/api";
import type {
  Connection,
  SftpEntry,
  SftpListing,
  SftpProgressEvent,
} from "../lib/types";
import DrawerShell from "./shell/DrawerShell";

interface Props {
  connections: Connection[];
  initialConnectionId?: string | null;
  open: boolean;
  onClose: () => void;
}

function parentPath(path: string): string {
  const trimmed = path.replace(/\/+$/, "");
  if (!trimmed || trimmed === "/") return "/";
  const index = trimmed.lastIndexOf("/");
  return index <= 0 ? "/" : trimmed.slice(0, index);
}

function joinRemote(base: string, name: string): string {
  return base === "/" ? `/${name}` : `${base.replace(/\/+$/, "")}/${name}`;
}

function humanSize(size: number): string {
  if (size < 1024) return `${size} B`;
  if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KiB`;
  if (size < 1024 * 1024 * 1024) {
    return `${(size / (1024 * 1024)).toFixed(1)} MiB`;
  }
  return `${(size / (1024 * 1024 * 1024)).toFixed(2)} GiB`;
}

function formatDate(value?: string | null): string {
  if (!value) return "—";
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? "—" : date.toLocaleString();
}

export default function SftpDrawer({
  connections,
  initialConnectionId,
  open,
  onClose,
}: Props) {
  const [connectionId, setConnectionId] = useState(
    initialConnectionId && connections.some((item) => item.id === initialConnectionId)
      ? initialConnectionId
      : (connections[0]?.id ?? ""),
  );
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [listing, setListing] = useState<SftpListing | null>(null);
  const [pathInput, setPathInput] = useState("");
  const [connecting, setConnecting] = useState(false);
  const [loading, setLoading] = useState(false);
  const [busyPath, setBusyPath] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [progress, setProgress] = useState<SftpProgressEvent | null>(null);
  const sessionRef = useRef<string | null>(null);

  useEffect(() => {
    return () => {
      if (sessionRef.current) {
        void api.sftpClose(sessionRef.current).catch(() => {});
        sessionRef.current = null;
      }
    };
  }, []);

  useEffect(() => {
    if (!isTauri()) return;
    let unlisten: (() => void) | undefined;
    void listen<SftpProgressEvent>("sftp:progress", ({ payload }) => {
      if (payload.sessionId !== sessionRef.current) return;
      setProgress(payload);
    }).then((dispose) => {
      unlisten = dispose;
    });
    return () => unlisten?.();
  }, []);

  const connect = useCallback(async () => {
    if (!connectionId) return;
    if (sessionRef.current) {
      void api.sftpClose(sessionRef.current).catch(() => {});
      sessionRef.current = null;
      setSessionId(null);
    }
    setConnecting(true);
    setError(null);
    setNotice(null);
    try {
      const opened = await api.sftpOpen(connectionId);
      sessionRef.current = opened.sessionId;
      setSessionId(opened.sessionId);
      setListing(opened.listing);
      setPathInput(opened.listing.path);
    } catch (err) {
      setError(String(err));
    } finally {
      setConnecting(false);
    }
  }, [connectionId]);

  const navigate = useCallback(
    async (path: string) => {
      if (!sessionRef.current) return;
      setLoading(true);
      setError(null);
      setNotice(null);
      setProgress(null);
      try {
        const next = await api.sftpList(sessionRef.current, path);
        setListing(next);
        setPathInput(next.path);
      } catch (err) {
        setError(String(err));
      } finally {
        setLoading(false);
      }
    },
    [],
  );

  const upload = async () => {
    if (!sessionRef.current || !listing) return;
    if (!isTauri()) {
      setError("浏览器 Mock 模式不支持系统文件对话框，请使用桌面版");
      return;
    }
    try {
      const picked = await openDialog({
        multiple: false,
        directory: false,
        title: "选择要上传的文件",
      });
      const localPath = Array.isArray(picked) ? picked[0] : picked;
      if (!localPath) return;
      const name = localPath.split(/[\\/]/).pop() || "upload.bin";
      const remotePath = joinRemote(listing.path, name);
      setBusyPath("__upload__");
      setError(null);
      setProgress(null);
      const bytes = await api.sftpUpload(
        sessionRef.current,
        localPath,
        remotePath,
      );
      await navigate(listing.path);
      setNotice(`已上传 ${name}（${bytes} 字节）`);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusyPath(null);
      setProgress(null);
    }
  };

  const download = async (entry: SftpEntry) => {
    if (!sessionRef.current) return;
    if (!isTauri()) {
      setError("浏览器 Mock 模式不支持系统文件对话框，请使用桌面版");
      return;
    }
    try {
      const localPath = await saveDialog({
        defaultPath: entry.name,
        title: "保存远端文件",
      });
      if (!localPath) return;
      setBusyPath(entry.path);
      setError(null);
      setProgress(null);
      const bytes = await api.sftpDownload(
        sessionRef.current,
        entry.path,
        localPath,
      );
      setNotice(`已下载 ${entry.name}（${bytes} 字节）`);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusyPath(null);
      setProgress(null);
    }
  };

  const makeDirectory = async () => {
    if (!sessionRef.current || !listing) return;
    const name = window.prompt("新文件夹名称");
    if (!name) return;
    if (name.includes("/") || name.includes("\\") || name === "." || name === "..") {
      setError("文件夹名称不合法。");
      return;
    }
    setBusyPath("__mkdir__");
    setError(null);
    try {
      await api.sftpMkdir(sessionRef.current, joinRemote(listing.path, name));
      await navigate(listing.path);
      setNotice(`已创建文件夹 ${name}`);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusyPath(null);
    }
  };

  const renameEntry = async (entry: SftpEntry) => {
    if (!sessionRef.current || !listing) return;
    const name = window.prompt("新名称", entry.name);
    if (!name || name === entry.name) return;
    if (name.includes("/") || name.includes("\\") || name === "." || name === "..") {
      setError("名称不合法。");
      return;
    }
    setBusyPath(entry.path);
    setError(null);
    try {
      await api.sftpRename(
        sessionRef.current,
        entry.path,
        joinRemote(listing.path, name),
      );
      await navigate(listing.path);
      setNotice(`已重命名为 ${name}`);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusyPath(null);
    }
  };

  const removeEntry = async (entry: SftpEntry) => {
    if (!sessionRef.current || !listing) return;
    const question = entry.isDir
      ? `删除目录「${entry.name}」及其中的全部内容？此操作不可撤销。`
      : `删除文件「${entry.name}」？`;
    if (!window.confirm(question)) {
      return;
    }
    setBusyPath(entry.path);
    setError(null);
    try {
      await api.sftpRemove(sessionRef.current, entry.path, entry.isDir);
      await navigate(listing.path);
      setNotice(`已删除 ${entry.name}`);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusyPath(null);
    }
  };

  const cancelTransfer = async () => {
    if (!sessionRef.current) return;
    try {
      const requested = await api.sftpCancel(sessionRef.current);
      if (requested) setNotice("已请求取消传输，当前分块完成后停止。");
    } catch (err) {
      setError(String(err));
    }
  };

  const openEntry = (entry: SftpEntry) => {
    if (entry.isDir) {
      void navigate(entry.path);
    } else {
      void download(entry);
    }
  };

  const progressPercent =
    progress && progress.total > 0
      ? Math.min(
          100,
          Math.round((progress.transferred / progress.total) * 100),
        )
      : null;

  return (
    <DrawerShell
      open={open}
      onClose={onClose}
      label="SFTP 文件浏览"
      width="min(880px,100vw)"
    >
        <div className="drawer-head">
          <div>
            <h2>SFTP 文件</h2>
            <p className="muted">
              浏览、上传和下载远端文件。主机密钥需先在终端中确认。
            </p>
          </div>
          <button className="icon-btn" onClick={onClose} aria-label="关闭">
            ×
          </button>
        </div>

        <div className="sftp-connect-row">
          <select
            value={connectionId}
            onChange={(event) => {
              setConnectionId(event.target.value);
              setSessionId(null);
              setListing(null);
              if (sessionRef.current) {
                void api.sftpClose(sessionRef.current).catch(() => {});
                sessionRef.current = null;
              }
            }}
          >
            {connections.length === 0 && (
              <option value="">请先创建连接</option>
            )}
            {connections.map((connection) => (
              <option key={connection.id} value={connection.id}>
                {connection.alias}（{connection.host}）
              </option>
            ))}
          </select>
          <button
            className="btn btn-primary btn-sm"
            disabled={!connectionId || connecting}
            onClick={() => void connect()}
          >
            {connecting ? "连接中…" : sessionId ? "重新连接" : "连接"}
          </button>
          {sessionId && (
            <span className="mini-badge mcp-standard_controlled">已连接</span>
          )}
        </div>

        {error && <div className="form-error">{error}</div>}
        {notice && <div className="sftp-notice">{notice}</div>}

        {sessionId && (
          <>
            <div className="sftp-pathbar">
              <button
                className="icon-btn"
                title="上一级"
                onClick={() => listing && void navigate(parentPath(listing.path))}
              >
                ↑
              </button>
              <input
                className="sftp-path-input"
                value={pathInput}
                spellCheck={false}
                onChange={(event) => setPathInput(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    event.preventDefault();
                    void navigate(pathInput);
                  }
                }}
              />
              <button
                className="btn btn-ghost btn-sm"
                onClick={() => void navigate(pathInput)}
              >
                转到
              </button>
              <button
                className="btn btn-ghost btn-sm"
                disabled={loading}
                onClick={() => listing && void navigate(listing.path)}
              >
                刷新
              </button>
              <button
                className="btn btn-ghost btn-sm"
                disabled={busyPath !== null}
                onClick={() => void makeDirectory()}
              >
                新建目录
              </button>
              <button
                className="btn btn-ghost btn-sm"
                disabled={busyPath !== null}
                onClick={() => void upload()}
              >
                {busyPath === "__upload__" ? "上传中…" : "上传"}
              </button>
            </div>

            {progress && (
              <div className="sftp-progress">
                <div className="sftp-progress-track">
                  <div
                    className={`sftp-progress-bar ${
                      progressPercent === null ? "is-indeterminate" : ""
                    }`}
                    style={
                      progressPercent === null
                        ? undefined
                        : { width: `${progressPercent}%` }
                    }
                  />
                </div>
                <div className="sftp-progress-meta">
                  <span className="muted small">
                    {progress.direction === "upload" ? "上传" : "下载"}{" "}
                    {humanSize(progress.transferred)}
                    {progress.total > 0
                      ? ` / ${humanSize(progress.total)} · ${progressPercent}%`
                      : ""}
                  </span>
                  <button
                    className="btn btn-ghost btn-sm"
                    onClick={() => void cancelTransfer()}
                  >
                    取消
                  </button>
                </div>
              </div>
            )}

            <div className="sftp-list">
              <div className="sftp-list-head">
                <span>名称</span>
                <span>大小</span>
                <span>修改时间</span>
                <span />
              </div>
              {loading && <div className="muted pad">读取目录中…</div>}
              {!loading && listing?.entries.length === 0 && (
                <div className="empty-hint">
                  <p>目录为空。</p>
                </div>
              )}
              {!loading &&
                listing?.entries.map((entry) => (
                  <div
                    className="sftp-row"
                    key={entry.path}
                    onDoubleClick={() => openEntry(entry)}
                  >
                    <div className="sftp-name">
                      <span
                        className={`sftp-icon ${entry.isDir ? "is-dir" : ""}`}
                      >
                        {entry.isSymlink ? "↗" : entry.isDir ? "▸" : "≡"}
                      </span>
                      <span title={entry.path}>{entry.name}</span>
                    </div>
                    <span className="muted small">
                      {entry.isDir ? "—" : humanSize(entry.size)}
                    </span>
                    <span className="muted small">
                      {formatDate(entry.modified)}
                    </span>
                    <div className="sftp-row-actions">
                      {entry.isDir ? (
                        <button
                          className="btn btn-ghost btn-sm"
                          disabled={busyPath !== null}
                          onClick={() => void navigate(entry.path)}
                        >
                          打开
                        </button>
                      ) : (
                        <button
                          className="btn btn-ghost btn-sm"
                          disabled={busyPath !== null}
                          onClick={() => void download(entry)}
                        >
                          {busyPath === entry.path ? "下载中…" : "下载"}
                        </button>
                      )}
                      <button
                        type="button"
                        className="icon-btn"
                        title="重命名"
                        disabled={busyPath !== null}
                        onClick={() => void renameEntry(entry)}
                      >
                        ✎
                      </button>
                      <button
                        type="button"
                        className="icon-btn"
                        title="删除"
                        disabled={busyPath !== null}
                        onClick={() => void removeEntry(entry)}
                      >
                        ×
                      </button>
                    </div>
                  </div>
                ))}
            </div>
          </>
        )}

        {!sessionId && !connecting && (
          <div className="empty-hint">
            <p>选择一个连接并点击「连接」开始浏览文件。</p>
            <p>SFTP 使用独立 SSH 会话，不会影响已打开的终端。</p>
          </div>
        )}

        <p className="muted small">
          双击目录进入，双击文件或点击「下载」保存到本机。上传会覆盖同名远端文件。
        </p>
    </DrawerShell>
  );
}
