import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { SessionEvent, SessionEventKind } from "@/lib/types";
import { cn } from "@/lib/utils";

export type DetailTimelineItem = {
  index: number;
  event: SessionEvent;
  eventNumber: number;
};

type SegmentLayout = {
  index: number;
  topRatio: number;
  heightRatio: number;
};

type MinimapLayout = {
  contentStart: number;
  contentHeight: number;
  segments: SegmentLayout[];
};

const DEFAULT_SCROLL_ROOT = "[data-session-detail-scroll] [data-slot=scroll-area-viewport]";
const DEFAULT_LIST_SELECTOR = "[data-session-message-list]";
const MIN_SEGMENT_RATIO = 0.004;
const TRACK_HEIGHT_CLASS = "h-[min(72vh,28rem)]";

export function getScrollOffset(element: HTMLElement, scrollRoot: HTMLElement) {
  const elementRect = element.getBoundingClientRect();
  const rootRect = scrollRoot.getBoundingClientRect();
  return elementRect.top - rootRect.top + scrollRoot.scrollTop;
}

export function scrollToDetailMessage(
  index: number,
  listSelector = DEFAULT_LIST_SELECTOR,
  scrollRootSelector = DEFAULT_SCROLL_ROOT,
) {
  const scrollRoot = document.querySelector(scrollRootSelector);
  const item = document.querySelector(`${listSelector} [data-message-index="${index}"]`);
  if (!(scrollRoot instanceof HTMLElement) || !(item instanceof HTMLElement)) return false;

  const targetTop = getScrollOffset(item, scrollRoot);
  scrollRoot.scrollTo({ top: targetTop, behavior: "smooth" });
  return true;
}

function readable(value: string | null | undefined) {
  return value ? value.replaceAll("_", " ") : "unknown";
}

export function eventMinimapClassName(role: string | undefined, kind: SessionEventKind | undefined) {
  switch (role) {
    case "user":
      return "bg-[#b8c5d4] hover:bg-[#9eb0c4] dark:bg-[#4a5f75] dark:hover:bg-[#5a7088]";
    case "assistant":
      return "bg-[#d4d3cb] hover:bg-[#c0bfb6] dark:bg-[#5c5b55] dark:hover:bg-[#6d6c65]";
    case "tool":
      return "bg-[#d9c48a] hover:bg-[#c9b06f] dark:bg-[#7a6838] dark:hover:bg-[#8c7842]";
    case "system":
      return "bg-muted hover:bg-muted-foreground/25";
    case "developer":
      return "bg-[#c8bdd9] hover:bg-[#b5a7ca] dark:bg-[#5f5470] dark:hover:bg-[#6f6480]";
    default:
      if (kind === "action" || kind === "observation" || kind === "tool_call" || kind === "tool_result") {
        return "bg-[#d9c48a] hover:bg-[#c9b06f] dark:bg-[#7a6838] dark:hover:bg-[#8c7842]";
      }
      return "bg-border/80 hover:bg-border";
  }
}

function buildFallbackLayout(items: DetailTimelineItem[]): MinimapLayout {
  const count = Math.max(items.length, 1);
  const heightRatio = 1 / count;
  return {
    contentStart: 0,
    contentHeight: 1,
    segments: items.map((item, index) => ({
      index: item.index,
      topRatio: index * heightRatio,
      heightRatio: Math.max(heightRatio, MIN_SEGMENT_RATIO),
    })),
  };
}

function measureMinimapLayout(
  items: DetailTimelineItem[],
  scrollRoot: HTMLElement,
  listSelector: string,
): MinimapLayout | null {
  const list = document.querySelector(listSelector);
  if (!(list instanceof HTMLElement) || items.length === 0) return null;

  const measurements = items
    .map((item) => {
      const element = list.querySelector(`[data-message-index="${item.index}"]`);
      if (!(element instanceof HTMLElement)) return null;
      return {
        index: item.index,
        top: getScrollOffset(element, scrollRoot),
        height: element.offsetHeight,
      };
    })
    .filter((entry): entry is { index: number; top: number; height: number } => entry != null);

  if (measurements.length === 0) return null;

  const contentStart = measurements[0].top;
  const last = measurements[measurements.length - 1];
  const contentHeight = Math.max(last.top + last.height - contentStart, 1);

  return {
    contentStart,
    contentHeight,
    segments: measurements.map(({ index, top, height }) => ({
      index,
      topRatio: (top - contentStart) / contentHeight,
      heightRatio: Math.max(height / contentHeight, MIN_SEGMENT_RATIO),
    })),
  };
}

function clampRatio(value: number) {
  return Math.max(0, Math.min(1, value));
}

type DetailTimelineProps = {
  items: DetailTimelineItem[];
  className?: string;
  highlightedIndex?: number | null;
  scrollRootSelector?: string;
  listSelector?: string;
  onScrollToMessage: (index: number) => void;
};

export function DetailTimeline({
  items,
  className,
  highlightedIndex = null,
  scrollRootSelector = DEFAULT_SCROLL_ROOT,
  listSelector = DEFAULT_LIST_SELECTOR,
  onScrollToMessage,
}: DetailTimelineProps) {
  const trackRef = useRef<HTMLDivElement>(null);
  const [layout, setLayout] = useState<MinimapLayout | null>(null);
  const [viewport, setViewport] = useState({ topRatio: 0, heightRatio: 1 });
  const layoutRef = useRef<MinimapLayout | null>(null);

  const fallbackLayout = useMemo(() => buildFallbackLayout(items), [items]);
  const activeLayout = layout ?? fallbackLayout;

  const updateViewport = useCallback((scrollRoot: HTMLElement, nextLayout: MinimapLayout) => {
    const visibleTop = scrollRoot.scrollTop - nextLayout.contentStart;
    const visibleBottom = visibleTop + scrollRoot.clientHeight;
    const topRatio = clampRatio(visibleTop / nextLayout.contentHeight);
    const bottomRatio = clampRatio(visibleBottom / nextLayout.contentHeight);
    setViewport({
      topRatio,
      heightRatio: Math.max(bottomRatio - topRatio, 0.05),
    });
  }, []);

  useEffect(() => {
    layoutRef.current = layout;
  }, [layout]);

  useEffect(() => {
    const scrollRoot = document.querySelector(scrollRootSelector);
    if (!(scrollRoot instanceof HTMLElement)) return;

    let frame = 0;

    const measure = () => {
      window.cancelAnimationFrame(frame);
      frame = window.requestAnimationFrame(() => {
        const nextLayout = measureMinimapLayout(items, scrollRoot, listSelector) ?? buildFallbackLayout(items);
        setLayout(nextLayout);
        updateViewport(scrollRoot, nextLayout);
      });
    };

    measure();

    const list = document.querySelector(listSelector);
    const resizeObserver = new ResizeObserver(measure);
    if (list instanceof HTMLElement) resizeObserver.observe(list);
    for (const item of items) {
      const element = document.querySelector(`${listSelector} [data-message-index="${item.index}"]`);
      if (element instanceof HTMLElement) resizeObserver.observe(element);
    }

    const onScroll = () => {
      const currentLayout = layoutRef.current ?? buildFallbackLayout(items);
      updateViewport(scrollRoot, currentLayout);
    };

    scrollRoot.addEventListener("scroll", onScroll, { passive: true });
    window.addEventListener("resize", measure);

    return () => {
      window.cancelAnimationFrame(frame);
      resizeObserver.disconnect();
      scrollRoot.removeEventListener("scroll", onScroll);
      window.removeEventListener("resize", measure);
    };
  }, [items, listSelector, scrollRootSelector, updateViewport]);

  function scrollToRatio(ratio: number) {
    const scrollRoot = document.querySelector(scrollRootSelector);
    if (!(scrollRoot instanceof HTMLElement)) return null;
    const nextLayout = layoutRef.current ?? activeLayout;
    const clamped = clampRatio(ratio);
    const targetTop = nextLayout.contentStart + clamped * nextLayout.contentHeight;
    scrollRoot.scrollTo({ top: targetTop, behavior: "smooth" });
    return nextLayout.segments.find(
      (segment) => clamped >= segment.topRatio && clamped <= segment.topRatio + segment.heightRatio,
    ) ?? nextLayout.segments.slice().reverse().find((segment) => segment.topRatio <= clamped) ?? null;
  }

  function handleTrackPointer(event: React.PointerEvent<HTMLDivElement>) {
    const track = trackRef.current;
    if (!track) return;
    const rect = track.getBoundingClientRect();
    const ratio = (event.clientY - rect.top) / rect.height;
    const segment = scrollToRatio(ratio);
    if (segment) onScrollToMessage(segment.index);
  }

  if (items.length === 0) return null;

  return (
    <aside
      className={cn("hidden w-10 shrink-0 min-w-0 lg:block", className)}
      aria-label="Event minimap"
      data-detail-timeline
    >
      <div className={cn("sticky top-4 z-10 w-10 rounded-md border bg-muted/20 p-1", TRACK_HEIGHT_CLASS)}>
        <div
          ref={trackRef}
          className="relative h-full w-full cursor-pointer overflow-hidden rounded-sm bg-background/70"
          onPointerDown={handleTrackPointer}
        >
          {activeLayout.segments.map((segment) => {
            const item = items.find((entry) => entry.index === segment.index);
            if (!item) return null;
            const role = readable(item.event.role);
            const kind = readable(item.event.kind);
            const title = `#${item.eventNumber} ${role} · ${kind}`;

            return (
              <button
                key={item.event.id}
                type="button"
                className={cn(
                  "absolute left-0 w-full border-0 p-0 transition-opacity",
                  eventMinimapClassName(item.event.role, item.event.kind),
                  highlightedIndex === item.index && "ring-1 ring-foreground/50 ring-inset",
                )}
                style={{
                  top: `${segment.topRatio * 100}%`,
                  height: `${segment.heightRatio * 100}%`,
                  minHeight: "2px",
                }}
                title={title}
                aria-label={title}
                data-message-index={item.index}
                onPointerDown={(event) => event.stopPropagation()}
                onClick={() => onScrollToMessage(item.index)}
              />
            );
          })}

          <div
            aria-hidden
            className="pointer-events-none absolute inset-x-0 rounded-sm border border-foreground/35 bg-foreground/10"
            style={{
              top: `${viewport.topRatio * 100}%`,
              height: `${viewport.heightRatio * 100}%`,
            }}
            data-detail-timeline-viewport
          />
        </div>
      </div>
    </aside>
  );
}
