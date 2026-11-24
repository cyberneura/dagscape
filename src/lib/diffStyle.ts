import type { DiffLine, RefKind } from "./api";

export function statusClass(status: string): string {
  switch (status) {
    case "A":
      return "text-emerald-600 dark:text-emerald-400";
    case "D":
      return "text-red-600 dark:text-red-400";
    case "R":
    case "C":
      return "text-sky-600 dark:text-sky-400";
    default:
      return "text-amber-600 dark:text-amber-400";
  }
}

export function lineClass(kind: DiffLine["kind"]): string {
  switch (kind) {
    case "add":
      return "bg-emerald-500/15 text-emerald-800 dark:text-emerald-200";
    case "del":
      return "bg-red-500/15 text-red-800 dark:text-red-200";
    case "hunk":
      return "bg-sky-500/10 text-sky-700 dark:text-sky-300";
    default:
      return "";
  }
}

export function chipClass(kind: RefKind): string {
  switch (kind) {
    case "head":
      return "bg-neutral-800 text-white dark:bg-neutral-200 dark:text-black";
    case "branch":
      return "bg-emerald-600/85 text-white";
    case "remote":
      return "bg-sky-600/85 text-white";
    case "tag":
      return "bg-amber-400 text-black";
  }
}
