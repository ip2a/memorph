import type { SkillCatalogItem } from "@/lib/types";

/** Real on-disk location for a catalog row (directory/copy path, or symlink target). */
export function realPathOf(item: SkillCatalogItem): string | undefined {
  const directory = item.installations.find(
    (installation) =>
      (installation.install_kind === "directory" ||
        installation.install_kind === "managed-copy") &&
      installation.status === "active",
  );
  if (directory) return directory.install_path;
  const symlink = item.installations.find(
    (installation) => installation.install_kind === "symlink",
  );
  return symlink?.symlink_target ?? item.installations[0]?.install_path;
}

export function displayHomePath(path: string) {
  return path.replace(/^\/Users\/[^/]+/, "~");
}
