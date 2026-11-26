import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  DEFAULT_SETTINGS,
  loadSettings,
  saveSettings,
  SETTINGS_EVENT,
} from "./settings";

describe("settings", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it("returns defaults when storage is empty", () => {
    expect(loadSettings()).toEqual(DEFAULT_SETTINGS);
    expect(loadSettings().fontSize).toBe(13);
  });

  it("migrates the legacy font-size key", () => {
    window.localStorage.setItem("shellx.fontSize", "18");
    const settings = loadSettings();
    expect(settings.fontSize).toBe(18);
    expect(settings.scrollback).toBe(DEFAULT_SETTINGS.scrollback);
  });

  it("clamps out-of-range values and persists booleans", () => {
    saveSettings({
      ...DEFAULT_SETTINGS,
      fontSize: 99,
      scrollback: 1,
      cursorBlink: false,
      restoreWorkspace: true,
      sessionLogging: true,
    });

    const loaded = loadSettings();
    expect(loaded.fontSize).toBe(24);
    expect(loaded.scrollback).toBe(1000);
    expect(loaded.cursorBlink).toBe(false);
    expect(loaded.restoreWorkspace).toBe(true);
    expect(loaded.sessionLogging).toBe(true);
  });

  it("dispatches a settings event with normalized values", () => {
    const handler = vi.fn();
    window.addEventListener(SETTINGS_EVENT, handler);

    saveSettings({ ...DEFAULT_SETTINGS, fontSize: 5 });

    expect(handler).toHaveBeenCalledTimes(1);
    const event = handler.mock.calls[0][0] as CustomEvent;
    expect(event.detail.fontSize).toBe(10);
    window.removeEventListener(SETTINGS_EVENT, handler);
  });
});
