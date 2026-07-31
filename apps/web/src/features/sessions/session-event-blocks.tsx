import { SessionBlock } from "@/features/sessions/session-block";
import { SessionChainOfThought } from "@/features/sessions/session-chain-of-thought";
import { segmentEventBlocks } from "@/features/sessions/session-chain-of-thought-utils";
import type { EventBlock } from "@/lib/types";

export function SessionEventBlocks({
  blocks,
  eventId,
}: {
  blocks: EventBlock[];
  eventId: string;
}) {
  const segments = segmentEventBlocks(blocks);

  return (
    <div className="flex flex-col gap-3" data-session-event-blocks>
      {segments.map((segment, index) => {
        if (segment.kind === "chain") {
          return (
            <SessionChainOfThought
              key={`${eventId}-chain-${index}`}
              blocks={segment.blocks}
            />
          );
        }

        return <SessionBlock key={`${eventId}-block-${index}`} block={segment.block} />;
      })}
    </div>
  );
}
