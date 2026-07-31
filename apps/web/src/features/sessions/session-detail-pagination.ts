export const SESSION_EVENT_PAGE_SIZES = [10, 20, 50, 80] as const;

export type SessionEventPageSize = (typeof SESSION_EVENT_PAGE_SIZES)[number];

export const DEFAULT_SESSION_EVENT_PAGE_SIZE: SessionEventPageSize = 20;

export function clampSessionEventPageSize(
  value: number | string | null | undefined,
): SessionEventPageSize {
  const parsed = Number(value);
  if (
    SESSION_EVENT_PAGE_SIZES.includes(parsed as SessionEventPageSize) &&
    Number.isFinite(parsed)
  ) {
    return parsed as SessionEventPageSize;
  }
  return DEFAULT_SESSION_EVENT_PAGE_SIZE;
}

export function readSessionEventPage(searchParams: URLSearchParams): number {
  const parsed = Number.parseInt(searchParams.get("page") ?? "1", 10);
  return Number.isFinite(parsed) && parsed >= 1 ? parsed : 1;
}

export {
  formatManagerListSummary as formatSessionEventListSummary,
  managerPageRange as sessionEventPageRange,
  managerTotalPages as sessionEventTotalPages,
} from "@/features/manager/manager-pagination";

export function buildSessionEventQuery(
  page: number,
  pageSize: SessionEventPageSize,
  eventSearch?: string,
  eventOrder: "asc" | "desc" = "asc",
) {
  const query: {
    event_offset: number;
    event_limit: number;
    event_search?: string;
    event_order?: "asc" | "desc";
  } = {
    event_offset: (page - 1) * pageSize,
    event_limit: pageSize,
  };
  const trimmed = eventSearch?.trim();
  if (trimmed) query.event_search = trimmed;
  if (eventOrder === "desc") query.event_order = "desc";
  return query;
}
