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

export function formatNumericDateTime(value: number | string | null | undefined) {
  if (value === null || value === undefined || value === "") return "-";
  const date = typeof value === "number" ? new Date(value > 100_000_000_000 ? value : value * 1000) : new Date(value);
  if (Number.isNaN(date.getTime())) return "-";
  const pad = (part: number) => String(part).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

function parseDateValue(value: number | string | null | undefined) {
  if (value === null || value === undefined || value === "") return null;
  const date = typeof value === "number" ? new Date(value > 100_000_000_000 ? value : value * 1000) : new Date(value);
  return Number.isNaN(date.getTime()) ? null : date;
}

export function formatChartAxisDateTime(
  value: number | string | null | undefined,
  rangeStart: number | string | null | undefined,
  rangeEnd: number | string | null | undefined,
) {
  const date = parseDateValue(value);
  const start = parseDateValue(rangeStart);
  const end = parseDateValue(rangeEnd);
  if (!date) return "-";
  const pad = (part: number) => String(part).padStart(2, "0");
  const time = `${pad(date.getHours())}:${pad(date.getMinutes())}`;
  if (!start || !end) return time;
  const spanMs = Math.abs(end.getTime() - start.getTime());
  if (spanMs <= 24 * 60 * 60 * 1000) return time;
  return `${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${time}`;
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

export function stripAnsi(value: string) {
  return value.replace(/\u001b\[[0-9;]*m/g, "");
}

export function formatExecutableVersion(value: string | null | undefined) {
  if (!value) return null;
  const cleaned = stripAnsi(value).replace(/\s+/g, " ").trim();
  const firstLine = cleaned.split(/\r?\n/, 1)[0]?.trim();
  return firstLine || null;
}
