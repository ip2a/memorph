import { useState } from "react";
import { ScrollPane } from "@/components/shared/scroll-pane";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { SkillGraphPanel } from "@/features/skills/skill-graph-panel";
import {
  SkillPruneDayTabs,
  SkillPrunePanel,
} from "@/features/skills/skill-prune-panel";
import {
  SkillStatsCustomDateRange,
  SkillStatsFilterTabs,
} from "@/features/skills/skill-stats-filters";
import { SkillStatsPanel } from "@/features/skills/skill-stats-panel";
import { useI18n } from "@/lib/i18n-context";
import type { I18nKey } from "@/lib/i18n-core";

const OVERVIEW_TABS = [
  { value: "summary", labelKey: "skillsTabSummary" },
  { value: "ranking", labelKey: "skillsTabRanking" },
  { value: "activity", labelKey: "skillsTabActivity" },
  { value: "prune", labelKey: "skillsSafePrune" },
] as const satisfies ReadonlyArray<{
  value: "summary" | "ranking" | "activity" | "prune";
  labelKey: I18nKey;
}>;

type OverviewTab = (typeof OVERVIEW_TABS)[number]["value"];

export function SkillOverviewPanel({
  skillId,
  provider,
}: {
  skillId: string | null;
  provider?: string;
}) {
  const { t } = useI18n();
  const [tab, setTab] = useState<OverviewTab>("summary");
  const [pruneDays, setPruneDays] = useState(30);
  const showStatsFilters = tab === "summary" || tab === "ranking";
  const showPruneFilters = tab === "prune";

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3">
      <Tabs
        value={tab}
        onValueChange={(value) => setTab(value as OverviewTab)}
        className="flex min-h-0 flex-1 flex-col gap-3"
      >
        <div className="flex shrink-0 flex-col gap-2">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <TabsList className="max-w-full overflow-x-auto">
              {OVERVIEW_TABS.map(({ value, labelKey }) => (
                <TabsTrigger key={value} value={value}>
                  {t(labelKey)}
                </TabsTrigger>
              ))}
            </TabsList>
            {showStatsFilters ? (
              <SkillStatsFilterTabs className="ml-auto shrink-0" />
            ) : null}
            {showPruneFilters ? (
              <SkillPruneDayTabs
                className="ml-auto shrink-0"
                days={pruneDays}
                onDaysChange={setPruneDays}
              />
            ) : null}
          </div>
          {showStatsFilters ? <SkillStatsCustomDateRange /> : null}
        </div>
        <ScrollPane className="min-h-0 flex-1">
          <TabsContent value="summary" className="mt-0">
            <SkillStatsPanel section="summary" provider={provider} />
          </TabsContent>
          <TabsContent value="ranking" className="mt-0">
            <SkillStatsPanel section="ranking" provider={provider} />
          </TabsContent>
          <TabsContent value="activity" className="mt-0">
            <SkillGraphPanel
              embedded
              skillId={skillId}
              provider={provider}
            />
          </TabsContent>
          <TabsContent value="prune" className="mt-0">
            <SkillPrunePanel
              embedded
              days={pruneDays}
              onDaysChange={setPruneDays}
            />
          </TabsContent>
        </ScrollPane>
      </Tabs>
    </div>
  );
}
