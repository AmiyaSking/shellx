import type { ComponentProps, ReactNode } from "react";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetTitle,
} from "@/components/ui/sheet";

interface Props {
  open: boolean;
  onClose: () => void;
  label: string;
  width?: string;
  onEscapeKeyDown?: ComponentProps<typeof SheetContent>["onEscapeKeyDown"];
  children: ReactNode;
}

export default function DrawerShell({
  open,
  onClose,
  label,
  width,
  onEscapeKeyDown,
  children,
}: Props) {
  return (
    <Sheet
      open={open}
      onOpenChange={(next) => {
        if (!next) onClose();
      }}
    >
      <SheetContent
        side="right"
        showCloseButton={false}
        onEscapeKeyDown={onEscapeKeyDown}
        style={{
          width: width ?? "min(500px,100vw)",
          maxWidth: "none",
          top: 38,
          height: "calc(100% - 38px)",
        }}
        className="w-full gap-0 border-l border-border bg-[#0a0b0d] p-0"
      >
        <SheetTitle className="sr-only">{label}</SheetTitle>
        <SheetDescription className="sr-only">{label}</SheetDescription>
        <div className="drawer-content flex h-full min-h-0 flex-col gap-3 p-4">
          {children}
        </div>
      </SheetContent>
    </Sheet>
  );
}
