import type { Environment } from "@/lib/types";

export const ENV_DOT: Record<Environment, string> = {
  dev: "bg-emerald-400",
  staging: "bg-amber-400",
  prod: "bg-red-400",
};

export const ENV_BADGE: Record<Environment, string> = {
  dev: "bg-emerald-400/10 text-emerald-300",
  staging: "bg-amber-400/10 text-amber-300",
  prod: "bg-red-400/10 text-red-300",
};
