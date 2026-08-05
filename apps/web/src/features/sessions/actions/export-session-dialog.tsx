import { zodResolver } from "@hookform/resolvers/zod";
import { useMutation } from "@tanstack/react-query";
import { FolderOpenIcon } from "lucide-react";
import { useEffect } from "react";
import { useForm, useWatch } from "react-hook-form";
import { toast } from "sonner";
import { DialogForm, DialogFormFooter } from "@/components/shared/dialog-form";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Field, FieldDescription, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { InputGroup, InputGroupAddon, InputGroupButton, InputGroupInput } from "@/components/ui/input-group";
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import type { SessionActionTarget } from "@/features/sessions/session-action-target";
import { exportSchema, providerLabel, workspaceOptions } from "@/features/sessions/model/schemas";
import type { ExportForm } from "@/features/sessions/model/schemas";
import { exportSession } from "@/lib/api";
import { useI18n } from "@/lib/i18n-context";
import type { MetaPayload } from "@/lib/types";

export function ExportSessionDialog({
  target,
  open,
  onOpenChange,
  meta,
}: {
  target: SessionActionTarget | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  meta?: MetaPayload;
}) {
  const { t } = useI18n();
  const form = useForm<ExportForm>({
    resolver: zodResolver(exportSchema),
    defaultValues: { output_prefix: "", format: "both", output_dir: "" },
  });

  useEffect(() => {
    if (!open || !target) return;
    form.reset({
      output_prefix: target.sessionId,
      format: "both",
      output_dir: target.workspace || meta?.selected_workspace || "",
    });
  }, [form, meta?.selected_workspace, open, target]);

  const exportMutation = useMutation({
    mutationFn: (values: ExportForm) => {
      if (!target) throw new Error("Missing session target");
      return exportSession({
        provider: target.providerId,
        session_id: target.sessionId,
        output_prefix: values.output_prefix.trim() || null,
        format: values.format,
        output_dir: values.output_dir?.trim() || null,
      });
    },
    onSuccess: (result) => {
      onOpenChange(false);
      toast.success(t("sessionExported"), {
        description: result.files.length ? result.files.join("\n") : providerLabel([], target?.providerId || ""),
      });
    },
  });

  const selectedFormat = useWatch({ control: form.control, name: "format" });

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-2xl" data-export-session-dialog>
        <DialogHeader>
          <DialogTitle>{t("export")}</DialogTitle>
          <DialogDescription>{t("sessionExportDescription")}</DialogDescription>
        </DialogHeader>
        <DialogForm onSubmit={form.handleSubmit((values) => exportMutation.mutate(values))}>
          <input type="hidden" name="provider" value={target?.providerId || ""} />
          <input type="hidden" name="session_id" value={target?.sessionId || ""} />
          <FieldGroup className="grid gap-4 sm:grid-cols-[minmax(0,1fr)_11rem]" data-export-modal-grid>
            <Field data-invalid={Boolean(form.formState.errors.output_prefix)} className="sm:col-span-2">
              <FieldLabel htmlFor="export-output-prefix">{t("sessionOutputFileName")}</FieldLabel>
              <Input
                id="export-output-prefix"
                placeholder={t("sessionOutputFileNamePlaceholder")}
                aria-invalid={Boolean(form.formState.errors.output_prefix)}
                {...form.register("output_prefix")}
              />
              {form.formState.errors.output_prefix ? <FieldDescription>{form.formState.errors.output_prefix.message}</FieldDescription> : null}
            </Field>

            <Field data-invalid={Boolean(form.formState.errors.format)}>
              <FieldLabel htmlFor="export-format">{t("sessionFormat")}</FieldLabel>
              <Select value={selectedFormat} onValueChange={(value) => form.setValue("format", value, { shouldValidate: true })}>
                <SelectTrigger id="export-format" className="w-full" aria-invalid={Boolean(form.formState.errors.format)}>
                  <SelectValue placeholder={t("sessionFormat")} />
                </SelectTrigger>
                <SelectContent>
                  <SelectGroup>
                    {[
                      "json",
                      "md",
                      "html",
                      "morph",
                      "both",
                    ].map((format) => (
                      <SelectItem key={format} value={format}>
                        {format}
                      </SelectItem>
                    ))}
                  </SelectGroup>
                </SelectContent>
              </Select>
              {form.formState.errors.format ? <FieldDescription>{form.formState.errors.format.message}</FieldDescription> : null}
            </Field>

            <Field>
              <FieldLabel htmlFor="export-output-dir">{t("sessionExportDirectory")}</FieldLabel>
              <InputGroup>
                <InputGroupInput id="export-output-dir" list="known-workspaces" placeholder={t("sessionExportDirectory")} {...form.register("output_dir")} />
                <InputGroupAddon align="inline-end">
                  <InputGroupButton type="button" variant="ghost" disabled>
                    <FolderOpenIcon data-icon="inline-start" />
                    {t("sessionBrowse")}
                  </InputGroupButton>
                </InputGroupAddon>
              </InputGroup>
              <FieldDescription>{t("sessionExportDirectoryDescription")}</FieldDescription>
            </Field>
          </FieldGroup>

          <datalist id="known-workspaces">
            {workspaceOptions(meta).map((item) => (
              <option key={item.path} value={item.path} />
            ))}
          </datalist>

          <DialogFormFooter
            onCancel={() => onOpenChange(false)}
            cancelLabel={t("cancel")}
            submitDisabled={!target}
            submitLabel={t("export")}
            submitting={exportMutation.isPending}
          />
        </DialogForm>
      </DialogContent>
    </Dialog>
  );
}
