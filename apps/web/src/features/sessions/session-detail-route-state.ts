import {
  clampSessionEventPageSize,
  DEFAULT_SESSION_EVENT_PAGE_SIZE,
  readSessionEventPage,
  type SessionEventPageSize,
} from "@/features/sessions/session-detail-pagination";
import type { SessionEventOrder } from "@/lib/types";

export type SessionDetailRouteState = {
  page: number;
  pageSize: SessionEventPageSize;
  eventSearch: string;
  eventOrder: SessionEventOrder;
};

export function readSessionEventOrder(value: string | null | undefined): SessionEventOrder {
  return value === "desc" ? "desc" : "asc";
}

export function readSessionDetailRouteState(searchParams: URLSearchParams): SessionDetailRouteState {
  return {
    page: readSessionEventPage(searchParams),
    pageSize: clampSessionEventPageSize(searchParams.get("pageSize")),
    eventSearch: searchParams.get("q") ?? "",
    eventOrder: readSessionEventOrder(searchParams.get("order")),
  };
}

export function writeSessionDetailRouteState(
  searchParams: URLSearchParams,
  next: Partial<SessionDetailRouteState>,
): URLSearchParams {
  const params = new URLSearchParams(searchParams);
  const page = next.page ?? readSessionEventPage(params);
  const pageSize = next.pageSize ?? clampSessionEventPageSize(params.get("pageSize"));
  const eventSearch = next.eventSearch ?? (params.get("q") ?? "");
  const eventOrder = next.eventOrder ?? readSessionEventOrder(params.get("order"));

  if (page <= 1) params.delete("page");
  else params.set("page", String(page));

  if (pageSize === DEFAULT_SESSION_EVENT_PAGE_SIZE) params.delete("pageSize");
  else params.set("pageSize", String(pageSize));

  const trimmedSearch = eventSearch.trim();
  if (trimmedSearch) params.set("q", trimmedSearch);
  else params.delete("q");

  if (eventOrder === "desc") params.set("order", "desc");
  else params.delete("order");

  return params;
}
