import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { RefreshCwIcon, WrenchIcon } from "lucide-react";
import { useMemo, useState } from "react";
import { toast } from "sonner";
import { PathText } from "@/components/shared/path-text";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Field, FieldContent, FieldDescription, FieldGroup, FieldTitle } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { InputGroup, InputGroupAddon, InputGroupInput } from "@/components/ui/input-group";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Separator } from "@/components/ui/separator";
import { Spinner } from "@/components/ui/spinner";
import { Textarea } from "@/components/ui/textarea";
import {
  checkForUpdate,
  getHooksOverview,
  getMeta,
  getProviderCatalog,
  openExternal,
  runHookProviderOperation,
  updateProviderCatalog,
  updateSettings,
} from "@/lib/api";
import { formatDateTime } from "@/lib/format";
import { useI18n } from "@/lib/i18n-context";
import type { I18nKey } from "@/lib/i18n-core";
import { queryKeys } from "@/lib/query-keys";
import type { AgentManagementEntry, HookOverviewPayload, ProviderCatalogEntry, SettingsPayload, UiLanguage, UpdateCheckPayload, UpdateSettingsPayload } from "@/lib/types";
import { AgentOrderList } from "@/features/settings/agent-order-list";

const SECTIONS = [
  { id: "general", labelKey: "general" },
  { id: "display", labelKey: "display" },
  { id: "order", labelKey: "order" },
  { id: "hook", labelKey: "hooks" },
  { id: "config", labelKey: "configFile" },
  { id: "about", labelKey: "about" },
] as const;

const HOME_BUTTONS = [
  ["view", "showView"],
  ["compress", "showCompress"],
  ["switch", "showSwitch"],
  ["export", "showExport"],
  ["sync", "showSync"],
  ["delete", "showDelete"],
] as const;

const ABOUT_LINKS = [
  { label: "GitHub", url: "https://github.com/ip2a/memorph", iconUrl: "https://github.com/favicon.ico" },
  { label: "npm", url: "https://www.npmjs.com/package/memorph", iconUrl: "https://www.npmjs.com/favicon.ico" },
  { label: "crates.io", url: "https://crates.io/crates/memorph", iconUrl: "https://crates.io/favicon.ico" },
  { label: "PyPI", url: "https://pypi.org/project/memorph/", iconUrl: "https://pypi.org/favicon.ico" },
] as const;

type SectionId = (typeof SECTIONS)[number]["id"];

type SettingsDraft = UpdateSettingsPayload & {
  hidden_agents: string[];
};

function defaultDraft(settings: SettingsPayload | undefined, catalog: ProviderCatalogEntry[] = []): SettingsDraft {
  const agentOrder = settings?.agent_order?.length ? settings.agent_order : catalog.map((provider) => provider.provider_id);
  return {
    sessions_per_provider: settings?.sessions_per_provider ?? 12,
    language: settings?.language ?? "auto",
    show_opencode_subagents: settings?.show_opencode_subagents ?? false,
    sort_providers_by_session_count: settings?.sort_providers_by_session_count ?? false,
    default_backup_dir: settings?.default_backup_dir || "./backups",
    logging: {
      max_size_bytes: Number(settings?.logging?.max_size_bytes ?? 5 * 1024 * 1024),
      retention_days: settings?.logging?.retention_days == null ? null : Number(settings.logging.retention_days),
    },
    home_buttons: {
      view: settings?.home_buttons?.view !== false,
      compress: settings?.home_buttons?.compress !== false,
      switch: settings?.home_buttons?.switch !== false,
      export: settings?.home_buttons?.export !== false,
      sync: settings?.home_buttons?.sync !== false,
      delete: settings?.home_buttons?.delete !== false,
    },
    agent_order: agentOrder,
    primary_agents: settings?.primary_agents ?? [],
    hidden_agents: catalog.filter((provider) => provider.hidden_state?.global).map((provider) => provider.provider_id),
  };
}

function logSizeMb(draft: SettingsDraft) {
  const bytes = Number(draft.logging.max_size_bytes ?? 0);
  return String((bytes > 0 ? bytes : 5 * 1024 * 1024) / 1024 / 1024).replace(/\.0$/, "");
}

async function openUrl(url: string) {
  try {
    await openExternal({ url });
    return;
  } catch {
    const opened = window.open(url, "_blank", "noopener,noreferrer");
    if (!opened) window.location.href = url;
  }
}

function hookStatus(provider: AgentManagementEntry) {
  return provider.hook?.status || "unknown";
}

function hookVersion(provider: AgentManagementEntry) {
  const hook = provider.hook || {};
  if (hook.installed_version && hook.current_version && hook.installed_version !== hook.current_version) {
    return `${hook.installed_version} -> ${hook.current_version}`;
  }
  return hook.installed_version || hook.current_version || "-";
}

function hookOperationIds(provider: AgentManagementEntry) {
  const capabilities = provider.hook_capabilities || {};
  if (!provider.hook_profile) return [];
  const available = {
    install_hook: capabilities.install !== false,
    verify_hook: capabilities.verify !== false,
    repair_hook: capabilities.repair !== false,
    uninstall_hook: capabilities.uninstall !== false,
  };
  const keepAvailable = (ids: string[]) => ids.filter((id) => available[id as keyof typeof available]);
  const status = hookStatus(provider);
  if (status === "not_installed") return keepAvailable(["install_hook", "verify_hook"]);
  if (["installed_disabled", "installed_stale_binary", "installed_stale_endpoint", "installed_broken_config", "installed_conflict", "repairable", "needs_user_action"].includes(status)) {
    return keepAvailable(["repair_hook", "verify_hook", "uninstall_hook"]);
  }
  if (status === "installed_ok") return keepAvailable(["verify_hook", "repair_hook", "uninstall_hook"]);
  return keepAvailable(["verify_hook"]);
}

function hookOperationLabel(operation: string) {
  if (operation === "install_hook") return "Install";
  if (operation === "verify_hook") return "Verify";
  if (operation === "repair_hook") return "Repair";
  if (operation === "uninstall_hook") return "Uninstall";
  return operation;
}

export function SettingsDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (open: boolean) => void }) {
  const queryClient = useQueryClient();
  const { t, setLanguageOverride } = useI18n();
  const [section, setSection] = useState<SectionId>("general");
  const [draftOverride, setDraftOverride] = useState<SettingsDraft | null>(null);
  const [updateResult, setUpdateResult] = useState<UpdateCheckPayload | null>(null);
  const [updateError, setUpdateError] = useState<string>("");

  const meta = useQuery({ queryKey: queryKeys.meta, queryFn: getMeta, enabled: open });
  const catalog = useQuery({ queryKey: queryKeys.providerCatalog(null), queryFn: () => getProviderCatalog(null), enabled: open });
  const hooksOverview = useQuery({ queryKey: queryKeys.hooks, queryFn: getHooksOverview, enabled: open && section === "hook" });

  const catalogProviders = useMemo(() => catalog.data?.providers ?? [], [catalog.data?.providers]);
  const providerMap = useMemo(() => new Map(catalogProviders.map((provider) => [provider.provider_id, provider])), [catalogProviders]);
  const initialDraft = useMemo(() => (meta.data ? defaultDraft(meta.data.settings, catalogProviders) : null), [catalogProviders, meta.data]);
  const draft = draftOverride ?? initialDraft;
  const orderedProviderIds = useMemo(() => {
    const current = draft?.agent_order ?? [];
    const missing = catalogProviders.map((provider) => provider.provider_id).filter((id) => !current.includes(id));
    return [...current, ...missing];
  }, [catalogProviders, draft?.agent_order]);

  const saveMutation = useMutation({
    mutationFn: async (current: SettingsDraft) => {
      const settingsBody: UpdateSettingsPayload = {
        sessions_per_provider: Math.max(1, Math.min(200, Number(current.sessions_per_provider || 12))),
        language: current.language,
        show_opencode_subagents: current.show_opencode_subagents,
        sort_providers_by_session_count: current.sort_providers_by_session_count,
        default_backup_dir: current.default_backup_dir || "./backups",
        logging: current.logging,
        home_buttons: current.home_buttons,
        agent_order: current.agent_order,
        primary_agents: current.primary_agents,
      };
      await updateSettings(settingsBody);
      await updateProviderCatalog({
        sort_order: { global: current.agent_order, workspace: [] },
        hidden_state: { global: current.hidden_agents, workspace: [] },
      });
      return getMeta();
    },
    onSuccess: async (nextMeta) => {
      queryClient.setQueryData(queryKeys.meta, nextMeta);
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.providers }),
        queryClient.invalidateQueries({ queryKey: queryKeys.providerCatalog(null) }),
        queryClient.invalidateQueries({ queryKey: queryKeys.agentsSummary }),
        queryClient.invalidateQueries({ queryKey: queryKeys.sessionsRoot }),
        queryClient.invalidateQueries({ queryKey: queryKeys.home }),
      ]);
      toast.success(t("saved"), { description: t("settings") });
      setLanguageOverride(null);
      onOpenChange(false);
    },
    onError: (error) => toast.error(t("error"), { description: error instanceof Error ? error.message : String(error) }),
  });

  const updateMutation = useMutation({
    mutationFn: checkForUpdate,
    onMutate: () => {
      setUpdateError("");
      setUpdateResult(null);
    },
    onSuccess: setUpdateResult,
    onError: (error) => setUpdateError(error instanceof Error ? error.message : String(error)),
  });

  const hookOperationMutation = useMutation({
    mutationFn: ({ provider, operation }: { provider: string; operation: string }) =>
      runHookProviderOperation(provider, operation),
    onSuccess: async (report, variables) => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.hooks }),
        queryClient.invalidateQueries({ queryKey: queryKeys.hookProvider(variables.provider) }),
        queryClient.invalidateQueries({ queryKey: queryKeys.agent(variables.provider) }),
        queryClient.invalidateQueries({ queryKey: queryKeys.agentsSummary }),
      ]);
      toast.success(hookOperationLabel(variables.operation), { description: report.message || variables.provider });
    },
    onError: (error) => toast.error(t("error"), { description: error instanceof Error ? error.message : String(error) }),
  });

  function patchDraft(patch: Partial<SettingsDraft>) {
    if (patch.language !== undefined) setLanguageOverride(patch.language);
    setDraftOverride((current) => {
      const base = current ?? initialDraft;
      return base ? { ...base, ...patch } : base;
    });
  }

  function setHomeButton(key: string, checked: boolean) {
    setDraftOverride((current) => {
      const base = current ?? initialDraft;
      return base ? { ...base, home_buttons: { ...base.home_buttons, [key]: checked } } : base;
    });
  }

  function setHiddenAgent(id: string, checked: boolean) {
    setDraftOverride((current) => {
      const base = current ?? initialDraft;
      if (!base) return base;
      const hidden = new Set(base.hidden_agents);
      if (checked) hidden.add(id);
      else hidden.delete(id);
      return { ...base, hidden_agents: [...hidden] };
    });
  }

  function setAgentOrder(next: string[]) {
    setDraftOverride((current) => {
      const base = current ?? initialDraft;
      return base ? { ...base, agent_order: next } : base;
    });
  }

  function shiftAgent(index: number, direction: "up" | "down") {
    const next = [...orderedProviderIds];
    const target = direction === "up" ? index - 1 : index + 1;
    if (target < 0 || target >= next.length) return;
    [next[index], next[target]] = [next[target], next[index]];
    setAgentOrder(next);
  }

  const settingsPaths = meta.data?.settings_paths;
  const configFile = meta.data?.config_file;

  function handleOpenChange(nextOpen: boolean) {
    if (!nextOpen) {
      setDraftOverride(null);
      setUpdateError("");
      setUpdateResult(null);
      setLanguageOverride(null);
    }
    onOpenChange(nextOpen);
  }

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent className="flex h-[min(760px,calc(100dvh-32px))] flex-col gap-0 p-0 sm:max-w-3xl" data-settings-dialog>
        <DialogHeader className="flex-row items-center border-b px-4 py-2.5 sm:px-5">
          <DialogTitle className="flex-1">{t("settings")}</DialogTitle>
        </DialogHeader>

        <div className="grid min-h-0 flex-1 grid-cols-[164px_minmax(0,1fr)]" data-settings-layout>
          <nav className="flex flex-col gap-1 border-r p-3" aria-label={t("settings")} data-settings-sidebar>
            {SECTIONS.map((item) => (
              <Button
                key={item.id}
                type="button"
                variant={section === item.id ? "secondary" : "ghost"}
                className="justify-start"
                onClick={() => setSection(item.id)}
              >
                {t(item.labelKey)}
              </Button>
            ))}
          </nav>

          <ScrollArea className="min-h-0">
            <div className="flex flex-col gap-5 p-4 sm:p-5">
              {!draft || meta.isLoading ? <SettingsLoading label={t("loadingSettings")} /> : null}

              {draft && section === "general" ? (
                <section className="flex flex-col gap-4" data-settings-section="general">
                  <SectionHead title={t("general")} />
                  <FieldGroup>
                    <Field orientation="responsive">
                      <FieldContent>
                        <FieldTitle>{t("language")}</FieldTitle>
                        <FieldDescription>{t("chooseLanguage")}</FieldDescription>
                      </FieldContent>
                      <Select value={draft.language} onValueChange={(value) => patchDraft({ language: value as UiLanguage })}>
                        <SelectTrigger className="w-44"><SelectValue /></SelectTrigger>
                        <SelectContent><SelectGroup><SelectItem value="zh">{t("languageNativeZh")}</SelectItem><SelectItem value="en">{t("languageNativeEn")}</SelectItem><SelectItem value="auto">{t("auto")}</SelectItem></SelectGroup></SelectContent>
                      </Select>
                    </Field>
                    <Field orientation="responsive">
                      <FieldContent>
                        <FieldTitle>{t("backupDir")}</FieldTitle>
                      </FieldContent>
                      <InputGroup className="max-w-xl">
                        <InputGroupAddon align="inline-start" className="min-w-0 max-w-[min(100%,14rem)] shrink pointer-events-none">
                          <PathText value={settingsPaths?.backup_dir_base} wrap="truncate" title={settingsPaths?.backup_dir_base || undefined} />
                        </InputGroupAddon>
                        <InputGroupAddon align="inline-start" className="pointer-events-none px-1 text-muted-foreground" aria-hidden="true">+</InputGroupAddon>
                        <InputGroupInput value={draft.default_backup_dir} onChange={(event) => patchDraft({ default_backup_dir: event.target.value })} placeholder="./backups" aria-label={t("backupDir")} />
                      </InputGroup>
                    </Field>
                    <ReadOnlyRow title={t("logDir")} value={settingsPaths?.log_dir || "~/.memorph/logs"} description={t("logDirHint")} />
                    <ReadOnlyRow title={t("logFileName")} value={settingsPaths?.log_file_name || "memorph.log"} description={settingsPaths?.log_file_path || "~/.memorph/logs/memorph.log"} />
                    <Field orientation="responsive">
                      <FieldContent><FieldTitle>{t("logMaxSizeMb")}</FieldTitle><FieldDescription>{t("logMaxSizeMbHint")}</FieldDescription></FieldContent>
                      <Input className="w-32" value={logSizeMb(draft)} inputMode="decimal" onChange={(event) => patchDraft({ logging: { ...draft.logging, max_size_bytes: Math.max(0, Number(event.target.value || 0) * 1024 * 1024) } })} aria-label={t("logMaxSizeMb")} />
                    </Field>
                    <Field orientation="responsive">
                      <FieldContent><FieldTitle>{t("logRetentionDays")}</FieldTitle><FieldDescription>{t("logRetentionDaysHint")}</FieldDescription></FieldContent>
                      <Input className="w-32" value={draft.logging.retention_days ?? ""} inputMode="numeric" placeholder={t("unlimited")} onChange={(event) => patchDraft({ logging: { ...draft.logging, retention_days: event.target.value === "" ? null : Math.max(0, Number(event.target.value)) } })} aria-label={t("logRetentionDays")} />
                    </Field>
                  </FieldGroup>
                </section>
              ) : null}

              {draft && section === "display" ? (
                <section className="flex flex-col gap-4" data-settings-section="display">
                  <SectionHead title={t("display")} />
                  <FieldGroup>
                    <Field orientation="responsive">
                      <FieldContent><FieldTitle>{t("sessionsPerProvider")}</FieldTitle><FieldDescription>{t("sessionsPerProviderHint")}</FieldDescription></FieldContent>
                      <Input className="w-32" type="number" min={1} max={200} value={draft.sessions_per_provider} onChange={(event) => patchDraft({ sessions_per_provider: Number(event.target.value || 1) })} />
                    </Field>
                    <Field>
                      <FieldTitle>{t("homeButtons")}</FieldTitle>
                      <FieldDescription>{t("homeButtonsHint")}</FieldDescription>
                      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
                        {HOME_BUTTONS.map(([key, labelKey]) => (
                          <label key={key} className="flex items-center gap-2 rounded-md border p-2 text-sm">
                            <Checkbox checked={Boolean(draft.home_buttons[key])} onCheckedChange={(checked) => setHomeButton(key, checked === true)} />
                            <span>{t(labelKey as I18nKey)}</span>
                          </label>
                        ))}
                      </div>
                    </Field>
                  </FieldGroup>
                </section>
              ) : null}

              {draft && section === "order" ? (
                <section className="flex flex-col gap-4" data-settings-section="order">
                  <SectionHead title={t("order")} />
                  <AgentOrderList
                    orderedProviderIds={orderedProviderIds}
                    providerMap={providerMap}
                    hiddenAgents={draft.hidden_agents}
                    onReorder={setAgentOrder}
                    onHiddenChange={setHiddenAgent}
                    onShift={shiftAgent}
                    t={t}
                  />
                </section>
              ) : null}

              {section === "hook" ? (
                <section className="flex flex-col gap-4" data-settings-section="hook">
                  <SectionHead title={t("hooks")} />
                  <HookSettingsSection
                    overview={hooksOverview.data}
                    isLoading={hooksOverview.isLoading}
                    error={hooksOverview.error}
                    pendingProvider={hookOperationMutation.variables?.provider ?? null}
                    pendingOperation={hookOperationMutation.isPending ? hookOperationMutation.variables?.operation ?? null : null}
                    onRun={(provider, operation) => hookOperationMutation.mutate({ provider, operation })}
                  />
                </section>
              ) : null}

              {section === "config" ? (
                <section className="flex flex-col gap-4" data-settings-section="config">
                  <SectionHead title={t("configFile")} />
                  <ReadOnlyRow title={t("configFileLocation")} value={configFile?.path || "-"} />
                  <Textarea className="min-h-80 font-mono text-xs" readOnly value={configFile?.content || ""} />
                </section>
              ) : null}

              {section === "about" ? (
                <section className="flex flex-col gap-4" data-settings-section="about">
                  <SectionHead title={t("about")} />
                  <div className="flex items-center justify-between gap-3 rounded-md border p-3">
                    <div><strong>{t("version")}</strong><div className="text-muted-foreground text-sm">v{meta.data?.version || ""}</div></div>
                    <Button type="button" variant="outline" disabled={updateMutation.isPending} onClick={() => updateMutation.mutate()}><RefreshCwIcon data-icon="inline-start" />{t("checkUpdate")}</Button>
                  </div>
                  {updateResult ? <UpdateResult result={updateResult} t={t} /> : null}
                  {updateError ? <div className="rounded-md border p-3 text-sm text-destructive">{t("updateCheckFailed", { error: updateError })}</div> : null}
                  <div className="flex flex-col gap-2">
                    {ABOUT_LINKS.map(({ label, url, iconUrl }) => (
                      <Button key={url} type="button" variant="outline" className="h-auto justify-start gap-3 py-3" onClick={() => openUrl(url)}>
                        <span className="inline-flex size-8 shrink-0 items-center justify-center rounded-lg border bg-background" aria-hidden="true">
                          <img src={iconUrl} alt="" className="size-4 object-contain" loading="lazy" />
                        </span>
                        <span className="min-w-0 truncate text-left"><strong>{label}</strong><span className="block truncate text-xs text-muted-foreground">{url}</span></span>
                      </Button>
                    ))}
                  </div>
                </section>
              ) : null}
            </div>
          </ScrollArea>
        </div>

        <DialogFooter className="-mx-0 -mb-0 gap-2 border-t px-4 py-2.5 sm:px-5">
          <Button type="button" variant="outline" onClick={() => handleOpenChange(false)}>
            {t("cancel")}
          </Button>
          <Button type="button" disabled={!draft || saveMutation.isPending} onClick={() => draft && saveMutation.mutate(draft)}>
            {saveMutation.isPending ? <Spinner data-icon="inline-start" /> : null}
            {t("save")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function SectionHead({ title }: { title: string }) {
  return <div className="border-b pb-2"><h3 className="text-base font-semibold">{title}</h3></div>;
}

function ReadOnlyRow({ title, value, description }: { title: string; value: string; description?: string }) {
  return (
    <>
      <Field orientation="responsive">
        <FieldContent><FieldTitle>{title}</FieldTitle>{description ? <FieldDescription>{description}</FieldDescription> : null}</FieldContent>
        <div className="max-w-xl truncate rounded-md border px-3 py-2 font-mono text-xs text-muted-foreground">{value}</div>
      </Field>
      <Separator className="last:hidden" />
    </>
  );
}

function HookSettingsSection({
  overview,
  isLoading,
  error,
  pendingProvider,
  pendingOperation,
  onRun,
}: {
  overview: HookOverviewPayload | undefined;
  isLoading: boolean;
  error: Error | null;
  pendingProvider: string | null;
  pendingOperation: string | null;
  onRun: (provider: string, operation: string) => void;
}) {
  if (isLoading && !overview) return <SettingsLoading label="Loading hooks" />;
  if (error) return <div className="rounded-md border p-3 text-sm text-destructive">{error.message}</div>;

  const providers = (overview?.providers ?? [])
    .filter((provider) => provider.hook_profile)
    .sort((left, right) => (left.name || left.provider_id).localeCompare(right.name || right.provider_id));

  if (!providers.length) {
    return <div className="rounded-md border p-3 text-sm text-muted-foreground">No managed hook providers were returned by the backend.</div>;
  }

  return (
    <div className="flex flex-col gap-4">
      {overview ? (
        <div className="grid gap-3 sm:grid-cols-3">
          <HookSummaryTile label="Providers" value={overview.summary.supported_providers} />
          <HookSummaryTile label="Installed" value={overview.summary.installed_ok} />
          <HookSummaryTile label="Active Runtime" value={overview.summary.active_runtime_sessions} />
        </div>
      ) : null}

      <div className="flex flex-col rounded-md border">
        {providers.map((provider, index) => {
          const operations = hookOperationIds(provider);
          const pending = pendingProvider === provider.provider_id;
          return (
            <div key={provider.provider_id} className="flex flex-col gap-3 border-b p-3 last:border-b-0">
              <div className="flex flex-wrap items-start justify-between gap-3">
                <div className="min-w-0 flex-1">
                  <div className="flex flex-wrap items-center gap-2">
                    <strong className="text-sm font-medium">{provider.name || provider.provider_id}</strong>
                    <Badge variant={hookStatus(provider) === "installed_ok" ? "secondary" : "outline"}>{hookStatus(provider)}</Badge>
                  </div>
                  <div className="mt-1 flex flex-wrap gap-x-3 gap-y-1 text-xs text-muted-foreground">
                    <span className="font-mono">{provider.provider_id}</span>
                    <span>Version {hookVersion(provider)}</span>
                    <span>Last event {formatDateTime(provider.hook?.last_event_at)}</span>
                  </div>
                  {provider.hook?.message ? <p className="mt-2 text-sm text-muted-foreground">{provider.hook.message}</p> : null}
                </div>
                <div className="flex flex-wrap justify-end gap-2">
                  {operations.map((operation) => (
                    <Button
                      key={operation}
                      type="button"
                      size="sm"
                      variant={operation === "uninstall_hook" ? "destructive" : index === 0 && operation === "install_hook" ? "default" : "outline"}
                      disabled={Boolean(pendingOperation)}
                      onClick={() => onRun(provider.provider_id, operation)}
                    >
                      {pending && pendingOperation === operation ? <Spinner data-icon="inline-start" /> : <WrenchIcon data-icon="inline-start" />}
                      {pending && pendingOperation === operation ? "Running" : hookOperationLabel(operation)}
                    </Button>
                  ))}
                </div>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

function HookSummaryTile({ label, value }: { label: string; value: string | number }) {
  return (
    <div className="rounded-md border p-3">
      <div className="text-xs text-muted-foreground">{label}</div>
      <div className="mt-1 text-lg font-semibold">{value}</div>
    </div>
  );
}

function SettingsLoading({ label }: { label: string }) {
  return <div className="flex items-center gap-2 text-sm text-muted-foreground"><Spinner />{label}</div>;
}

function UpdateResult({ result, t }: { result: UpdateCheckPayload; t: (key: I18nKey, vars?: Record<string, string | number | null | undefined>) => string }) {
  return (
    <div className="flex flex-col gap-1 rounded-md border p-3 text-sm">
      <strong>{result.has_update ? t("updateAvailable") : t("upToDate")}</strong>
      <span className="text-muted-foreground">Install Source: {result.install_source_label} · Latest Version: v{result.latest_version}</span>
      <span className="font-mono text-xs text-muted-foreground">{t("updateCommand", { command: result.update_command })}</span>
    </div>
  );
}
