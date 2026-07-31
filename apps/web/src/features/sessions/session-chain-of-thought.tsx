import { DotIcon, WrenchIcon } from "lucide-react";
import {
  ChainOfThought,
  ChainOfThoughtContent,
  ChainOfThoughtHeader,
  ChainOfThoughtStep,
} from "@/components/ai-elements/chain-of-thought";
import { Badge } from "@/components/ui/badge";
import { SessionBlock } from "@/features/sessions/session-block";
import { SessionContent } from "@/features/sessions/session-content";
import {
  blocksToChainSteps,
  type ChainStep,
} from "@/features/sessions/session-chain-of-thought-utils";
import type { EventBlock } from "@/lib/types";

function ChainStepContent({ step }: { step: ChainStep }) {
  if (step.kind === "thinking" && step.thinkingText) {
    return <SessionContent value={step.thinkingText} />;
  }

  return <SessionBlock block={step.block} embedded />;
}

function stepLabel(step: ChainStep) {
  if (step.kind === "tool_result" && step.block.type === "tool_result" && step.block.is_error) {
    return (
      <span className="inline-flex flex-wrap items-center gap-2">
        {step.label}
        <Badge variant="destructive" className="px-1.5 py-0 text-[10px] font-normal">
          Error
        </Badge>
      </span>
    );
  }

  return step.label;
}

export function SessionChainOfThought({
  blocks,
  defaultOpen = false,
}: {
  blocks: EventBlock[];
  defaultOpen?: boolean;
}) {
  const steps = blocksToChainSteps(blocks);
  if (steps.length === 0) return null;

  return (
    <ChainOfThought defaultOpen={defaultOpen} data-chain-of-thought>
      <ChainOfThoughtHeader>Chain of thought</ChainOfThoughtHeader>
      <ChainOfThoughtContent>
        {steps.map((step) => (
          <ChainOfThoughtStep
            key={step.id}
            data-chain-step={step.kind}
            icon={step.kind === "tool_call" ? WrenchIcon : DotIcon}
            label={stepLabel(step)}
            description={step.description}
            status="complete"
          >
            <ChainStepContent step={step} />
          </ChainOfThoughtStep>
        ))}
      </ChainOfThoughtContent>
    </ChainOfThought>
  );
}
