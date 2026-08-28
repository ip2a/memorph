import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { RefreshCwIcon } from "lucide-react";
import { useMemo, useState, type ReactNode } from "react";
import { toast } from "sonner";
import { MemorphLogo } from "@/components/shared/memorph-logo";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Field, FieldContent, FieldDescription, FieldGroup, FieldLegend, FieldSet, FieldTitle } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { ScrollPane } from "@/components/shared/scroll-pane";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Spinner } from "@/components/ui/spinner";
import { Toggle } from "@/components/ui/toggle";
import {
  checkForUpdate,
  getMeta,
  getProviderCatalog,
  openExternal,
  updateProviderCatalog,
  updateSettings,
} from "@/lib/api";
import { looksLikeJson } from "@/lib/format-content";
import { useI18n } from "@/lib/i18n-context";
import type { I18nKey } from "@/lib/i18n-core";
import { queryKeys } from "@/lib/query-keys";
import { cn } from "@/lib/utils";
import type { HomeSessionLayout, ProviderCatalogEntry, SettingsPayload, UiLanguage, UpdateCheckPayload, UpdateSettingsPayload } from "@/lib/types";
import { AgentOrderList } from "@/features/settings/agent-order-list";
import { IndexSettingsPanel } from "@/features/settings/index-settings-panel";
import { SkillsCatalogPageSizeField } from "@/features/settings/skills-catalog-page-size-field";
import { CustomRangePreferenceField } from "@/features/settings/custom-range-preference-field";
import { clampSkillsCatalogPageSize } from "@/features/skills/skills-catalog-page-size";

const SECTIONS = [
  { id: "general", labelKey: "general" },
  { id: "index", labelKey: "indexSection" },
  { id: "display", labelKey: "display" },
  { id: "order", labelKey: "order" },
  { id: "config", labelKey: "configFile" },
  { id: "about", labelKey: "about" },
] as const;

const SETTINGS_WORKSPACE_TOKEN = "WORKSPACE";
const SETTINGS_EXPORT_DIR_VALUE = "工作空间";

const HOME_BUTTONS = [
  ["view", "view"],
  ["compress", "compression"],
  ["switch", "homeButtonSwitch"],
  ["export", "export"],
  ["sync", "sync"],
  ["rename", "rename"],
  ["delete", "homeButtonDelete"],
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
    skills_catalog_page_size: clampSkillsCatalogPageSize(settings?.skills_catalog_page_size),
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
      compress: settings?.home_buttons?.compress === true,
      switch: settings?.home_buttons?.switch !== false,
      export: settings?.home_buttons?.export === true,
      sync: settings?.home_buttons?.sync === true,
      rename: settings?.home_buttons?.rename !== false,
      delete: settings?.home_buttons?.delete !== false,
    },
    home_session_layout: settings?.home_session_layout ?? "tabs",
    agent_order: agentOrder,
    primary_agents: settings?.primary_agents ?? [],
    server: {
      web_port: settings?.server?.web_port ?? 3737,
      api_port: settings?.server?.api_port ?? 3223,
    },
    hidden_agents: catalog.filter((provider) => provider.hidden_state?.global).map((provider) => provider.provider_id),
  };
}

function clampPort(port: number, fallback: number) {
  return Math.max(1, Math.min(65535, Number(port || fallback)));
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

export function SettingsDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (open: boolean) => void }) {
  const queryClient = useQueryClient();
  const { t, setLanguageOverride } = useI18n();
  const [section, setSection] = useState<SectionId>("general");
  const [draftOverride, setDraftOverride] = useState<SettingsDraft | null>(null);
  const [updateResult, setUpdateResult] = useState<UpdateCheckPayload | null>(null);
  const [updateError, setUpdateError] = useState<string>("");

  const meta = useQuery({ queryKey: queryKeys.meta, queryFn: getMeta, enabled: open });
  const catalog = useQuery({ queryKey: queryKeys.providerCatalog(null), queryFn: () => getProviderCatalog(null), enabled: open });

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
        skills_catalog_page_size: clampSkillsCatalogPageSize(current.skills_catalog_page_size),
        language: current.language,
        show_opencode_subagents: current.show_opencode_subagents,
        sort_providers_by_session_count: current.sort_providers_by_session_count,
        default_backup_dir: current.default_backup_dir || "./backups",
        logging: current.logging,
        home_buttons: current.home_buttons,
        home_session_layout: current.home_session_layout,
        agent_order: current.agent_order,
        primary_agents: current.primary_agents,
        server: {
          web_port: clampPort(current.server.web_port, 3737),
          api_port: clampPort(current.server.api_port, 3223),
        },
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
        queryClient.invalidateQueries({ queryKey: queryKeys.skillsRoot }),
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
      <DialogContent className="flex h-[min(70dvh,640px)] flex-col gap-0 p-0 sm:max-w-3xl" data-settings-dialog>
        <DialogHeader variant="bordered" className="flex-row items-center">
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

          <ScrollPane className="min-h-0 flex-1" innerClassName="flex flex-col gap-5 p-4 sm:p-5">
              {(!draft || meta.isLoading) && section !== "index" ? <SettingsLoading label={t("loadingSettings")} /> : null}

              {section === "index" ? <IndexSettingsPanel /> : null}

              {draft && section === "general" ? (
                <section className="flex flex-col gap-4" data-settings-section="general">
                  <SectionHead title={t("general")} />
                  <FieldGroup data-settings-general-rows>
                    <Field orientation="responsive">
                      <FieldContent><FieldTitle>{t("language")}</FieldTitle></FieldContent>
                      <Select value={draft.language} onValueChange={(value) => patchDraft({ language: value as UiLanguage })}>
                        <SelectTrigger className="w-44"><SelectValue /></SelectTrigger>
                        <SelectContent><SelectGroup><SelectItem value="zh">{t("languageNativeZh")}</SelectItem><SelectItem value="en">{t("languageNativeEn")}</SelectItem><SelectItem value="auto">{t("auto")}</SelectItem></SelectGroup></SelectContent>
                      </Select>
                    </Field>
                    <Field orientation="responsive">
                      <FieldContent><FieldTitle>{t("backupDir")}</FieldTitle><FieldDescription>{t("backupDirHint")}</FieldDescription></FieldContent>
                      <SettingsPathValue
                        value={`${SETTINGS_WORKSPACE_TOKEN}/${formatSettingsPathSuffix(settingsPaths?.backup_dir_input || "./backups")}`}
                      />
                    </Field>
                    <Field orientation="responsive">
                      <FieldContent><FieldTitle>{t("exportDir")}</FieldTitle><FieldDescription>{t("exportDirHint")}</FieldDescription></FieldContent>
                      <SettingsPathValue value={SETTINGS_EXPORT_DIR_VALUE} />
                    </Field>
                  </FieldGroup>
                  <SectionHead title={t("defaultPortsSection")} />
                  <FieldGroup>
                    <Field orientation="responsive">
                      <FieldContent><FieldTitle>{t("webPort")}</FieldTitle><FieldDescription>{t("webPortHint")}</FieldDescription></FieldContent>
                      <Input className="w-32" type="number" min={1} max={65535} value={draft.server.web_port} onChange={(event) => patchDraft({ server: { ...draft.server, web_port: Number(event.target.value || 0) } })} aria-label={t("webPort")} />
                    </Field>
                    <Field orientation="responsive">
                      <FieldContent><FieldTitle>{t("apiPort")}</FieldTitle><FieldDescription>{t("apiPortHint")}</FieldDescription></FieldContent>
                      <Input className="w-32" type="number" min={1} max={65535} value={draft.server.api_port} onChange={(event) => patchDraft({ server: { ...draft.server, api_port: Number(event.target.value || 0) } })} aria-label={t("apiPort")} />
                    </Field>
                  </FieldGroup>
                  <SectionHead title={t("logConfigSection")} />
                  <FieldGroup>
                    <Field orientation="responsive">
                      <FieldContent><FieldTitle>{t("logDir")}</FieldTitle><FieldDescription>{t("logDirHint")}</FieldDescription></FieldContent>
                      <SettingsValueText value={settingsPaths?.log_dir || "~/.memorph/logs"} />
                    </Field>
                    <Field orientation="responsive">
                      <FieldContent><FieldTitle>{t("logFileName")}</FieldTitle></FieldContent>
                      <SettingsValueText value={settingsPaths?.log_file_name || "memorph.log"} />
                    </Field>
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
                    <Field orientation="responsive">
                      <FieldContent><FieldTitle>{t("homeSessionLayout")}</FieldTitle><FieldDescription>{t("homeSessionLayoutHint")}</FieldDescription></FieldContent>
                      <Select
                        value={draft.home_session_layout}
                        onValueChange={(value) => patchDraft({ home_session_layout: value as HomeSessionLayout })}
                      >
                        <SelectTrigger className="w-44"><SelectValue /></SelectTrigger>
                        <SelectContent>
                          <SelectGroup>
                            <SelectItem value="stack">{t("homeSessionLayoutStack")}</SelectItem>
                            <SelectItem value="tabs">{t("homeSessionLayoutTabs")}</SelectItem>
                          </SelectGroup>
                        </SelectContent>
                      </Select>
                    </Field>
                    <FieldSet>
                      <FieldLegend variant="label">{t("homeButtons")}</FieldLegend>
                      <FieldDescription>{t("homeButtonsHint")}</FieldDescription>
                      <div className="flex flex-wrap gap-2">
                        {HOME_BUTTONS.map(([key, labelKey]) => {
                          const enabled = Boolean(draft.home_buttons[key]);
                          return (
                            <Toggle
                              key={key}
                              pressed={enabled}
                              variant="outline"
                              size="sm"
                              className={cn(
                                "rounded-full transition-all",
                                enabled
                                  ? "border-primary/50 bg-primary/10 text-foreground hover:bg-primary/15 dark:bg-primary/15 dark:hover:bg-primary/20"
                                  : "text-muted-foreground line-through opacity-60 hover:opacity-80",
                              )}
                              aria-label={t(labelKey as I18nKey)}
                              onPressedChange={(checked) => setHomeButton(key, checked)}
                            >
                              {t(labelKey as I18nKey)}
                            </Toggle>
                          );
                        })}
                      </div>
                    </FieldSet>
                  </FieldGroup>
                  <SectionHead title={t("timeRangeSection")} />
                  <CustomRangePreferenceField />
                  <SectionHead title={t("skills")} />
                  <FieldGroup>
                    <SkillsCatalogPageSizeField
                      value={draft.skills_catalog_page_size}
                      onChange={(next) => patchDraft({ skills_catalog_page_size: next })}
                    />
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

              {section === "config" ? (
                <section className="flex flex-col gap-4" data-settings-section="config">
                  <SectionHead title={t("configFile")} />
                  <ConfigFilePreview content={configFile?.content || ""} />
                  <ReadOnlyRow title={t("configFileLocation")} value={configFile?.path || "-"} />
                </section>
              ) : null}

              {section === "about" ? (
                <section className="flex flex-col gap-4" data-settings-section="about">
                  <SectionHead title={t("about")} />
                  <div className="divide-y">
                    <div className="flex items-center justify-between gap-3 py-3">
                      <div className="flex min-w-0 items-center gap-3">
                        <MemorphLogo size="md" />
                        <div className="min-w-0">
                          <strong>{t("version")}</strong>
                          <div className="text-muted-foreground text-sm">v{meta.data?.version || ""}</div>
                        </div>
                      </div>
                      <Button type="button" variant="outline" disabled={updateMutation.isPending} onClick={() => updateMutation.mutate()}><RefreshCwIcon data-icon="inline-start" />{t("checkUpdate")}</Button>
                    </div>
                    {updateResult ? <UpdateResult result={updateResult} t={t} /> : null}
                    {updateError ? <div className="py-3 text-sm text-destructive">{t("updateCheckFailed", { error: updateError })}</div> : null}
                    {ABOUT_LINKS.map(({ label, url, iconUrl }) => (
                      <button key={url} type="button" className="flex w-full items-center gap-3 py-3 text-left transition-colors hover:bg-muted/60" onClick={() => openUrl(url)}>
                        <span className="inline-flex size-8 shrink-0 items-center justify-center" aria-hidden="true">
                          <img src={iconUrl} alt="" className="size-4 object-contain" loading="lazy" />
                        </span>
                        <span className="min-w-0 truncate"><strong>{label}</strong><span className="block truncate text-xs text-muted-foreground">{url}</span></span>
                      </button>
                    ))}
                  </div>
                </section>
              ) : null}
          </ScrollPane>
        </div>

        <DialogFooter variant="bordered" className="gap-2">
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

function ConfigFilePreview({ content }: { content: string }) {
  if (!content) {
    return <p className="text-sm text-muted-foreground">-</p>;
  }

  let text = content;
  if (looksLikeJson(content)) {
    try {
      text = JSON.stringify(JSON.parse(content), null, 2);
    } catch {
      text = content;
    }
  }

  return (
    <ScrollArea className="h-80 rounded-md border border-border bg-muted/40">
      <pre className="whitespace-pre-wrap break-words p-3 font-mono text-xs text-foreground">{text}</pre>
    </ScrollArea>
  );
}

function formatSettingsPathSuffix(path: string) {
  return path.replace(/^\.\/+/, "").replace(/^\/+/, "") || path;
}

function SettingsRow({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="flex min-h-10 items-center gap-4 py-3">
      <FieldTitle className="min-w-0 flex-1">{title}</FieldTitle>
      <div className="flex shrink-0 items-center justify-end">{children}</div>
    </div>
  );
}

function SettingsPathValue({ value }: { value: string }) {
  return <SettingsValueText value={value} />;
}

function SettingsValueText({ value }: { value: string }) {
  return <span className="max-w-xl truncate font-mono text-xs text-muted-foreground">{value}</span>;
}

function ReadOnlyRow({ title, value }: { title: string; value: string }) {
  return (
    <SettingsRow title={title}>
      <SettingsValueText value={value} />
    </SettingsRow>
  );
}

function SettingsLoading({ label }: { label: string }) {
  return <div className="flex items-center gap-2 text-sm text-muted-foreground"><Spinner />{label}</div>;
}

function UpdateResult({ result, t }: { result: UpdateCheckPayload; t: (key: I18nKey, vars?: Record<string, string | number | null | undefined>) => string }) {
  return (
    <div className="flex flex-col gap-1 py-3 text-sm">
      <strong>{result.has_update ? t("updateAvailable") : t("upToDate")}</strong>
      <span className="text-muted-foreground">Install Source: {result.install_source_label} · Latest Version: v{result.latest_version}</span>
      <span className="font-mono text-xs text-muted-foreground">{t("updateCommand", { command: result.update_command })}</span>
    </div>
  );
}
