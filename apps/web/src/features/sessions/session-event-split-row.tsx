import { useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { cn } from "@/lib/utils";

export function SessionEventSplitRow({
  className,
  left,
  right,
  syncKey,
  ...props
}: {
  className?: string;
  left: ReactNode;
  right: ReactNode;
  syncKey?: string;
} & React.ComponentProps<"div">) {
  const leftRef = useRef<HTMLDivElement>(null);
  const [leftHeight, setLeftHeight] = useState<number | null>(null);
  const [syncHeight, setSyncHeight] = useState(false);

  useLayoutEffect(() => {
    const media = window.matchMedia("(min-width: 1024px)");
    const updateSync = () => setSyncHeight(media.matches);
    updateSync();
    media.addEventListener("change", updateSync);
    return () => media.removeEventListener("change", updateSync);
  }, []);

  useLayoutEffect(() => {
    const node = leftRef.current;
    if (!node) return;

    const update = () => setLeftHeight(node.offsetHeight);
    update();

    const observer = new ResizeObserver(update);
    observer.observe(node);
    return () => observer.disconnect();
  }, [syncKey]);

  return (
    <div
      className={cn(
        "grid w-full min-w-0 items-start gap-3 max-lg:grid-cols-1 lg:grid-cols-[minmax(0,2fr)_minmax(0,3fr)]",
        className,
      )}
      {...props}
    >
      <div ref={leftRef} className="min-w-0">
        {left}
      </div>
      <div
        className="min-h-0 min-w-0 max-lg:h-auto"
        style={syncHeight && leftHeight != null ? { height: leftHeight } : undefined}
      >
        {right}
      </div>
    </div>
  );
}
