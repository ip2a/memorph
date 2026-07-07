import type { SessionEvent } from "@/lib/types";

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
