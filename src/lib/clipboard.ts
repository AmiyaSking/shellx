import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";
import { isTauri } from "./api";

export async function readClipboardText(): Promise<string> {
  if (isTauri()) {
    try {
      return await readText();
    } catch {
      // Fall through to the web API if the plugin is unavailable.
    }
  }
  try {
    return await navigator.clipboard.readText();
  } catch {
    return "";
  }
}

export async function writeClipboardText(text: string): Promise<void> {
  if (!text) return;
  if (isTauri()) {
    try {
      await writeText(text);
      return;
    } catch {
      // Fall through to the web API.
    }
  }
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    // Clipboard access can be denied; nothing else we can do here.
  }
}
