import { z } from "zod";
import type { MetaPayload, ProviderInfo } from "@/lib/types";

export const renameSchema = z.object({
  title: z.string().trim().min(1, "Enter a title."),
});

export type RenameForm = z.infer<typeof renameSchema>;

export const switchSchema = z.object({
  to: z.string().min(1, "Choose a target provider."),
  target_title: z.string().trim().optional(),
  to_dir: z.string().trim().optional(),
});

export type SwitchForm = z.infer<typeof switchSchema>;

export const exportSchema = z.object({
  output_prefix: z.string().trim().min(1, "Enter an output file name."),
  format: z.string().min(1, "Choose a format."),
  output_dir: z.string().trim().optional(),
});

export type ExportForm = z.infer<typeof exportSchema>;

export const createSyncSchema = z.object({
  title: z.string().trim().optional(),
  to_dir: z.string().trim().optional(),
  targets: z.array(z.string()).min(1, "Choose at least one target provider."),
});

export type CreateSyncForm = z.infer<typeof createSyncSchema>;

export function defaultSwitchTarget(providers: ProviderInfo[], sourceProviderId: string) {
  const candidates = providers.filter((provider) => provider.export);
  if (!candidates.length) return "";
  if (sourceProviderId === "codex") {
    return candidates.find((provider) => provider.id === "claude")?.id ?? candidates[0].id;
  }
  return candidates[0].id;
}

export function workspaceOptions(meta?: MetaPayload) {
  return meta?.workspaces ?? [];
}

export function providerLabel(providers: ProviderInfo[], providerId: string) {
  return providers.find((provider) => provider.id === providerId)?.name ?? providerId;
}

export function syncTargetProviders(providers: ProviderInfo[], sourceProviderId: string) {
  return providers.filter((provider) => provider.id !== sourceProviderId && provider.export);
}
