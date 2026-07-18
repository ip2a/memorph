import { useMemo, useState } from "react";
import {
  PackageIcon,
  RefreshCwIcon,
  SearchIcon,
  Trash2Icon,
} from "lucide-react";
import { PageError, PageSkeleton } from "@/components/shared/page-states";
import { PanelCard } from "@/components/shared/panel-card";
import { ScrollPane } from "@/components/shared/scroll-pane";
import { SelectableRowButton } from "@/components/shared/selectable-row-button";
import { TwoPanePage } from "@/components/shared/two-pane-page";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyTitle,
} from "@/components/ui/empty";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import {
  useInstallSkill,
  useSkills,
  useUninstallSkill,
} from "@/features/skills/queries";
import { useI18n } from "@/lib/i18n-context";
import type { SkillAgent, SkillEntry } from "@/lib/types";

export function SkillsPage() {
  const { t } = useI18n();
  const skillsQuery = useSkills();
  const installMutation = useInstallSkill();
  const uninstallMutation = useUninstallSkill();
  const [search, setSearch] = useState("");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [removal, setRemoval] = useState<{
    skill: SkillEntry;
    agent: SkillAgent;
  } | null>(null);

  const skills = useMemo(() => {
    const query = search.trim().toLocaleLowerCase();
    if (!query) return skillsQuery.data?.skills || [];
    return (skillsQuery.data?.skills || []).filter((skill) =>
      `${skill.name} ${skill.description || ""}`
        .toLocaleLowerCase()
        .includes(query),
    );
  }, [search, skillsQuery.data?.skills]);

  if (skillsQuery.isLoading) return <PageSkeleton />;
  if (skillsQuery.isError) {
    return (
      <PageError
        title={t("skillsLoadFailed")}
        message={
          skillsQuery.error instanceof Error
            ? skillsQuery.error.message
            : t("skillsLoadFailed")
        }
        onRetry={() => skillsQuery.refetch()}
      />
    );
  }

  const selected =
    skills.find((skill) => skill.id === selectedId) || skills[0] || null;
  const pending = installMutation.isPending || uninstallMutation.isPending;
  const mutationError = installMutation.error || uninstallMutation.error;

  return (
    <div className="flex h-full min-h-0 flex-col gap-4">
      <header className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">
            {t("skills")}
          </h1>
          <p className="text-muted-foreground mt-1 text-sm">
            {t("skillsDescription")}
          </p>
        </div>
        <Button
          type="button"
          variant="outline"
          onClick={() => skillsQuery.refetch()}
          disabled={skillsQuery.isFetching}
        >
          {skillsQuery.isFetching ? (
            <Spinner />
          ) : (
            <RefreshCwIcon data-icon="inline-start" />
          )}
          {t("refresh")}
        </Button>
      </header>

      {mutationError ? (
        <PageError
          title={t("skillsActionFailed")}
          message={
            mutationError instanceof Error
              ? mutationError.message
              : t("skillsActionFailed")
          }
        />
      ) : null}

      <TwoPanePage className="flex-1">
        <PanelCard className="flex min-h-0 flex-col gap-3 p-3">
          <label className="relative block">
            <SearchIcon className="text-muted-foreground pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2" />
            <Input
              aria-label={t("searchSkills")}
              value={search}
              onChange={(event) => setSearch(event.target.value)}
              placeholder={t("searchSkills")}
              className="pl-9"
            />
          </label>
          <div className="text-muted-foreground flex items-center justify-between text-xs">
            <span>{t("skillsFound", { count: skills.length })}</span>
            <span>
              {t("skillsAgents", {
                count: skillsQuery.data?.agents.length || 0,
              })}
            </span>
          </div>
          <ScrollPane className="flex-1" innerClassName="flex flex-col gap-2">
            {skills.map((skill) => (
              <SelectableRowButton
                key={skill.id}
                selected={skill.id === selectedId}
                onClick={() => setSelectedId(skill.id)}
                leading={<PackageIcon className="size-4" />}
                title={skill.name}
                meta={skill.description || skill.directory}
                trailing={
                  <Badge variant="outline">{skill.installations.length}</Badge>
                }
              />
            ))}
            {!skills.length ? (
              <Empty className="border border-dashed">
                <EmptyHeader>
                  <EmptyTitle>
                    {search ? t("skillsNoMatches") : t("skillsEmpty")}
                  </EmptyTitle>
                  <EmptyDescription>
                    {search
                      ? t("skillsNoMatchesDescription")
                      : t("skillsEmptyDescription")}
                  </EmptyDescription>
                </EmptyHeader>
              </Empty>
            ) : null}
          </ScrollPane>
        </PanelCard>

        <PanelCard className="min-h-0 p-4">
          {selected ? (
            <ScrollPane innerClassName="flex flex-col gap-5">
              <div>
                <div className="flex flex-wrap items-center gap-2">
                  <h2 className="text-xl font-semibold">{selected.name}</h2>
                  <Badge variant="secondary">{selected.directory}</Badge>
                </div>
                <p className="text-muted-foreground mt-2 text-sm">
                  {selected.description || t("skillsNoDescription")}
                </p>
              </div>

              <section className="flex flex-col gap-3">
                <h3 className="text-sm font-semibold">
                  {t("skillsInstallations")}
                </h3>
                {skillsQuery.data?.agents.map((agent) => {
                  const installation = selected.installations.find(
                    (item) => item.provider_id === agent.provider_id,
                  );
                  const acting =
                    pending &&
                    ((installMutation.variables?.skill_id === selected.id &&
                      installMutation.variables.provider ===
                        agent.provider_id) ||
                      (uninstallMutation.variables?.skill_id === selected.id &&
                        uninstallMutation.variables.provider ===
                          agent.provider_id));
                  return (
                    <div
                      key={agent.provider_id}
                      className="flex flex-wrap items-center justify-between gap-3 rounded-lg border p-3"
                    >
                      <div className="min-w-0">
                        <div className="flex items-center gap-2">
                          <strong className="text-sm">{agent.name}</strong>
                          <Badge
                            variant={installation ? "secondary" : "outline"}
                          >
                            {installation ? t("installed") : t("notInstalled")}
                          </Badge>
                          {installation?.managed ? (
                            <Badge variant="outline">
                              {t("skillsManaged")}
                            </Badge>
                          ) : null}
                        </div>
                        <p className="text-muted-foreground mt-1 truncate font-mono text-xs">
                          {installation?.path || agent.skills_dir}
                        </p>
                      </div>
                      {installation ? (
                        <Button
                          type="button"
                          variant="destructive"
                          size="sm"
                          disabled={pending || !installation.managed}
                          title={
                            !installation.managed
                              ? t("skillsUserOwnedHint")
                              : undefined
                          }
                          onClick={() => setRemoval({ skill: selected, agent })}
                        >
                          {acting ? (
                            <Spinner />
                          ) : (
                            <Trash2Icon data-icon="inline-start" />
                          )}
                          {t("remove")}
                        </Button>
                      ) : (
                        <Button
                          type="button"
                          size="sm"
                          disabled={pending}
                          onClick={() =>
                            installMutation.mutate({
                              skill_id: selected.id,
                              provider: agent.provider_id,
                            })
                          }
                        >
                          {acting ? <Spinner /> : null}
                          {t("install")}
                        </Button>
                      )}
                    </div>
                  );
                })}
              </section>
            </ScrollPane>
          ) : (
            <Empty>
              <EmptyHeader>
                <EmptyTitle>{t("skillsSelect")}</EmptyTitle>
                <EmptyDescription>
                  {t("skillsSelectDescription")}
                </EmptyDescription>
              </EmptyHeader>
            </Empty>
          )}
        </PanelCard>
      </TwoPanePage>

      <AlertDialog
        open={!!removal}
        onOpenChange={(open) => !open && setRemoval(null)}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("skillsRemoveTitle")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("skillsRemoveDescription", {
                skill: removal?.skill.name,
                agent: removal?.agent.name,
              })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("cancel")}</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                if (!removal) return;
                uninstallMutation.mutate({
                  skill_id: removal.skill.id,
                  provider: removal.agent.provider_id,
                });
                setRemoval(null);
              }}
            >
              {t("remove")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
