import { zodResolver } from "@hookform/resolvers/zod";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { FolderOpenIcon } from "lucide-react";
import { useEffect, useMemo } from "react";
import { useForm, useWatch } from "react-hook-form";
import { useNavigate } from "react-router-dom";
import { toast } from "sonner";
import { z } from "zod";
import { DialogForm, DialogFormFooter } from "@/components/shared/dialog-form";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Field, FieldDescription, FieldGroup, FieldLabel } from "@/components/ui/field";
import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
} from "@/components/ui/input-group";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { getMeta, importSession, listProviders, selectFile, selectFolder } from "@/lib/api";
import { useI18n } from "@/lib/i18n-context";
import { queryKeys } from "@/lib/query-keys";
import { useUiStore } from "@/stores/ui-store";

const importSessionSchema = (messages: { providerRequired: string; fileOrIdRequired: string }) =>
  z.object({
    provider: z.string().min(1, messages.providerRequired),
    file_or_id: z.string().trim().min(1, messages.fileOrIdRequired),
    to_dir: z.string().trim().optional(),
  });

type ImportSessionForm = z.infer<ReturnType<typeof importSessionSchema>>;

export function ImportSessionDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (open: boolean) => void }) {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const selectedWorkspace = useUiStore((state) => state.selectedWorkspace);
  const { t } = useI18n();
  const schema = useMemo(
    () => importSessionSchema({ providerRequired: t("importProviderRequired"), fileOrIdRequired: t("importFileOrIdRequired") }),
    [t],
  );

  const providers = useQuery({
    queryKey: queryKeys.providers,
    queryFn: listProviders,
  });

  const meta = useQuery({
    queryKey: queryKeys.meta,
    queryFn: getMeta,
  });

  const importProviders = useMemo(() => providers.data ?? [], [providers.data]);
  const fallbackProvider = importProviders[0]?.id ?? "";
  const workspace = selectedWorkspace || meta.data?.selected_workspace || "";

  const form = useForm<ImportSessionForm>({
    resolver: zodResolver(schema),
    defaultValues: {
      provider: "",
      file_or_id: "",
      to_dir: "",
    },
  });
  const selectedProvider = useWatch({ control: form.control, name: "provider" });

  useEffect(() => {
    if (!open) return;
    form.reset({ provider: fallbackProvider, file_or_id: "", to_dir: workspace });
  }, [fallbackProvider, form, open, workspace]);

  const importMutation = useMutation({
    mutationFn: importSession,
    onSuccess: async (result, variables) => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.sessionsRoot }),
        queryClient.invalidateQueries({ queryKey: queryKeys.home }),
      ]);
      onOpenChange(false);
      form.reset({ provider: variables.provider, file_or_id: "", to_dir: variables.to_dir || workspace });
      toast.success(t("imported"), {
        description: result.resume_command || `${result.provider_name}: ${result.new_session_id}`,
      });
      navigate(`/sessions/${encodeURIComponent(variables.provider)}/${encodeURIComponent(result.new_session_id)}`);
    },
  });

  function submitImport(values: ImportSessionForm) {
    importMutation.mutate({
      provider: values.provider,
      file_or_id: values.file_or_id.trim(),
      to_dir: values.to_dir?.trim() || null,
    });
  }

  async function browseImportFile() {
    try {
      const result = await selectFile({ start_path: form.getValues("file_or_id") || workspace || null });
      if (result.path) form.setValue("file_or_id", result.path, { shouldValidate: true });
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      toast.error(t("error"), {
        description: /only available in the desktop app/i.test(message) ? t("importFilePickerDesktopOnly") : message,
      });
    }
  }

  async function browseTargetDir() {
    try {
      const result = await selectFolder({ start_path: form.getValues("to_dir") || workspace || null });
      if (result.path) form.setValue("to_dir", result.path, { shouldValidate: true });
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      toast.error(t("error"), {
        description: /only available in the desktop app/i.test(message) ? t("importFolderPickerDesktopOnly") : message,
      });
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-xl" data-import-session-dialog>
        <DialogHeader>
          <DialogTitle>{t("importSession")}</DialogTitle>
          <DialogDescription>{t("importSessionDescription")}</DialogDescription>
        </DialogHeader>

        <DialogForm onSubmit={form.handleSubmit(submitImport)}>
          <FieldGroup className="grid gap-4 sm:grid-cols-[minmax(0,0.7fr)_minmax(0,1.3fr)]" data-import-modal-grid>
            <Field data-invalid={Boolean(form.formState.errors.provider)}>
              <FieldLabel htmlFor="import-provider">{t("importTargetProvider")}</FieldLabel>
              <Select value={selectedProvider} onValueChange={(value) => form.setValue("provider", value, { shouldValidate: true })}>
                <SelectTrigger id="import-provider" className="w-full" aria-invalid={Boolean(form.formState.errors.provider)}>
                  <SelectValue placeholder={providers.isLoading ? t("loading") : t("provider")} />
                </SelectTrigger>
                <SelectContent>
                  <SelectGroup>
                    {importProviders.map((provider) => (
                      <SelectItem key={provider.id} value={provider.id}>
                        {provider.name}
                      </SelectItem>
                    ))}
                  </SelectGroup>
                </SelectContent>
              </Select>
              {form.formState.errors.provider ? <FieldDescription>{form.formState.errors.provider.message}</FieldDescription> : null}
            </Field>

            <Field data-invalid={Boolean(form.formState.errors.file_or_id)}>
              <FieldLabel htmlFor="import-file-or-id">{t("importFileOrId")}</FieldLabel>
              <InputGroup>
                <InputGroupInput
                  id="import-file-or-id"
                  placeholder={t("importFileOrIdPlaceholder")}
                  aria-invalid={Boolean(form.formState.errors.file_or_id)}
                  {...form.register("file_or_id")}
                />
                <InputGroupAddon align="inline-end">
                  <InputGroupButton type="button" variant="ghost" onClick={browseImportFile}>
                    <FolderOpenIcon data-icon="inline-start" />
                    {t("sessionBrowse")}
                  </InputGroupButton>
                </InputGroupAddon>
              </InputGroup>
              {form.formState.errors.file_or_id ? <FieldDescription>{form.formState.errors.file_or_id.message}</FieldDescription> : null}
            </Field>

            <Field className="sm:col-span-2">
              <FieldLabel htmlFor="import-target-dir">{t("sessionTargetDirectory")}</FieldLabel>
              <InputGroup>
                <InputGroupInput
                  id="import-target-dir"
                  list="known-workspaces"
                  placeholder={workspace || t("sessionWorkspacePath")}
                  {...form.register("to_dir")}
                />
                <InputGroupAddon align="inline-end">
                  <InputGroupButton type="button" variant="ghost" onClick={browseTargetDir}>
                    <FolderOpenIcon data-icon="inline-start" />
                    {t("sessionBrowse")}
                  </InputGroupButton>
                </InputGroupAddon>
              </InputGroup>
              <FieldDescription>{t("sessionTargetDirectoryDescription")}</FieldDescription>
            </Field>
          </FieldGroup>

          <datalist id="known-workspaces">
            {(meta.data?.workspaces ?? []).map((item) => (
              <option key={item.path} value={item.path} />
            ))}
          </datalist>

          <DialogFormFooter
            onCancel={() => onOpenChange(false)}
            cancelLabel={t("cancel")}
            submitDisabled={!importProviders.length}
            submitLabel={t("import")}
            submitting={importMutation.isPending}
          />
        </DialogForm>
      </DialogContent>
    </Dialog>
  );
}
