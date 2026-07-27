import type { ButtonHTMLAttributes, ReactNode } from "react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

type SelectableRowButtonProps = Omit<ButtonHTMLAttributes<HTMLButtonElement>, "title"> & {
  leading?: ReactNode;
  meta?: ReactNode;
  details?: ReactNode;
  selected?: boolean;
  title: ReactNode;
  trailing?: ReactNode;
};

export function SelectableRowButton({
  className,
  leading,
  meta,
  details,
  selected = false,
  title,
  trailing,
  ...props
}: SelectableRowButtonProps) {
  const body =
    meta || details ? (
      <span className="flex min-w-0 flex-col gap-1">
        <strong className="truncate text-sm font-medium">{title}</strong>
        {meta ? (
          <span className="text-muted-foreground truncate text-xs">{meta}</span>
        ) : null}
        {details}
      </span>
    ) : (
      <strong className="truncate text-sm font-medium">{title}</strong>
    );

  return (
    <Button
      type="button"
      variant={selected ? "secondary" : "outline"}
      className={cn("h-auto min-h-11 w-full justify-start px-3 py-2 text-left", className)}
      {...props}
    >
      <span
        className={cn(
          "grid w-full min-w-0 items-center gap-3",
          leading ? "grid-cols-[auto_minmax(0,1fr)_auto]" : "grid-cols-[minmax(0,1fr)_auto]",
        )}
      >
        {leading}
        {body}
        {trailing}
      </span>
    </Button>
  );
}
