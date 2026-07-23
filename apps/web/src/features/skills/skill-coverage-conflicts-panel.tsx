import { useMemo, useState, type ReactNode } from "react";
import { Link } from "react-router-dom";
import { PanelCard } from "@/components/shared/panel-card";
import { SectionHeading } from "@/components/shared/section-heading";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  useSkillConflicts,
  useSkillCoverage,
  useSkillCoverageEvidence,
} from "@/features/skills/queries";
import { useI18n } from "@/lib/i18n-context";
import type { I18nKey } from "@/lib/i18n-core";

const ranges = ["7d", "30d", "90d", "all"];
function PanelShell({ embedded, children }: { embedded: boolean; children: ReactNode }) {
  return embedded ? (
    <div className="space-y-3">{children}</div>
  ) : (
    <PanelCard className="space-y-3 p-4">{children}</PanelCard>
  );
}

const categories: readonly [string, I18nKey][] = [
  ["section", "skillsCategorySection"],
  ["script", "skillsCategoryScript"],
  ["reference", "skillsCategoryReference"],
  ["asset", "skillsCategoryAsset"],
  ["other-file", "skillsCategoryOther"],
];

export function SkillCoverageConflictsPanel({
  skillId,
  embedded = false,
}: {
  skillId: string | null;
  embedded?: boolean;
}) {
  const { t } = useI18n();
  const [range, setRange] = useState("90d");
  const [category, setCategory] = useState("section");
  const [targetKey, setTargetKey] = useState<string | null>(null);
  const [evidencePage, setEvidencePage] = useState(1);
  const coverage = useSkillCoverage(skillId, range);
  const conflicts = useSkillConflicts(skillId);
  const evidence = useSkillCoverageEvidence(skillId, targetKey, evidencePage);
  const targets = useMemo(
    () => coverage.data?.targets.filter((target) => target.target_kind === category) ?? [],
    [category, coverage.data?.targets],
  );
  if (!skillId) {
    return (
      <p className="text-muted-foreground text-sm">
        {t("skillsSelectForCoverage")}
      </p>
    );
  }

  return (
    <div className="grid gap-3 lg:grid-cols-2">
      <PanelShell embedded={embedded}>
        <div className="flex flex-wrap items-center justify-between gap-2">
          <SectionHeading title={t("skillsCoverage")} className="border-0 pb-0" />
          <div className="flex gap-1">
            {ranges.map((item) => (
              <Button
                key={item}
                size="sm"
                variant={range === item ? "default" : "outline"}
                onClick={() => setRange(item)}
              >
                {item}
              </Button>
            ))}
          </div>
        </div>
        {coverage.isLoading ? (
          <p className="text-sm text-muted-foreground">{t("skillsLoading")}</p>
        ) : coverage.isError ? (
          <p className="text-sm text-destructive">{coverage.error.message}</p>
        ) : (
          <>
            <div>
              <p className="text-2xl font-semibold">
                {coverage.data?.percent.toFixed(1)}%{" "}
                <span className="text-sm font-normal text-muted-foreground">
                  {coverage.data?.covered}/{coverage.data?.total}
                </span>
              </p>
              <Progress value={coverage.data?.percent ?? 0} className="mt-2" />
              {coverage.data?.completeness_status !== "complete" ? (
                <p className="mt-2 text-xs text-amber-700 dark:text-amber-300">
                  {t("skillsIncompleteCoverageHint")}
                </p>
              ) : null}
            </div>
            <Tabs value={category} onValueChange={setCategory}>
              <TabsList className="max-w-full overflow-x-auto">
                {categories.map(([value, label]) => (
                  <TabsTrigger key={value} value={value}>
                    {t(label)}
                  </TabsTrigger>
                ))}
              </TabsList>
            </Tabs>
            <div className="max-h-56 space-y-2 overflow-auto">
              {targets.map((target) => (
                <button
                  type="button"
                  key={`${target.target_kind}:${target.target_key}`}
                  className="flex w-full items-start justify-between gap-2 rounded-md border p-2 text-left text-sm"
                  onClick={() => {
                    setTargetKey(target.target_key);
                    setEvidencePage(1);
                  }}
                >
                  <span>{target.section_title ?? target.target_path ?? target.target_key}</span>
                  <Badge variant={target.confidence ? "outline" : "secondary"}>
                    {target.observations} · {target.confidence ?? t("skillsUncovered")}
                  </Badge>
                </button>
              ))}
              {targets.length === 0 ? (
                <p className="text-sm text-muted-foreground">{t("skillsEmptyCategory")}</p>
              ) : null}
            </div>
          </>
        )}
      </PanelShell>
      <PanelShell embedded={embedded}>
        <SectionHeading title={t("skillsConflicts")} className="border-0 pb-0" />
        {conflicts.isLoading ? (
          <p className="text-sm text-muted-foreground">{t("skillsLoading")}</p>
        ) : conflicts.isError ? (
          <p className="text-sm text-destructive">{conflicts.error.message}</p>
        ) : conflicts.data?.length ? (
          <div className="max-h-64 space-y-2 overflow-auto">
            {conflicts.data.map((item) => (
              <div key={item.id} className="rounded-md border p-3 text-sm">
                <div className="flex items-center gap-2">
                  <Badge variant={item.severity === "error" ? "destructive" : "outline"}>
                    {item.conflict_kind}
                  </Badge>
                  <span>{Math.round(item.similarity * 100)}%</span>
                </div>
                <p className="mt-2 font-medium">{item.left_name} ↔ {item.right_name}</p>
                <p className="text-muted-foreground">{item.evidence}</p>
                <p className="text-muted-foreground">{t("skillsRecommendation", { recommendation: item.recommendation })}</p>
              </div>
            ))}
          </div>
        ) : (
          <p className="text-sm text-muted-foreground">{t("skillsNoConflicts")}</p>
        )}
      </PanelShell>
      <Sheet open={Boolean(targetKey)} onOpenChange={(open) => !open && setTargetKey(null)}>
        <SheetContent className="overflow-y-auto">
          <SheetHeader>
            <SheetTitle>{t("skillsCoverageEvidence")}</SheetTitle>
            <SheetDescription>{t("skillsCoverageEvidenceDescription")}</SheetDescription>
          </SheetHeader>
          <div className="space-y-3 px-4 pb-4">
            {(evidence.data?.items ?? []).map((item) => (
              <div key={item.invocation_id} className="rounded-md border p-3 text-sm">
                <div className="flex gap-2">
                  <Badge variant="outline">{item.match_kind}</Badge>
                  <Badge variant={item.confidence === "low" ? "secondary" : "default"}>
                    {item.confidence}
                  </Badge>
                </div>
                {item.evidence_text ? (
                  <p className="mt-2 text-muted-foreground">{item.evidence_text}</p>
                ) : null}
                <Link
                  className="mt-2 inline-block text-primary underline"
                  to={`/sessions/${encodeURIComponent(item.provider_id)}/${encodeURIComponent(item.session_id)}`}
                >
                  {t("skillsOpenSession", { session: item.session_id })}
                </Link>
              </div>
            ))}
            {evidence.data?.items.length === 0 ? (
              <p className="text-sm text-muted-foreground">{t("skillsNoEvidence")}</p>
            ) : null}
            {(evidence.data?.total ?? 0) > (evidence.data?.page_size ?? 20) ? (
              <div className="flex justify-between">
                <Button
                  size="sm"
                  variant="outline"
                  disabled={evidencePage === 1}
                  onClick={() => setEvidencePage((page) => page - 1)}
                >
                  {t("skillsPreviousPage")}
                </Button>
                <Button
                  size="sm"
                  variant="outline"
                  disabled={evidencePage * (evidence.data?.page_size ?? 20) >= (evidence.data?.total ?? 0)}
                  onClick={() => setEvidencePage((page) => page + 1)}
                >
                  {t("skillsNextPage")}
                </Button>
              </div>
            ) : null}
          </div>
        </SheetContent>
      </Sheet>
    </div>
  );
}
