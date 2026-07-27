import { Badge } from "@/components/ui/badge";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { useSkillHealth, useSkillHealthSummary } from "@/features/skills/queries";
import { useI18n } from "@/lib/i18n-context";
import type { SkillHealthCheck } from "@/lib/types";
import { cn } from "@/lib/utils";

function healthBadgeVariant(severity: SkillHealthCheck["severity"]) {
  if (severity === "error") return "destructive" as const;
  if (severity === "warning") return "outline" as const;
  return "secondary" as const;
}

function HealthCheckTag({ check }: { check: SkillHealthCheck }) {
  const { t } = useI18n();

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Badge
          variant={healthBadgeVariant(check.severity)}
          className={cn(
            "cursor-default rounded-full",
            check.severity === "warning" &&
              "border-amber-500/50 bg-amber-500/10 text-amber-950 dark:text-amber-100",
          )}
        >
          {check.title}
        </Badge>
      </TooltipTrigger>
      <TooltipContent
        side="bottom"
        className="max-w-sm space-y-1 text-left whitespace-normal"
      >
        <p className="font-medium">{check.title}</p>
        {check.description ? <p>{check.description}</p> : null}
        {check.evidence ? (
          <p>{t("skillsEvidence", { evidence: check.evidence })}</p>
        ) : null}
        {check.recommendation ? (
          <p>{t("skillsRecommendation", { recommendation: check.recommendation })}</p>
        ) : null}
      </TooltipContent>
    </Tooltip>
  );
}

export function SkillHealthDetails({ skillId }: { skillId: string | null }) {
  const { t } = useI18n();
  const healthSummary = useSkillHealthSummary();
  const health = useSkillHealth(skillId);

  if (!skillId) {
    return (
      <p className="text-muted-foreground text-sm">{t("skillsSelectForHealth")}</p>
    );
  }

  if (health.isLoading) return null;

  const checks = health.data?.checks.filter((item) => item.severity !== "pass") ?? [];
  const incomplete = healthSummary.data?.completeness_status !== "complete";

  return (
    <div className="space-y-2">
      {incomplete ? (
        <p className="rounded border border-amber-500/50 bg-amber-500/10 p-2 text-xs">
          {t("skillsIncompleteHealthHint")}
        </p>
      ) : null}
      {checks.map((check) => (
        <div key={check.check_id} className="rounded-md border p-2 text-sm">
          <div className="flex flex-wrap items-center gap-2">
            <Badge variant={healthBadgeVariant(check.severity)}>{check.severity}</Badge>
            <span className="font-medium">{check.title}</span>
          </div>
          {check.description ? (
            <p className="text-muted-foreground mt-2 text-xs">{check.description}</p>
          ) : null}
          {check.evidence ? (
            <p className="mt-1 text-xs">{t("skillsEvidence", { evidence: check.evidence })}</p>
          ) : null}
          {check.recommendation ? (
            <p className="text-muted-foreground mt-1 text-xs">
              {t("skillsRecommendation", { recommendation: check.recommendation })}
            </p>
          ) : null}
        </div>
      ))}
      {health.data && checks.length === 0 && !incomplete ? (
        <p className="text-sm">{t("skillsHealthPassed")}</p>
      ) : null}
    </div>
  );
}

export function SkillHealthTags({ skillId }: { skillId: string | null }) {
  const { t } = useI18n();
  const healthSummary = useSkillHealthSummary();
  const health = useSkillHealth(skillId);

  if (!skillId || health.isLoading) return null;

  const checks = health.data?.checks.filter((item) => item.severity !== "pass") ?? [];
  const incomplete = healthSummary.data?.completeness_status !== "complete";

  if (!checks.length && !incomplete && !health.data) return null;

  return (
    <>
      {incomplete ? (
        <Tooltip>
          <TooltipTrigger asChild>
            <Badge
              variant="outline"
              className="cursor-default rounded-full border-amber-500/50 bg-amber-500/10 text-amber-950 dark:text-amber-100"
            >
              {t("skillsIncompleteHealthShort")}
            </Badge>
          </TooltipTrigger>
          <TooltipContent
            side="bottom"
            className="max-w-sm text-left whitespace-normal"
          >
            {t("skillsIncompleteHealthHint")}
          </TooltipContent>
        </Tooltip>
      ) : null}
      {checks.map((check) => (
        <HealthCheckTag key={check.check_id} check={check} />
      ))}
      {health.data && checks.length === 0 && !incomplete ? (
        <Badge variant="secondary" className="rounded-full">
          {t("skillsHealthPassed")}
        </Badge>
      ) : null}
    </>
  );
}
