import { forwardRef, type ComponentProps, type ReactNode } from "react";

import { cn } from "@/lib/utils";

type ScrollPaneProps = ComponentProps<"div"> & {
  innerClassName?: string;
  children: ReactNode;
};

/** Panel scroll container with native scrolling and no visible scrollbar track. */
export const ScrollPane = forwardRef<HTMLDivElement, ScrollPaneProps>(function ScrollPane(
  { className, innerClassName, children, ...props },
  ref,
) {
  return (
    <div
      ref={ref}
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
});
