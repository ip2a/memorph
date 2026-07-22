import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

export function SessionEventSplitRow({
  className,
  left,
  right,
  ...props
}: {
  className?: string;
  left: ReactNode;
  right: ReactNode;
} & React.ComponentProps<"div">) {
  return (
    <div
      className={cn(
        "grid w-full min-w-0 items-stretch gap-5 max-lg:grid-cols-1 lg:grid-cols-[minmax(0,2fr)_minmax(0,3fr)]",
        className,
      )}
      {...props}
    >
      <div className="flex min-w-0 items-center max-lg:items-start">{left}</div>
      <div className="min-w-0">{right}</div>
    </div>
  );
}
