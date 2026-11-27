import { useEffect } from "react";

interface Props {
  text: string;
  onConfirm: () => void;
  onCancel: () => void;
}

/** Review multi-line or very large clipboard content before it reaches a shell. */
export default function PasteConfirmDialog({ text, onConfirm, onCancel }: Props) {
  const lines = text.split(/\r\n|\r|\n/).length;
  const preview = text.length > 800 ? `${text.slice(0, 800)}…` : text;

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onCancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel]);

  return (
    <div className="modal-backdrop" role="presentation">
      <div
        className="modal approval-modal paste-modal"
        role="alertdialog"
        aria-modal="true"
        aria-label="确认粘贴"
      >
        <div className="modal-head">
          <div>
            <div className="approval-kicker">
              <span className="risk-badge risk-medium">粘贴确认</span>
              <span className="source-badge source-user">
                {lines} 行 · {text.length} 字符
              </span>
            </div>
            <h2>要粘贴多行内容到终端吗？</h2>
            <p className="muted">
              多行文本可能包含会立即执行的命令。确认内容无误后再粘贴到远端 shell。
            </p>
          </div>
        </div>

        <pre className="command-block paste-preview">{preview}</pre>

        <div className="modal-foot">
          <div className="muted small">可在设置中关闭此确认。</div>
          <div className="modal-foot-right">
            <button
              type="button"
              className="btn btn-ghost"
              onClick={onCancel}
              autoFocus
            >
              取消
            </button>
            <button type="button" className="btn btn-primary" onClick={onConfirm}>
              粘贴
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
