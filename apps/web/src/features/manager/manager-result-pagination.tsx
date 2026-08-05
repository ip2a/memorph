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
  MANAGER_PAGE_SIZES,
  type ManagerPageSize,
  managerPageRange,
  managerTotalPages,
} from "@/features/manager/manager-pagination";
import { useI18n } from "@/lib/i18n-context";
import { cn } from "@/lib/utils";

type ManagerResultPaginationProps = {
  page: number;
  pageSize: ManagerPageSize;
  totalCount: number;
  disabled?: boolean;
  onPageChange: (page: number) => void;
  onPageSizeChange: (pageSize: ManagerPageSize) => void;
};

export function ManagerResultPagination({
  page,
  pageSize,
  totalCount,
  disabled = false,
  onPageChange,
  onPageSizeChange,
}: ManagerResultPaginationProps) {
  const { t } = useI18n();
  const totalPages = managerTotalPages(totalCount, pageSize);
  const currentPage = Math.min(page, totalPages);
  const range = managerPageRange(currentPage, pageSize, totalCount);
  const canGoBack = currentPage > 1;
  const canGoForward = currentPage < totalPages;

  return (
    <div
      className="flex shrink-0 flex-wrap items-center justify-between gap-3 border-t pt-3"
      data-manager-pagination
    >
      <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
        <span>
          {t("managerResultsSummary", {
            from: range.from,
            to: range.to,
            total: totalCount,
          })}
        </span>
        <span aria-hidden="true">·</span>
        <span>
          {t("managerPageOf", { page: currentPage, total: totalPages })}
        </span>
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <Select
          value={String(pageSize)}
          onValueChange={(value) =>
            onPageSizeChange(Number(value) as ManagerPageSize)
          }
          disabled={disabled}
        >
          <SelectTrigger
            className="min-h-10 w-[7.5rem]"
            aria-label={t("managerResultsPerPage")}
            data-manager-page-size
          >
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectGroup>
              {MANAGER_PAGE_SIZES.map((size) => (
                <SelectItem key={size} value={String(size)}>
                  {t("managerPerPage", { size })}
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
                text={t("managerPrevious")}
                aria-disabled={!canGoBack || disabled}
                className={cn(
                  "min-h-10",
                  (!canGoBack || disabled) && "pointer-events-none opacity-50",
                )}
                data-manager-page-prev
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
                aria-label={t("managerPageNumber", { page: currentPage })}
              >
                {currentPage}
              </Button>
            </PaginationItem>
            <PaginationItem>
              <PaginationNext
                href="#"
                text={t("managerNext")}
                aria-disabled={!canGoForward || disabled}
                className={cn(
                  "min-h-10",
                  (!canGoForward || disabled) && "pointer-events-none opacity-50",
                )}
                data-manager-page-next
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
