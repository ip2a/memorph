import { useEffect, useMemo, useRef, useState } from "react";
import { CornerDownRightIcon, FolderIcon, Trash2Icon, UnlinkIcon } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
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
import { ScrollPane } from "@/components/shared/scroll-pane";
import { Spinner } from "@/components/ui/spinner";
import { useSkillGroupInstallations } from "@/features/skills/queries";
import { skillUsedByLabel } from "@/features/skills/skill-used-by-label";
import { useI18n } from "@/lib/i18n-context";
import type { I18nKey } from "@/lib/i18n-core";
import { cn } from "@/lib/utils";
import type { SkillInstallation } from "@/lib/types";

function installKindLabelFromDeployment(
  deployment_mode: SkillInstallation["deployment_mode"],
  t: (key: I18nKey) => string,
) {
  if (deployment_mode === "symlink") return t("skillsInstallKindSymlink");
  if (deployment_mode === "copy") return t("skillsInstallKindManagedCopy");
  return t("skillsInstallKindDirectory");
}

function isRealInstallation(installation: SkillInstallation) {
  return installation.deployment_mode !== "symlink";
}

function symlinksPointingTo(
  installations: SkillInstallation[],
  realPath: string,
) {
  return installations.filter(
    (installation) =>
      installation.deployment_mode === "symlink" &&
      installation.symlink_target === realPath,
  );
}

function DeleteButton({
  pending,
  title,
  onClick,
}: {
  pending: boolean;
  title: string;
  onClick: () => void;
}) {
  return (
    <Button
      type="button"
      variant="ghost"
      size="icon-sm"
      className="text-muted-foreground hover:text-destructive size-8 shrink-0 opacity-60 group-hover:opacity-100"
      disabled={pending}
      title={title}
      onClick={(event) => {
        event.stopPropagation();
        onClick();
      }}
    >
      <Trash2Icon className="size-4" />
    </Button>
  );
}

function InstallationMetaBadges({
  installation,
  drifted,
  t,
}: {
  installation: SkillInstallation;
  drifted: boolean;
  t: (key: I18nKey, vars?: Record<string, string | number>) => string;
}) {
  const broken =
    installation.deployment_mode === "symlink" &&
    installation.link_status === "broken";

  return (
    <>
      {installation.scope_kind === "project" ? (
        <Badge variant="outline">{t("skillsProjectScopeOption")}</Badge>
      ) : null}
      <Badge variant="outline">
        {installKindLabelFromDeployment(installation.deployment_mode, t)}
      </Badge>
      {broken ? (
        <Badge variant="destructive">{t("skillsLinkBroken")}</Badge>
      ) : null}
      {drifted ? (
        <Badge variant="destructive" title={t("skillsConsolidateConflictHint")}>
          ≠
        </Badge>
      ) : null}
    </>
  );
}

function RealDirHeader({
  installation,
  catalogRealPath,
  selecting,
  selected,
  drifted,
  pending,
  t,
  onSelect,
  onDelete,
}: {
  installation: SkillInstallation;
  catalogRealPath: string | null;
  selecting: boolean;
  selected: boolean;
  drifted: boolean;
  pending: boolean;
  t: (key: I18nKey, vars?: Record<string, string | number>) => string;
  onSelect: () => void;
  onDelete: () => void;
}) {
  const isCurrentCatalogEntry =
    catalogRealPath != null && installation.path === catalogRealPath;

  const body = (
    <>
      {selecting ? (
        <span
          className={cn(
            "mt-1 size-4 shrink-0 rounded-full border-2 transition-colors",
            selected
              ? "border-primary bg-primary"
              : "border-muted-foreground/35 bg-transparent",
          )}
          aria-hidden
        />
      ) : null}
      <div className="min-w-0 flex-1 space-y-2">
        <div className="flex flex-wrap items-center gap-2">
          <FolderIcon className="text-muted-foreground size-4 shrink-0" />
          <strong>{skillUsedByLabel(installation.used_by, t)}</strong>
          {isCurrentCatalogEntry ? (
            <Badge
              variant="outline"
              title={catalogRealPath ?? undefined}
              data-skills-consolidate-location={catalogRealPath}
            >
              {t("skillsConsolidateCurrentLocation")}
            </Badge>
          ) : null}
          <InstallationMetaBadges
            installation={installation}
            drifted={selecting && drifted}
            t={t}
          />
        </div>
        <div className="space-y-1 text-xs">
          <p className="break-all font-mono">{installation.path}</p>
        </div>
      </div>
      <DeleteButton pending={pending} title={t("remove")} onClick={onDelete} />
    </>
  );

  if (!selecting) {
    return (
      <div className="group flex w-full items-start gap-3 p-3">{body}</div>
    );
  }

  return (
    <button
      type="button"
      disabled={pending}
      onClick={onSelect}
      className={cn(
        "group flex w-full items-start gap-3 p-3 text-left transition-colors",
        selected ? "bg-primary/5 hover:bg-primary/8" : "hover:bg-muted/30",
      )}
    >
      {body}
    </button>
  );
}

function SymlinkRow({
  installation,
  drifted,
  pending,
  t,
  onDelete,
}: {
  installation: SkillInstallation;
  drifted: boolean;
  pending: boolean;
  t: (key: I18nKey, vars?: Record<string, string | number>) => string;
  onDelete: () => void;
}) {
  return (
    <div className="group flex items-start gap-2 py-1">
      <CornerDownRightIcon className="text-muted-foreground/70 mt-1 size-4 shrink-0" />
      <div className="min-w-0 flex-1 space-y-2">
        <div className="flex flex-wrap items-center gap-2">
          <strong>{skillUsedByLabel(installation.used_by, t)}</strong>
          <InstallationMetaBadges
            installation={installation}
            drifted={drifted}
            t={t}
          />
        </div>
        <div className="space-y-1 text-xs">
          <p className="break-all font-mono">{installation.path}</p>
        </div>
      </div>
      <DeleteButton pending={pending} title={t("remove")} onClick={onDelete} />
    </div>
  );
}

function RealDirGroup({
  real,
  symlinks,
  catalogRealPath,
  selecting,
  selected,
  canonicalFingerprint,
  pending,
  t,
  onSelect,
  onDelete,
}: {
  real: SkillInstallation;
  symlinks: SkillInstallation[];
  catalogRealPath: string | null;
  selecting: boolean;
  selected: boolean;
  canonicalFingerprint: string | null;
  pending: boolean;
  t: (key: I18nKey, vars?: Record<string, string | number>) => string;
  onSelect: () => void;
  onDelete: (installation: SkillInstallation) => void;
}) {
  const drifted =
    selecting &&
    canonicalFingerprint != null &&
    real.fingerprint !== canonicalFingerprint;

  return (
    <div
      className={cn(
        "overflow-hidden rounded-lg border transition-colors",
        selecting && selected ? "border-primary/45" : "border-border/60",
      )}
    >
      <RealDirHeader
        installation={real}
        catalogRealPath={catalogRealPath}
        selecting={selecting}
        selected={selected}
        drifted={drifted}
        pending={pending}
        t={t}
        onSelect={onSelect}
        onDelete={() => onDelete(real)}
      />
      {symlinks.length ? (
        <div className="border-t border-border/50 bg-muted/15 px-3 py-3">
          <div className="relative pl-5">
            <span
              aria-hidden
              className="bg-border/80 absolute top-2 bottom-2 left-1.5 w-px"
            />
            <div className="space-y-3">
              {symlinks.map((installation) => {
                const linkDrifted =
                  selecting &&
                  canonicalFingerprint != null &&
                  installation.fingerprint !== canonicalFingerprint;
                return (
                  <div key={installation.path} className="relative">
                    <span
                      aria-hidden
                      className="bg-border/80 absolute top-[1.15rem] -left-2.5 h-px w-3"
                    />
                    <SymlinkRow
                      installation={installation}
                      drifted={linkDrifted}
                      pending={pending}
                      t={t}
                      onDelete={() => onDelete(installation)}
                    />
                  </div>
                );
              })}
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}

function OrphanSymlinkGroup({
  symlinks,
  selecting,
  canonicalFingerprint,
  pending,
  t,
  onDelete,
}: {
  symlinks: SkillInstallation[];
  selecting: boolean;
  canonicalFingerprint: string | null;
  pending: boolean;
  t: (key: I18nKey, vars?: Record<string, string | number>) => string;
  onDelete: (installation: SkillInstallation) => void;
}) {
  if (!symlinks.length) return null;

  const byTarget = symlinks.reduce<Map<string, SkillInstallation[]>>(
    (groups, installation) => {
      const target = installation.symlink_target ?? "";
      const bucket = groups.get(target) ?? [];
      bucket.push(installation);
      groups.set(target, bucket);
      return groups;
    },
    new Map(),
  );

  return (
    <section className="space-y-2">
      <h3 className="text-muted-foreground text-xs font-medium tracking-wide uppercase">
        {t("skillsConsolidateSymlinks")}
      </h3>
      {Array.from(byTarget.entries()).map(([target, links]) => (
        <div
          key={target || "unknown"}
          className="rounded-lg border border-dashed border-border/70 px-3 py-2"
        >
          <p className="text-muted-foreground mb-2 break-all font-mono text-xs">
            {t("skillsConsolidatePointsTo")}{" "}
            <span className="text-foreground">{target || "—"}</span>
          </p>
          <div className="space-y-2">
            {links.map((installation) => {
              const drifted =
                selecting &&
                canonicalFingerprint != null &&
                installation.fingerprint !== canonicalFingerprint;
              return (
                <SymlinkRow
                  key={installation.path}
                  installation={installation}
                  drifted={drifted}
                  pending={pending}
                  t={t}
                  onDelete={() => onDelete(installation)}
                />
              );
            })}
          </div>
        </div>
      ))}
    </section>
  );
}

export function SkillConsolidatePanel({
  active,
  sourceId,
  skillName,
  catalogRealPath,
  pending,
  onConfirm,
  onDeleteInstallation,
  onRemoveSymlinks,
}: {
  active: boolean;
  sourceId: string | null;
  skillName: string;
  catalogRealPath: string | null;
  pending: boolean;
  onConfirm: (canonicalPath: string) => void;
  onDeleteInstallation: (installPath: string) => void;
  onRemoveSymlinks: () => void;
}) {
  const { t } = useI18n();
  const groupQuery = useSkillGroupInstallations(active ? sourceId : null);
  const installations = groupQuery.data?.installations ?? [];
  const [selecting, setSelecting] = useState(false);
  const [consolidateConfirmOpen, setConsolidateConfirmOpen] = useState(false);
  const [canonicalPath, setCanonicalPath] = useState<string | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<SkillInstallation | null>(
    null,
  );
  const [removeSymlinksOpen, setRemoveSymlinksOpen] = useState(false);
  const confirmStarted = useRef(false);

  const { realDirs, symlinks } = useMemo(() => {
    const real: SkillInstallation[] = [];
    const links: SkillInstallation[] = [];
    for (const installation of installations) {
      if (isRealInstallation(installation)) {
        real.push(installation);
      } else {
        links.push(installation);
      }
    }
    return { realDirs: real, symlinks: links };
  }, [installations]);

  const { grouped, orphanSymlinks } = useMemo(() => {
    const realPaths = new Set(realDirs.map((item) => item.path));
    const grouped = realDirs.map((real) => ({
      real,
      symlinks: symlinks.filter((item) => item.symlink_target === real.path),
    }));
    const orphanSymlinks = symlinks.filter(
      (item) => !item.symlink_target || !realPaths.has(item.symlink_target),
    );
    return { grouped, orphanSymlinks };
  }, [realDirs, symlinks]);

  const canonicalFingerprint = selecting
    ? installations.find((installation) => installation.path === canonicalPath)
        ?.fingerprint ?? null
    : null;

  useEffect(() => {
    setSelecting(false);
    setConsolidateConfirmOpen(false);
    setCanonicalPath(null);
    setDeleteTarget(null);
    setRemoveSymlinksOpen(false);
  }, [sourceId, catalogRealPath]);

  useEffect(() => {
    if (!active) {
      setSelecting(false);
    }
  }, [active]);

  useEffect(() => {
    if (confirmStarted.current && !pending) {
      confirmStarted.current = false;
      setSelecting(false);
    }
  }, [pending]);

  const relatedSymlinkCount = deleteTarget
    ? symlinksPointingTo(installations, deleteTarget.path).length
    : 0;

  function handleDelete(installation: SkillInstallation) {
    if (isRealInstallation(installation)) {
      setDeleteTarget(installation);
      return;
    }
    onDeleteInstallation(installation.path);
  }

  const canStartConsolidate = installations.length >= 2;
  const canConfirmConsolidate = selecting && Boolean(canonicalPath);

  function startSelecting() {
    const preferred =
      realDirs.find((item) => item.path === catalogRealPath)?.path ??
      realDirs[0]?.path ??
      installations[0]?.path ??
      null;
    setCanonicalPath(preferred);
    setSelecting(true);
  }

  function executeConsolidate() {
    if (!canonicalPath) return;
    confirmStarted.current = true;
    onConfirm(canonicalPath);
    setConsolidateConfirmOpen(false);
  }

  return (
    <>
      <div
        className="flex min-h-0 flex-1 flex-col gap-3"
        data-skills-consolidate-panel
      >
        <p className="text-muted-foreground shrink-0 text-xs">
          {t("skillsConsolidateDescription")}
        </p>

        <ScrollPane className="min-h-0 flex-1">
          <div className="flex flex-col gap-2.5 p-1">
            {groupQuery.isLoading ? (
              <div className="flex justify-center py-6">
                <Spinner />
              </div>
            ) : installations.length === 0 ? (
              <p className="text-muted-foreground text-sm">
                {t("skillsConsolidateEmpty")}
              </p>
            ) : (
              <>
                {grouped.length ? (
                  <section className="space-y-2">
                    <h3 className="text-muted-foreground text-xs font-medium tracking-wide uppercase">
                      {t("skillsConsolidateRealDirs")}
                    </h3>
                    <div className="flex flex-col gap-2">
                      {grouped.map(({ real, symlinks: links }) => (
                        <RealDirGroup
                          key={real.path}
                          real={real}
                          symlinks={links}
                          catalogRealPath={catalogRealPath}
                          selecting={selecting}
                          selected={selecting && real.path === canonicalPath}
                          canonicalFingerprint={canonicalFingerprint}
                          pending={pending}
                          t={t}
                          onSelect={() => setCanonicalPath(real.path)}
                          onDelete={handleDelete}
                        />
                      ))}
                    </div>
                  </section>
                ) : null}
                <OrphanSymlinkGroup
                  symlinks={orphanSymlinks}
                  selecting={selecting}
                  canonicalFingerprint={canonicalFingerprint}
                  pending={pending}
                  t={t}
                  onDelete={handleDelete}
                />
                {!selecting && !canStartConsolidate && installations.length > 0 ? (
                  <p className="text-muted-foreground text-xs">
                    {t("skillsConsolidateNeedTwo")}
                  </p>
                ) : null}
              </>
            )}
          </div>
        </ScrollPane>

        <div className="flex shrink-0 flex-col gap-2 border-t pt-3 sm:flex-row sm:items-center sm:justify-between">
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="w-full sm:mr-auto sm:w-auto"
            disabled={pending || symlinks.length === 0}
            title={t("skillsRemoveSymlinksHint")}
            onClick={() => setRemoveSymlinksOpen(true)}
          >
            <UnlinkIcon data-icon="inline-start" />
            {t("skillsRemoveSymlinks")}
          </Button>
          {selecting ? (
            <div className="flex w-full flex-col gap-2 sm:w-auto sm:flex-row sm:justify-end">
              <Button
                type="button"
                variant="outline"
                className="w-full sm:w-auto"
                disabled={pending}
                onClick={() => setSelecting(false)}
              >
                {t("cancel")}
              </Button>
              <Button
                type="button"
                className="w-full sm:w-auto"
                disabled={pending || !canConfirmConsolidate}
                onClick={() => setConsolidateConfirmOpen(true)}
              >
                {t("skillsConsolidateApplyCanonical")}
              </Button>
            </div>
          ) : (
            <Button
              type="button"
              className="w-full sm:w-auto"
              disabled={pending || !canStartConsolidate}
              onClick={startSelecting}
            >
              {t("skillsConsolidate")}
            </Button>
          )}
        </div>
      </div>

      <AlertDialog
        open={consolidateConfirmOpen}
        onOpenChange={setConsolidateConfirmOpen}
      >
        <AlertDialogContent className="sm:max-w-md">
          <AlertDialogHeader>
            <AlertDialogTitle>{t("skillsConsolidateConfirmTitle")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("skillsConsolidateConfirmDescription", {
                path: canonicalPath ?? "",
              })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={pending}>
              {t("cancel")}
            </AlertDialogCancel>
            <AlertDialogAction disabled={pending} onClick={executeConsolidate}>
              {pending ? <Spinner data-icon="inline-start" /> : null}
              {t("skillsConsolidateConfirm")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog
        open={Boolean(deleteTarget)}
        onOpenChange={(next) => !next && setDeleteTarget(null)}
      >
        <AlertDialogContent className="sm:max-w-md">
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t("skillsConsolidateDeleteRealTitle")}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {relatedSymlinkCount > 0
                ? t("skillsConsolidateDeleteRealDescription", {
                    path: deleteTarget?.path ?? "",
                    count: relatedSymlinkCount,
                  })
                : t("skillsConsolidateDeleteRealDescriptionNoSymlinks", {
                    path: deleteTarget?.path ?? "",
                  })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={pending}>
              {t("cancel")}
            </AlertDialogCancel>
            <AlertDialogAction
              disabled={pending}
              onClick={() => {
                if (deleteTarget) {
                  onDeleteInstallation(deleteTarget.path);
                  setDeleteTarget(null);
                }
              }}
            >
              {pending ? <Spinner data-icon="inline-start" /> : null}
              {t("delete")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog
        open={removeSymlinksOpen}
        onOpenChange={setRemoveSymlinksOpen}
      >
        <AlertDialogContent className="sm:max-w-md">
          <AlertDialogHeader>
            <AlertDialogTitle>{t("skillsRemoveSymlinksTitle")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("skillsRemoveSymlinksDescription", { skill: skillName })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={pending}>
              {t("cancel")}
            </AlertDialogCancel>
            <AlertDialogAction
              disabled={pending}
              onClick={() => {
                onRemoveSymlinks();
                setRemoveSymlinksOpen(false);
              }}
            >
              {pending ? <Spinner data-icon="inline-start" /> : null}
              {t("skillsRemoveSymlinks")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}
