import { DotIcon, WrenchIcon } from "lucide-react";
import { ChainOfThoughtStep } from "@/components/ai-elements/chain-of-thought";
import { Badge } from "@/components/ui/badge";
import { SessionBlock } from "@/features/sessions/session-block";
import { SessionContent } from "@/features/sessions/session-content";
import {
  blocksToChainSteps,
  type ChainStep,
} from "@/features/sessions/session-chain-of-thought-utils";
import type { EventBlock } from "@/lib/types";
import { useI18n } from "@/lib/i18n-context";

function ChainStepContent({ step }: { step: ChainStep }) {
  if (step.kind === "thinking" && step.thinkingText) {
    return <SessionContent value={step.thinkingText} />;
  }

  return <SessionBlock block={step.block} embedded />;
}

function stepLabel(step: ChainStep, errorLabel: string) {
  if (step.kind === "tool_result" && step.block.type === "tool_result" && step.block.is_error) {
    return (
      <span className="inline-flex flex-wrap items-center gap-2">
        {step.label}
        <Badge variant="destructive" className="px-1.5 py-0 text-[10px] font-normal">
          {errorLabel}
        </Badge>
      </span>
    );
  }

  return step.label;
}

export function SessionChainOfThought({ blocks }: { blocks: EventBlock[] }) {
  const { t } = useI18n();
  const steps = blocksToChainSteps(blocks, t);
  if (steps.length === 0) return null;

  return (
    <div className="not-prose w-full space-y-3" data-chain-of-thought>
      {steps.map((step) => (
        <ChainOfThoughtStep
          key={step.id}
          data-chain-step={step.kind}
          icon={step.kind === "tool_call" ? WrenchIcon : DotIcon}
          label={stepLabel(step, t("chainError"))}
          description={step.description}
          status="complete"
        >
          <ChainStepContent step={step} />
        </ChainOfThoughtStep>
      ))}
    </div>
  );
}
