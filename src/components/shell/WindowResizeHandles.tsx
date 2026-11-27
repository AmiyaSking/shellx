import { getCurrentWindow } from "@tauri-apps/api/window";
import { isTauri } from "@/lib/api";
import { cn } from "@/lib/utils";

type ResizeDirection =
  | "East"
  | "North"
  | "NorthEast"
  | "NorthWest"
  | "South"
  | "SouthEast"
  | "SouthWest"
  | "West";

interface Handle {
  direction: ResizeDirection;
  className: string;
  cursor: string;
}

const HANDLES: Handle[] = [
  { direction: "North", className: "inset-x-2 top-0 h-1", cursor: "ns-resize" },
  { direction: "South", className: "inset-x-2 bottom-0 h-1", cursor: "ns-resize" },
  { direction: "West", className: "inset-y-2 left-0 w-1", cursor: "ew-resize" },
  { direction: "East", className: "inset-y-2 right-0 w-1", cursor: "ew-resize" },
  { direction: "NorthWest", className: "left-0 top-0 size-2", cursor: "nwse-resize" },
  { direction: "NorthEast", className: "right-0 top-0 size-2", cursor: "nesw-resize" },
  { direction: "SouthWest", className: "left-0 bottom-0 size-2", cursor: "nesw-resize" },
  { direction: "SouthEast", className: "right-0 bottom-0 size-2", cursor: "nwse-resize" },
];

/**
 * Borderless windows need explicit hit areas to keep edge/corner resizing.
 */
export default function WindowResizeHandles() {
  if (!isTauri()) return null;
  const win = getCurrentWindow();

  return (
    <div className="pointer-events-none fixed inset-0 z-40">
      {HANDLES.map((handle) => (
        <div
          key={handle.direction}
          className={cn("pointer-events-auto absolute", handle.className)}
          style={{ cursor: handle.cursor }}
          onMouseDown={(event) => {
            event.preventDefault();
            void win.startResizeDragging(handle.direction);
          }}
        />
      ))}
    </div>
  );
}
