import { useState } from "react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  useSkillContext,
  useSkillContextSummary,
} from "@/features/skills/queries";
import { useI18n } from "@/lib/i18n-context";
import { cn } from "@/lib/utils";

const BASELINES = [32_000, 128_000, 200_000] as const;

export function SkillContextHealthPanel({
  skillId,
  provider,
  embedded = false,
}: {
  skillId: string | null;
  provider?: string;
  embedded?: boolean;
}) {
  const { t } = useI18n();
  const [baseline, setBaseline] = useState<number>();
  const contextSummary = useSkillContextSummary(provider, baseline);
  const context = useSkillContext(skillId, baseline);
  const selected = context.data;

  const baselineSelect = (
    <select
      aria-label={t("skillsContextBaseline")}
      value={baseline ?? ""}
      onChange={(event) =>
        setBaseline(event.target.value ? Number(event.target.value) : undefined)
      }
      className="h-8 rounded-md border bg-background px-2 text-sm"
    >
      <option value="">{t("skillsUnknownModel")}</option>
      {BASELINES.map((value) => (
        <option key={value} value={value}>
          {value / 1000}K
        </option>
      ))}
    </select>
  );

  const body = (
    <div className="space-y-3">
      <p className="text-muted-foreground text-xs">
        {t("skillsLocalEstimateAlgorithm", {
          algorithm: contextSummary.data?.algorithm_version ?? "unicode-mixed-v1",
        })}
      </p>
      {selected ? (
        <div className="grid grid-cols-1 gap-2 text-sm sm:grid-cols-3">
          {(
            [
              [t("skillsResidentMetadata"), selected.metadata],
              [t("skillsOnDemandBody"), selected.body],
              [t("skillsAuxiliaryFiles"), selected.auxiliary],
            ] as const
          ).map(([label, layer]) => (
            <div key={label} className="rounded-md border p-2">
              <div className="text-muted-foreground text-xs">{label}</div>
              <strong>≈ {layer.estimated_tokens.toLocaleString()}</strong>
              <div className="text-muted-foreground text-xs">
                {layer.token_lower}–{layer.token_upper} {t("skillsTokens")}
              </div>
              {layer.baseline_percent != null ? (
                <div className="mt-1 h-1.5 overflow-hidden rounded bg-muted">
                  <div
                    className="h-full bg-primary"
                    style={{
                      width: `${Math.min(layer.baseline_percent, 100)}%`,
                    }}
                  />
                </div>
              ) : null}
            </div>
          ))}
        </div>
      ) : (
        <p className="text-muted-foreground text-sm">{t("skillsSelectForBudget")}</p>
      )}
    </div>
  );

  if (embedded) {
    return (
      <div className={cn("space-y-3")}>
        <div className="flex items-center justify-between gap-3">
          <h3 className="text-sm font-semibold">{t("skillsContextBudget")}</h3>
          {baselineSelect}
        </div>
        {body}
      </div>
    );
  }

  return (
    <Card>
      <CardHeader className="flex-row items-center justify-between">
        <CardTitle>{t("skillsContextBudget")}</CardTitle>
        {baselineSelect}
      </CardHeader>
      <CardContent>{body}</CardContent>
    </Card>
  );
}
