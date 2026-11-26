export interface AppSettings {
  fontSize: number;
  scrollback: number;
  cursorBlink: boolean;
  /** Ask before pasting multi-line or very large text into a shell. */
  pasteWarning: boolean;
  /** Copy the terminal selection to the clipboard as soon as it changes. */
  copyOnSelect: boolean;
  /** Reopen saved tabs/panes and reconnect on startup. */
  restoreWorkspace: boolean;
  /** Write raw PTY output to the app data directory. */
  sessionLogging: boolean;
}

const KEY = "shellx.settings.v1";
const LEGACY_FONT_KEY = "shellx.fontSize";
export const SETTINGS_EVENT = "shellx:settings";

export const DEFAULT_SETTINGS: AppSettings = {
  fontSize: 13,
  scrollback: 5000,
  cursorBlink: true,
  pasteWarning: true,
  copyOnSelect: false,
  restoreWorkspace: false,
  sessionLogging: false,
};

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

function normalize(input: Partial<AppSettings> | null | undefined): AppSettings {
  return {
    fontSize: clamp(
      Number.isFinite(input?.fontSize) ? Number(input?.fontSize) : DEFAULT_SETTINGS.fontSize,
      10,
      24,
    ),
    scrollback: clamp(
      Number.isFinite(input?.scrollback)
        ? Number(input?.scrollback)
        : DEFAULT_SETTINGS.scrollback,
      1000,
      100000,
    ),
    cursorBlink:
      typeof input?.cursorBlink === "boolean"
        ? input.cursorBlink
        : DEFAULT_SETTINGS.cursorBlink,
    pasteWarning:
      typeof input?.pasteWarning === "boolean"
        ? input.pasteWarning
        : DEFAULT_SETTINGS.pasteWarning,
    copyOnSelect:
      typeof input?.copyOnSelect === "boolean"
        ? input.copyOnSelect
        : DEFAULT_SETTINGS.copyOnSelect,
    restoreWorkspace:
      typeof input?.restoreWorkspace === "boolean"
        ? input.restoreWorkspace
        : DEFAULT_SETTINGS.restoreWorkspace,
    sessionLogging:
      typeof input?.sessionLogging === "boolean"
        ? input.sessionLogging
        : DEFAULT_SETTINGS.sessionLogging,
  };
}

export function loadSettings(): AppSettings {
  try {
    const raw = window.localStorage.getItem(KEY);
    if (raw) {
      return normalize(JSON.parse(raw) as Partial<AppSettings>);
    }
    // Migrate the font-size value saved by the earlier font zoom shortcut.
    const legacyRaw = window.localStorage.getItem(LEGACY_FONT_KEY);
    if (legacyRaw !== null) {
      const legacyFont = Number(legacyRaw);
      if (Number.isFinite(legacyFont)) {
        return normalize({ ...DEFAULT_SETTINGS, fontSize: legacyFont });
      }
    }
  } catch {
    // Corrupt or unavailable storage: use defaults.
  }
  return { ...DEFAULT_SETTINGS };
}

export function saveSettings(settings: AppSettings): void {
  const normalized = normalize(settings);
  try {
    window.localStorage.setItem(KEY, JSON.stringify(normalized));
    window.localStorage.setItem(LEGACY_FONT_KEY, String(normalized.fontSize));
  } catch {
    // Settings persistence is best-effort.
  }
  window.dispatchEvent(
    new CustomEvent<AppSettings>(SETTINGS_EVENT, { detail: normalized }),
  );
}
