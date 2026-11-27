import { useEffect, useState } from "react";
import { api } from "../lib/api";
import type { AuthPromptEvent } from "../lib/types";

interface Props {
  prompt: AuthPromptEvent;
  onClose: () => void;
}

/** Keyboard-interactive / 2FA prompts raised while `pty_open` is in flight. */
export default function AuthPromptDialog({ prompt, onClose }: Props) {
  const [answers, setAnswers] = useState<string[]>(() =>
    prompt.prompts.map(() => ""),
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // The parent reuses this component for queued prompts; never leak the
  // previous prompt's answers or busy state into the next one.
  useEffect(() => {
    setAnswers(prompt.prompts.map(() => ""));
    setBusy(false);
    setError(null);
  }, [prompt.promptId]);

  const cancel = async () => {
    if (busy) return;
    setBusy(true);
    try {
      await api.authCancel(prompt.promptId);
    } catch {
      // The connection will time out or be cancelled separately.
    } finally {
      setBusy(false);
      onClose();
    }
  };

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") void cancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [busy, prompt.promptId]);

  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      const accepted = await api.authRespond(prompt.promptId, answers);
      if (!accepted) {
        setError("认证请求已过期或被取消。");
        setBusy(false);
        return;
      }
      setBusy(false);
      onClose();
    } catch (err) {
      setError(String(err));
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop" role="presentation">
      <div
        className="modal auth-prompt-modal"
        role="alertdialog"
        aria-modal="true"
        aria-label="交互式认证"
      >
        <div className="modal-head">
          <div>
            <div className="approval-kicker">
              <span className="risk-badge risk-medium">交互式认证</span>
              <span className="source-badge source-user">{prompt.host}</span>
            </div>
            <h2>{prompt.name || "服务器要求额外认证"}</h2>
            <p className="muted">
              {prompt.instructions ||
                "请输入服务器要求的验证码或口令。内容只用于本次认证。"}
            </p>
          </div>
        </div>

        <div className="auth-prompt-body">
          {prompt.prompts.map((field, index) => (
            <label className="auth-prompt-field" key={`${field.prompt}-${index}`}>
              <span>{field.prompt || `应答 ${index + 1}`}</span>
              <input
                type={field.echo ? "text" : "password"}
                value={answers[index] ?? ""}
                autoFocus={index === 0}
                autoComplete="one-time-code"
                onChange={(event) => {
                  const value = event.target.value;
                  setAnswers((current) =>
                    current.map((answer, answerIndex) =>
                      answerIndex === index ? value : answer,
                    ),
                  );
                }}
              />
            </label>
          ))}
          {prompt.prompts.length === 0 && (
            <p className="muted small">服务器没有提供输入项，直接确认即可。</p>
          )}
          {error && <div className="form-error">{error}</div>}
        </div>

        <div className="modal-foot">
          <div className="muted small">认证内容不会写入连接配置。</div>
          <div className="modal-foot-right">
            <button
              type="button"
              className="btn btn-ghost"
              disabled={busy}
              onClick={() => void cancel()}
            >
              取消连接
            </button>
            <button
              type="button"
              className="btn btn-primary"
              disabled={busy}
              onClick={() => void submit()}
            >
              {busy ? "提交中…" : "提交认证"}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
