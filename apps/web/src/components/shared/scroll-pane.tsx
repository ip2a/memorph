import type { ComponentProps, ReactNode } from "react";

import { cn } from "@/lib/utils";

type ScrollPaneProps = ComponentProps<"div"> & {
  innerClassName?: string;
  children: ReactNode;
};

/** Panel scroll container with native scrolling and no visible scrollbar track. */
export function ScrollPane({ className, innerClassName, children, ...props }: ScrollPaneProps) {
  return (
    <div
      className={cn(
        "min-h-0 size-full overflow-x-hidden overflow-y-auto",
        "[scrollbar-width:none] [-ms-overflow-style:none] [&::-webkit-scrollbar]:hidden",
        className,
      )}
      {...props}
    >
      <div className={cn("pe-1", innerClassName)}>{children}</div>
    </div>
  );
}
