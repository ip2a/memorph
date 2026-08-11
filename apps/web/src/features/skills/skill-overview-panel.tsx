import { useState } from "react";
import { ScrollPane } from "@/components/shared/scroll-pane";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Separator } from "@/components/ui/separator";
import { SkillGraphPanel } from "@/features/skills/skill-graph-panel";
import { SkillStatsFilterTabs } from "@/features/skills/skill-stats-filters";
import {
  SkillStatsAnalysisProgress,
  SkillStatsPanel,
  SkillStatsAnalyzeButton,
} from "@/features/skills/skill-stats-panel";
import { useI18n } from "@/lib/i18n-context";
import type { I18nKey } from "@/lib/i18n-core";

const OVERVIEW_TABS = [
  { value: "summary", labelKey: "skillsTabSummary" },
  { value: "activity", labelKey: "skillsTabActivity" },
] as const satisfies ReadonlyArray<{
  value: "summary" | "activity";
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
  const showStatsFilters = tab === "summary" || tab === "activity";

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3">
      <Tabs
        value={tab}
        onValueChange={(value) => setTab(value as OverviewTab)}
        className="flex min-h-0 flex-1 flex-col gap-3"
      >
        <div className="flex shrink-0 flex-col gap-2">
          <div className="flex flex-wrap items-center gap-2">
            <TabsList className="shrink-0 max-w-full overflow-x-auto">
              {OVERVIEW_TABS.map(({ value, labelKey }) => (
                <TabsTrigger key={value} value={value}>
                  {t(labelKey)}
                </TabsTrigger>
              ))}
            </TabsList>
            {showStatsFilters ? (
              <div className="ml-auto flex min-w-0 items-center gap-2">
                <SkillStatsAnalyzeButton />
                <SkillStatsAnalysisProgress className="min-w-0 max-w-xs flex-1" />
                <SkillStatsFilterTabs className="shrink-0" />
              </div>
            ) : null}
          </div>
        </div>
        <ScrollPane className="min-h-0 flex-1">
          <TabsContent value="summary" className="mt-0">
            <SkillStatsPanel section="summary" provider={provider} />
          </TabsContent>
          <TabsContent value="activity" className="mt-0 min-w-0">
            <div className="grid min-w-0 gap-4">
              <SkillGraphPanel
                embedded
                skillId={skillId}
                provider={provider}
              />
              <Separator />
              <SkillStatsPanel section="ranking" provider={provider} />
            </div>
          </TabsContent>
        </ScrollPane>
      </Tabs>
    </div>
  );
}
