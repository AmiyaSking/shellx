import type { HostKeyPrompt } from "../lib/types";

interface Props {
  hostKey: HostKeyPrompt;
  onTrust: () => void;
  onCancel: () => void;
}

/**
 * First-connect fingerprint confirmation. ShellX never trusts an unknown host
 * key silently: the backend refuses the handshake until the user confirms the
 * exact fingerprint shown here.
 */
export default function HostKeyDialog({ hostKey, onTrust, onCancel }: Props) {
  return (
    <div className="modal-backdrop" role="presentation">
      <div
        className="modal hostkey-modal"
        role="alertdialog"
        aria-modal="true"
        aria-label="确认主机密钥"
      >
        <div className="modal-head">
          <div>
            <div className="approval-kicker">
              <span className="risk-badge risk-medium">首次连接</span>
              <span className="source-badge source-user">主机密钥校验</span>
            </div>
            <h2>确认服务器指纹</h2>
            <p className="muted">
              ShellX 还没有记录过这台服务器的主机密钥。请与服务器管理员提供的
              SHA256 指纹核对后再信任。
            </p>
          </div>
        </div>

        <div className="hostkey-body">
          <div className="hostkey-row">
            <span className="meta-label">目标</span>
            <code>
              {hostKey.host}:{hostKey.port}
            </code>
          </div>
          <div className="hostkey-row">
            <span className="meta-label">算法</span>
            <code>{hostKey.algorithm}</code>
          </div>
          <div>
            <span className="meta-label">SHA256 指纹</span>
            <pre className="command-block hostkey-fingerprint">{hostKey.fingerprint}</pre>
          </div>
          <p className="muted small">
            确认后，该指纹会写入 ShellX 自己的 known_hosts 文件；以后密钥变化会被直接拒绝。
          </p>
        </div>

        <div className="modal-foot">
          <div className="muted small">不确定时请取消连接。</div>
          <div className="modal-foot-right">
            <button type="button" className="btn btn-ghost" onClick={onCancel}>
              取消连接
            </button>
            <button type="button" className="btn btn-primary" onClick={onTrust}>
              信任并连接
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
