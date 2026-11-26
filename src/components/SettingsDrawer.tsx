import { useEffect, useState } from "react";
import {
  DEFAULT_SETTINGS,
  loadSettings,
  saveSettings,
  type AppSettings,
} from "../lib/settings";

interface Props {
  onClose: () => void;
}

const SCROLLBACK_OPTIONS = [1000, 5000, 10000, 30000, 100000];

export default function SettingsDrawer({ onClose }: Props) {
  const [settings, setSettings] = useState<AppSettings>(() => loadSettings());

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const update = (patch: Partial<AppSettings>) => {
    const next = { ...settings, ...patch };
    setSettings(next);
    saveSettings(next);
  };

  return (
    <div className="drawer-backdrop" role="presentation" onClick={onClose}>
      <aside
        className="drawer"
        role="dialog"
        aria-modal="true"
        aria-label="设置"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="drawer-head">
          <div>
            <h2>设置</h2>
            <p className="muted">终端外观与剪贴板行为，保存在本机。</p>
          </div>
          <button className="icon-btn" onClick={onClose} aria-label="关闭">
            ×
          </button>
        </div>

        <div className="settings-body">
          <section className="settings-section">
            <h3>终端</h3>

            <label className="settings-row">
              <span className="settings-copy">
                <strong>字号</strong>
                <span className="muted small">终端文字的像素大小</span>
              </span>
              <input
                type="range"
                min={10}
                max={24}
                step={1}
                value={settings.fontSize}
                onChange={(event) => update({ fontSize: Number(event.target.value) })}
              />
              <span className="settings-value">{settings.fontSize}</span>
            </label>

            <label className="settings-row">
              <span className="settings-copy">
                <strong>回滚缓冲</strong>
                <span className="muted small">保留的历史输出行数</span>
              </span>
              <select
                value={settings.scrollback}
                onChange={(event) =>
                  update({ scrollback: Number(event.target.value) })
                }
              >
                {SCROLLBACK_OPTIONS.map((value) => (
                  <option key={value} value={value}>
                    {value.toLocaleString()} 行
                  </option>
                ))}
              </select>
            </label>

            <label className="settings-row settings-check">
              <span className="settings-copy">
                <strong>光标闪烁</strong>
                <span className="muted small">关闭可减少远程桌面下的重绘</span>
              </span>
              <input
                type="checkbox"
                checked={settings.cursorBlink}
                onChange={(event) => update({ cursorBlink: event.target.checked })}
              />
            </label>
          </section>

          <section className="settings-section">
            <h3>剪贴板与安全</h3>

            <label className="settings-row settings-check">
              <span className="settings-copy">
                <strong>多行粘贴确认</strong>
                <span className="muted small">
                  粘贴包含换行或超长文本时先预览确认
                </span>
              </span>
              <input
                type="checkbox"
                checked={settings.pasteWarning}
                onChange={(event) => update({ pasteWarning: event.target.checked })}
              />
            </label>

            <label className="settings-row settings-check">
              <span className="settings-copy">
                <strong>选中即复制</strong>
                <span className="muted small">选中终端文本后自动写入剪贴板</span>
              </span>
              <input
                type="checkbox"
                checked={settings.copyOnSelect}
                onChange={(event) => update({ copyOnSelect: event.target.checked })}
              />
            </label>
          </section>

          <p className="settings-note">
            设置立即生效并应用到所有终端标签。图标、主题与快捷键自定义将在后续版本加入。
          </p>

          <div className="settings-foot">
            <button
              type="button"
              className="btn btn-ghost btn-sm"
              onClick={() => {
                setSettings(DEFAULT_SETTINGS);
                saveSettings(DEFAULT_SETTINGS);
              }}
            >
              恢复默认
            </button>
          </div>
        </div>
      </aside>
    </div>
  );
}
