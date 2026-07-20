import {
  clampSessionEventPageSize,
  DEFAULT_SESSION_EVENT_PAGE_SIZE,
  readSessionEventPage,
  type SessionEventPageSize,
} from "@/features/sessions/session-detail-pagination";

export type SessionDetailRouteState = {
  page: number;
  pageSize: SessionEventPageSize;
};

export function readSessionDetailRouteState(searchParams: URLSearchParams): SessionDetailRouteState {
  return {
    page: readSessionEventPage(searchParams),
    pageSize: clampSessionEventPageSize(searchParams.get("pageSize")),
  };
}

export function writeSessionDetailRouteState(
  searchParams: URLSearchParams,
  next: Partial<SessionDetailRouteState>,
): URLSearchParams {
  const params = new URLSearchParams(searchParams);
  const page = next.page ?? readSessionEventPage(params);
  const pageSize = next.pageSize ?? clampSessionEventPageSize(params.get("pageSize"));

  if (page <= 1) params.delete("page");
  else params.set("page", String(page));

  if (pageSize === DEFAULT_SESSION_EVENT_PAGE_SIZE) params.delete("pageSize");
  else params.set("pageSize", String(pageSize));

  return params;
}
