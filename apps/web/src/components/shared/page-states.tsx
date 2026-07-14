import { FolderSearchIcon, RefreshCwIcon } from "lucide-react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import { Skeleton } from "@/components/ui/skeleton";

export function PageSkeleton() {
  return (
    <div className="flex flex-col gap-4">
      <Skeleton className="h-8 w-64" />
      <Skeleton className="h-28 w-full" />
      <Skeleton className="h-48 w-full" />
    </div>
  );
}

export function PageEmpty({
  title,
  description,
  onRefresh,
}: {
  title: string;
  description: string;
  onRefresh?: () => void;
}) {
  return (
    <Empty>
      <EmptyHeader>
        <EmptyMedia variant="icon">
          <FolderSearchIcon />
        </EmptyMedia>
        <EmptyTitle>{title}</EmptyTitle>
        <EmptyDescription>{description}</EmptyDescription>
      </EmptyHeader>
      {onRefresh ? (
        <EmptyContent>
          <Button variant="outline" className="min-h-10" onClick={onRefresh}>
            <RefreshCwIcon data-icon="inline-start" />
            Refresh
          </Button>
        </EmptyContent>
      ) : null}
    </Empty>
  );
}

export function PageError({
  title,
  message,
  onRetry,
}: {
  title: string;
  message: string;
  onRetry?: () => void;
}) {
  return (
    <Alert variant="destructive">
      <AlertTitle>{title}</AlertTitle>
      <AlertDescription className="flex flex-col items-start gap-3">
        <span>{message}</span>
        {onRetry ? (
          <Button
            type="button"
            variant="outline"
            className="min-h-10"
            onClick={onRetry}
          >
            <RefreshCwIcon data-icon="inline-start" />
            Retry
          </Button>
        ) : null}
      </AlertDescription>
    </Alert>
  );
}
