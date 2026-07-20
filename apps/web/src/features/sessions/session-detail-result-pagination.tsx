import { Button } from "@/components/ui/button";
import {
  Pagination,
  PaginationContent,
  PaginationItem,
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
  const totalPages = sessionEventTotalPages(totalCount, pageSize);
  const currentPage = Math.min(page, totalPages);
  const canGoBack = currentPage > 1;
  const canGoForward = currentPage < totalPages;

  return (
    <div
      className="flex shrink-0 flex-wrap items-center justify-between gap-3"
      data-session-detail-pagination
    >
      <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
        <span>{formatSessionEventListSummary(currentPage, pageSize, totalCount)} events</span>
        <span aria-hidden="true">·</span>
        <span>
          Page {currentPage} / {totalPages}
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
            aria-label="Events per page"
            data-session-detail-page-size
          >
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectGroup>
              {SESSION_EVENT_PAGE_SIZES.map((size) => (
                <SelectItem key={size} value={String(size)}>
                  {size} / page
                </SelectItem>
              ))}
            </SelectGroup>
          </SelectContent>
        </Select>

        <Pagination className="mx-0 w-auto justify-end">
          <PaginationContent>
            <PaginationItem>
              <PaginationPrevious
                href="#"
                text="Prev"
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
              <Button
                type="button"
                variant="outline"
                size="icon"
                className="min-h-10 min-w-10"
                disabled
                aria-current="page"
                aria-label={`Page ${currentPage}`}
              >
                {currentPage}
              </Button>
            </PaginationItem>
            <PaginationItem>
              <PaginationNext
                href="#"
                text="Next"
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
          </PaginationContent>
        </Pagination>
      </div>
    </div>
  );
}
