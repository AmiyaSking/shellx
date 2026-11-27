import { useEffect, useState, type ReactNode } from "react";

interface Props {
  open: boolean;
  /** Should match the sheet/dialog exit animation. */
  exitMs?: number;
  children: ReactNode;
}

/**
 * Keeps a closed panel mounted long enough for its exit animation to finish.
 */
export default function KeepMounted({ open, exitMs = 240, children }: Props) {
  const [mounted, setMounted] = useState(open);

  useEffect(() => {
    if (open) {
      setMounted(true);
      return;
    }
    if (!mounted) return;
    const timer = window.setTimeout(() => setMounted(false), exitMs);
    return () => window.clearTimeout(timer);
  }, [open, mounted, exitMs]);

  return mounted ? <>{children}</> : null;
}
