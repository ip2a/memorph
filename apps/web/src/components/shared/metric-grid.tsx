import type { HTMLAttributes, ReactNode } from "react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

type MetricGridProps = HTMLAttributes<HTMLDivElement> & {
  columns?: "auto" | "two" | "three" | "four";
};

type MetricTileProps = {
  active?: boolean;
  className?: string;
  hint?: ReactNode;
  label: ReactNode;
  onClick?: () => void;
  value: ReactNode;
  variant?: "bordered" | "compact" | "plain";
};

const gridColumns = {
  auto: "grid gap-3 sm:grid-cols-2 xl:grid-cols-4",
  two: "grid grid-cols-2 gap-3",
  three: "grid grid-cols-3 gap-2",
  four: "grid gap-3 sm:grid-cols-2 xl:grid-cols-4",
};

export function MetricGrid({ columns = "auto", className, ...props }: MetricGridProps) {
  return <div className={cn(gridColumns[columns], className)} {...props} />;
}

export function MetricTile({
  active = false,
  className,
  hint,
  label,
  onClick,
  value,
  variant = "plain",
}: MetricTileProps) {
  if (variant === "bordered") {
    return (
      <div className={cn("rounded-md border p-2", className)}>
        <span className="flex min-w-0 flex-col items-center gap-1 text-center">
          <strong className="truncate text-lg font-semibold">{value ?? "-"}</strong>
          <span className="text-muted-foreground text-xs">{label}</span>
          {hint ? <span className="text-muted-foreground truncate font-mono text-xs">{hint}</span> : null}
        </span>
      </div>
    );
  }

  const content = (
    <span className="flex min-w-0 flex-col items-start gap-1 text-left">
      <span className="text-muted-foreground truncate font-mono text-xs uppercase">{label}</span>
      <strong className={cn("truncate font-semibold", variant === "compact" ? "text-base" : "text-sm")}>{value ?? "-"}</strong>
      {hint ? <span className="text-muted-foreground truncate font-mono text-xs">{hint}</span> : null}
    </span>
  );

  if (onClick) {
    return (
      <Button
        type="button"
        variant={active ? "secondary" : "outline"}
        className={cn("h-auto justify-start px-3 py-2", className)}
        onClick={onClick}
      >
        {content}
      </Button>
    );
  }

  return (
    <div
      className={cn(
        variant === "compact" && "rounded-md border px-3 py-2",
        variant === "plain" && "border-b pb-3",
        className,
      )}
    >
      {content}
    </div>
  );
}
