import { useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  useSkillContext,
  useSkillContextSummary,
  useSkillHealth,
  useSkillHealthSummary,
} from "@/features/skills/queries";
import { useI18n } from "@/lib/i18n-context";

const BASELINES = [32_000, 128_000, 200_000] as const;

export function SkillContextHealthPanel({
  skillId,
  provider,
}: {
  skillId: string | null;
  provider?: string;
}) {
  const { t } = useI18n();
  const [baseline, setBaseline] = useState<number>();
  const contextSummary = useSkillContextSummary(provider, baseline);
  const context = useSkillContext(skillId, baseline);
  const healthSummary = useSkillHealthSummary();
  const health = useSkillHealth(skillId);
  const selected = context.data;

  return (
    <section className="grid gap-3 lg:grid-cols-2">
      <Card>
        <CardHeader className="flex-row items-center justify-between">
          <CardTitle>{t("skillsContextBudget")}</CardTitle>
          <select
            aria-label={t("skillsContextBaseline")}
            value={baseline ?? ""}
            onChange={(event) =>
              setBaseline(
                event.target.value ? Number(event.target.value) : undefined,
              )
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
        </CardHeader>
        <CardContent className="space-y-3">
          <p className="text-muted-foreground text-xs">
            {t("skillsLocalEstimateAlgorithm", {
              algorithm: contextSummary.data?.algorithm_version ?? "unicode-mixed-v1",
            })}
          </p>
          {selected ? (
            <div className="grid grid-cols-3 gap-2 text-sm">
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
            <p className="text-muted-foreground text-sm">
              {t("skillsSelectForBudget")}
            </p>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="flex-row items-center justify-between">
          <CardTitle>{t("skillsHealth")}</CardTitle>
          <div className="flex gap-1 text-xs">
            <Badge variant="destructive">
              {t("skillsErrors", { count: healthSummary.data?.errors ?? 0 })}
            </Badge>
            <Badge variant="outline">
              {t("skillsWarnings", { count: healthSummary.data?.warnings ?? 0 })}
            </Badge>
          </div>
        </CardHeader>
        <CardContent className="space-y-2">
          {healthSummary.data?.completeness_status !== "complete" ? (
            <p className="rounded border border-amber-500/50 bg-amber-500/10 p-2 text-xs">
              {t("skillsIncompleteHealthHint")}
            </p>
          ) : null}
          {health.data?.checks
            .filter((item) => item.severity !== "pass")
            .map((item) => (
              <details
                key={item.check_id}
                className="rounded-md border p-2 text-sm"
              >
                <summary className="cursor-pointer font-medium">
                  <Badge
                    variant={
                      item.severity === "error" ? "destructive" : "outline"
                    }
                    className="mr-2"
                  >
                    {item.severity}
                  </Badge>
                  {item.title}
                </summary>
                <p className="mt-2 text-xs">{t("skillsEvidence", { evidence: item.evidence })}</p>
                <p className="text-muted-foreground mt-1 text-xs">
                  {t("skillsRecommendation", { recommendation: item.recommendation })}
                </p>
              </details>
            ))}
          {skillId &&
          health.data &&
          health.data.checks.every((item) => item.severity === "pass") ? (
            <p className="text-sm">{t("skillsHealthPassed")}</p>
          ) : null}
          {!skillId ? (
            <p className="text-muted-foreground text-sm">
              {t("skillsSelectForHealth")}
            </p>
          ) : null}
        </CardContent>
      </Card>
    </section>
  );
}
