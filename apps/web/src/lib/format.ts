import type { SessionItem } from "@/lib/types";

export function sessionTitle(session: SessionItem) {
  return session.display_title || session.title || session.native_title || session.session_id;
}

export function formatDateTime(value: number | string | null | undefined) {
  if (value === null || value === undefined || value === "") return "-";
  const date = typeof value === "number" ? new Date(value > 100_000_000_000 ? value : value * 1000) : new Date(value);
  if (Number.isNaN(date.getTime())) return "-";
  return new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  }).format(date);
}

export function formatBytes(value: number | null | undefined) {
  if (!value) return "-";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let size = value;
  let unit = 0;
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit += 1;
  }
  const digits = size >= 10 || unit === 0 ? 0 : 1;
  return `${size.toFixed(digits)} ${units[unit]}`;
}

export function compactPath(value: string | null | undefined) {
  if (!value) return "-";
  const segments = value.split(/[\\/]/).filter(Boolean);
  if (segments.length <= 3) return value;
  return `.../${segments.slice(-3).join("/")}`;
}

export function formatDetailTitle(value: string) {
  return value.replaceAll("/", "/\u200b");
}
