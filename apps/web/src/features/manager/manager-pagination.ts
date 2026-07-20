export const MANAGER_PAGE_SIZES = [10, 20, 50, 70] as const;

export type ManagerPageSize = (typeof MANAGER_PAGE_SIZES)[number];

export const DEFAULT_MANAGER_PAGE_SIZE: ManagerPageSize = 20;

export function clampManagerPageSize(
  value: number | string | null | undefined,
): ManagerPageSize {
  const parsed = Number(value);
  if (
    MANAGER_PAGE_SIZES.includes(parsed as ManagerPageSize) &&
    Number.isFinite(parsed)
  ) {
    return parsed as ManagerPageSize;
  }
  return DEFAULT_MANAGER_PAGE_SIZE;
}

export function readManagerPage(
  searchParams: URLSearchParams,
): number {
  const parsed = Number.parseInt(searchParams.get("page") ?? "1", 10);
  return Number.isFinite(parsed) && parsed >= 1 ? parsed : 1;
}

export function managerTotalPages(totalCount: number, pageSize: number): number {
  if (totalCount <= 0) return 1;
  return Math.max(1, Math.ceil(totalCount / pageSize));
}

export function managerPageRange(
  page: number,
  pageSize: number,
  totalCount: number,
): { from: number; to: number } {
  if (totalCount <= 0) {
    return { from: 0, to: 0 };
  }
  const from = (page - 1) * pageSize + 1;
  const to = Math.min(page * pageSize, totalCount);
  return { from, to };
}

export function formatManagerListSummary(
  page: number,
  pageSize: number,
  totalCount: number,
): string {
  const { from, to } = managerPageRange(page, pageSize, totalCount);
  if (totalCount === 0) return "0 results";
  return `${from}–${to} of ${totalCount}`;
}
