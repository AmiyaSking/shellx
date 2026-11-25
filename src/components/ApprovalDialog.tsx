import { useState } from "react";
import { api } from "../lib/api";
import {
  ENV_LABELS,
  RISK_LABELS,
  type ApprovalRequest,
  type CommandOutcome,
} from "../lib/types";

interface Props {
  approval: ApprovalRequest;
  onResolved: (outcome: CommandOutcome) => void;
}

export default function ApprovalDialog({ approval, onResolved }: Props) {
  const [busy, setBusy] = useState(false);
  const [editing, setEditing] = useState(false);
  const [command, setCommand] = useState(approval.command);
  const [error, setError] = useState<string | null>(null);

  const decide = async (approve: boolean) => {
    if (approve && editing && !command.trim()) {
      setError("修改后的命令不能为空。");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const override =
        approve && editing && command.trim() !== approval.command ? command.trim() : null;
      const outcome = await api.resolveApproval(approval.id, approve, override);
      onResolved(outcome);
    } catch (err) {
      setError(String(err));
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop approval-backdrop" role="presentation">
      <div
        className="modal approval-modal"
        role="alertdialog"
        aria-modal="true"
        aria-label="命令审批"
      >
        <div className="modal-head">
          <div>
            <div className="approval-kicker">
              <span className={`risk-badge risk-${approval.risk}`}>
                {RISK_LABELS[approval.risk]}
              </span>
              <span className={`source-badge source-${approval.source}`}>
                {approval.source === "agent" ? "AI Agent 发起" : "用户手动发起"}
              </span>
            </div>
            <h2>命令需要人工审批</h2>
            <p className="muted">
              策略引擎已挂起本次调用。请确认目标服务器与完整命令后再决定是否执行。
            </p>
          </div>
        </div>

        <div className="approval-meta">
          <div>
            <span className="meta-label">目标服务器</span>
            <strong>{approval.connectionAlias}</strong>
          </div>
          <div>
            <span className="meta-label">环境</span>
            <span className={`badge badge-${approval.environment}`}>
              {ENV_LABELS[approval.environment]}
            </span>
          </div>
          <div>
            <span className="meta-label">请求时间</span>
            <span>{new Date(approval.requestedAt).toLocaleString()}</span>
          </div>
        </div>

        <div className="approval-section">
          <div className="approval-section-head">
            <span>完整命令</span>
            <label className="check">
              <input
                type="checkbox"
                checked={editing}
                onChange={(event) => setEditing(event.target.checked)}
              />
              修改后执行
            </label>
          </div>
          {editing ? (
            <textarea
              className="command-editor"
              value={command}
              onChange={(event) => setCommand(event.target.value)}
              rows={3}
              spellCheck={false}
            />
          ) : (
            <pre className="command-block">{approval.command}</pre>
          )}
        </div>

        <div className="approval-section">
          <div className="approval-section-head">
            <span>风险原因</span>
          </div>
          <ul className="reason-list">
            {approval.reasons.map((reason) => (
              <li key={reason}>{reason}</li>
            ))}
          </ul>
        </div>

        {error && <div className="form-error">{error}</div>}

        <div className="modal-foot">
          <div className="muted small">
            审批结果与命令会写入本地审计日志。
          </div>
          <div className="modal-foot-right">
            <button
              type="button"
              className="btn btn-danger"
              disabled={busy}
              onClick={() => void decide(false)}
            >
              拒绝
            </button>
            <button
              type="button"
              className="btn btn-primary"
              disabled={busy}
              onClick={() => void decide(true)}
            >
              {busy ? "处理中…" : editing ? "批准并执行修改后的命令" : "批准并执行"}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
