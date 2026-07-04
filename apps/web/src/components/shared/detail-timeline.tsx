import type { SessionEvent } from "@/lib/types";
import { formatBytes } from "@/lib/format";
import { cn } from "@/lib/utils";

export function estimateEventSize(event: SessionEvent) {
  const sizeBytes = (event.metadata as { size_bytes?: number | null }).size_bytes;
  if (sizeBytes != null) return Number(sizeBytes) || 0;

  return (event.blocks ?? []).reduce((sum, block) => {
    if (block == null) return sum;
    if ("text" in block && block.text != null) return sum + String(block.text).length;
    if ("content" in block && block.content != null) return sum + String(block.content).length;
    if ("diff_text" in block && block.diff_text != null) return sum + String(block.diff_text).length;
    if ("payload" in block && block.payload != null) return sum + JSON.stringify(block.payload).length;
    if ("raw" in block && block.raw != null) return sum + JSON.stringify(block.raw).length;
    return sum + JSON.stringify(block).length;
  }, 0);
}

export function scrollToDetailMessage(index: number, listSelector = "[data-session-message-list]") {
  const item = document.querySelector(`${listSelector} [data-message-index="${index}"]`);
  if (!item) return false;
  item.scrollIntoView({ behavior: "smooth", block: "start" });
  return true;
}

type DetailTimelineProps = {
  events: SessionEvent[];
  className?: string;
  onScrollToMessage: (index: number) => void;
};

export function DetailTimeline({ events, className, onScrollToMessage }: DetailTimelineProps) {
  const sizes = events.map(estimateEventSize);
  const total = sizes.reduce((sum, size) => sum + size, 0) || 1;
  const max = Math.max(1, ...sizes);

  return (
    <aside
      className={cn("hidden min-h-0 w-5 flex-col overflow-hidden border bg-background lg:flex", className)}
      aria-label="Timeline"
      data-detail-timeline
    >
      {sizes.map((size, index) => {
        const ratio = size / total;
        const intensity = max ? size / max : 0;
        const depth = Math.max(0.08, Math.min(0.85, 0.12 + intensity * 0.7));
        const flexGrow = Math.max(0.02, ratio);

        return (
          <button
            key={events[index]?.id ?? index}
            type="button"
            className="min-h-[3px] w-full shrink-0 border-0 border-b border-background/90 p-0 last:border-b-0 hover:!bg-foreground focus-visible:!bg-foreground focus-visible:outline-none"
            style={{
              flex: `${flexGrow} 1 0`,
              backgroundColor: `color-mix(in oklch, var(--foreground) ${Math.round(depth * 100)}%, transparent)`,
            }}
            title={formatBytes(size)}
            data-message-index={index}
            onClick={() => onScrollToMessage(index)}
          />
        );
      })}
    </aside>
  );
}
