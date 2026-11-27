import { useEffect, useState } from "react";
import { Copy, Minus, Square, X } from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { isTauri } from "@/lib/api";
import { cn } from "@/lib/utils";

export default function WindowControls() {
  const tauri = isTauri();
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    if (!tauri) return;
    const win = getCurrentWindow();
    let disposed = false;
    let unlisten: (() => void) | undefined;

    const sync = () => {
      void win.isMaximized().then((value) => {
        if (!disposed) setMaximized(value);
      });
    };

    sync();
    void win.onResized(sync).then((dispose) => {
      if (disposed) dispose();
      else unlisten = dispose;
    });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [tauri]);

  const minimize = () => {
    if (tauri) void getCurrentWindow().minimize();
  };
  const toggleMaximize = () => {
    if (tauri) void getCurrentWindow().toggleMaximize();
  };
  const close = () => {
    if (tauri) void getCurrentWindow().close();
  };

  return (
    <div className="flex h-full items-stretch border-l border-border">
      <button
        type="button"
        aria-label="最小化"
        title="最小化"
        onClick={minimize}
        className={cn(
          "flex w-11 items-center justify-center text-muted-foreground transition-colors hover:bg-white/[0.06] hover:text-foreground",
          !tauri && "cursor-default",
        )}
      >
        <Minus className="size-3.5" />
      </button>
      <button
        type="button"
        aria-label={maximized ? "还原" : "最大化"}
        title={maximized ? "还原" : "最大化"}
        onClick={toggleMaximize}
        className={cn(
          "flex w-11 items-center justify-center text-muted-foreground transition-colors hover:bg-white/[0.06] hover:text-foreground",
          !tauri && "cursor-default",
        )}
      >
        {maximized ? <Copy className="size-3" /> : <Square className="size-3" />}
      </button>
      <button
        type="button"
        aria-label="关闭"
        title="关闭"
        onClick={close}
        className="flex w-11 items-center justify-center text-muted-foreground transition-colors hover:bg-red-500 hover:text-white"
      >
        <X className="size-3.5" />
      </button>
    </div>
  );
}
