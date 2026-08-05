import { useEffect, useState } from "react";

import { Input } from "@/components/ui/input";
import {
  Pagination,
  PaginationContent,
  PaginationFirst,
  PaginationItem,
  PaginationLast,
  PaginationNext,
  PaginationPrevious,
} from "@/components/ui/pagination";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  formatSessionEventListSummary,
  SESSION_EVENT_PAGE_SIZES,
  sessionEventTotalPages,
  type SessionEventPageSize,
} from "@/features/sessions/session-detail-pagination";
import { cn } from "@/lib/utils";
import { useI18n } from "@/lib/i18n-context";

type SessionDetailResultPaginationProps = {
  page: number;
  pageSize: SessionEventPageSize;
  totalCount: number;
  disabled?: boolean;
  onPageChange: (page: number) => void;
  onPageSizeChange: (pageSize: SessionEventPageSize) => void;
};

export function SessionDetailResultPagination({
  page,
  pageSize,
  totalCount,
  disabled = false,
  onPageChange,
  onPageSizeChange,
}: SessionDetailResultPaginationProps) {
  const { t } = useI18n();
  const totalPages = sessionEventTotalPages(totalCount, pageSize);
  const currentPage = Math.min(page, totalPages);
  const canGoBack = currentPage > 1;
  const canGoForward = currentPage < totalPages;
  const [pageInput, setPageInput] = useState(String(currentPage));

  useEffect(() => {
    setPageInput(String(currentPage));
  }, [currentPage]);

  const commitPageInput = () => {
    const parsed = Number.parseInt(pageInput, 10);
    if (!Number.isFinite(parsed)) {
      setPageInput(String(currentPage));
      return;
    }

    const nextPage = Math.min(Math.max(1, parsed), totalPages);
    setPageInput(String(nextPage));
    if (nextPage !== currentPage) {
      onPageChange(nextPage);
    }
  };

  return (
    <div
      className="flex shrink-0 flex-wrap items-center justify-between gap-3"
      data-session-detail-pagination
    >
      <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
        <span>{formatSessionEventListSummary(currentPage, pageSize, totalCount)} {t("events")}</span>
        <span aria-hidden="true">·</span>
        <span>
          {t("sessionPageOf", { page: currentPage, total: totalPages })}
        </span>
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <Select
          value={String(pageSize)}
          onValueChange={(value) => onPageSizeChange(Number(value) as SessionEventPageSize)}
          disabled={disabled}
        >
          <SelectTrigger
            className="min-h-10 w-[7.5rem]"
            aria-label={t("eventsPerPage")}
            data-session-detail-page-size
          >
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectGroup>
              {SESSION_EVENT_PAGE_SIZES.map((size) => (
                <SelectItem key={size} value={String(size)}>
                  {t("perPage", { count: size })}
                </SelectItem>
              ))}
            </SelectGroup>
          </SelectContent>
        </Select>

        <Pagination className="mx-0 w-auto justify-end">
          <PaginationContent>
            <PaginationItem>
              <PaginationFirst
                href="#"
                aria-disabled={!canGoBack || disabled}
                className={cn(
                  (!canGoBack || disabled) && "pointer-events-none opacity-50",
                )}
                data-session-detail-page-first
                onClick={(event) => {
                  event.preventDefault();
                  if (!canGoBack || disabled) return;
                  onPageChange(1);
                }}
              />
            </PaginationItem>
            <PaginationItem>
              <PaginationPrevious
                href="#"
                text={t("previousPage")}
                aria-disabled={!canGoBack || disabled}
                className={cn(
                  "min-h-10",
                  (!canGoBack || disabled) && "pointer-events-none opacity-50",
                )}
                data-session-detail-page-prev
                onClick={(event) => {
                  event.preventDefault();
                  if (!canGoBack || disabled) return;
                  onPageChange(currentPage - 1);
                }}
              />
            </PaginationItem>
            <PaginationItem>
              <Input
                type="number"
                min={1}
                max={totalPages}
                inputMode="numeric"
                value={pageInput}
                disabled={disabled || totalPages <= 1}
                aria-label={t("goToPage")}
                data-session-detail-page-jump
                className="h-10 w-14 px-1 text-center [appearance:textfield] [&::-webkit-inner-spin-button]:appearance-none [&::-webkit-outer-spin-button]:appearance-none"
                onChange={(event) => setPageInput(event.target.value)}
                onBlur={commitPageInput}
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    event.preventDefault();
                    commitPageInput();
                  }
                }}
              />
            </PaginationItem>
            <PaginationItem>
              <PaginationNext
                href="#"
                text={t("nextPage")}
                aria-disabled={!canGoForward || disabled}
                className={cn(
                  "min-h-10",
                  (!canGoForward || disabled) && "pointer-events-none opacity-50",
                )}
                data-session-detail-page-next
                onClick={(event) => {
                  event.preventDefault();
                  if (!canGoForward || disabled) return;
                  onPageChange(currentPage + 1);
                }}
              />
            </PaginationItem>
            <PaginationItem>
              <PaginationLast
                href="#"
                aria-disabled={!canGoForward || disabled}
                className={cn(
                  (!canGoForward || disabled) && "pointer-events-none opacity-50",
                )}
                data-session-detail-page-last
                onClick={(event) => {
                  event.preventDefault();
                  if (!canGoForward || disabled) return;
                  onPageChange(totalPages);
                }}
              />
            </PaginationItem>
          </PaginationContent>
        </Pagination>
      </div>
    </div>
  );
}
