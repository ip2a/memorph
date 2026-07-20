import { useEffect, useState, type ReactNode } from "react";
import { useTheme } from "next-themes";
import { codeToHtml } from "shiki";
import { cn } from "@/lib/utils";

function extractLanguage(className?: string) {
  if (!className) return "text";
  const match = className.match(/language-([\w-]+)/);
  return match?.[1] ?? "text";
}

export function MarkdownCodeBlock({
  code,
  className,
}: {
  code: string;
  className?: string;
}) {
  const { resolvedTheme } = useTheme();
  const language = extractLanguage(className);
  const [html, setHtml] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    void codeToHtml(code, {
      lang: language,
      theme: resolvedTheme === "dark" ? "github-dark" : "github-light",
    })
      .then((next) => {
        if (!cancelled) setHtml(next);
      })
      .catch(() => {
        if (!cancelled) setHtml(null);
      });

    return () => {
      cancelled = true;
    };
  }, [code, language, resolvedTheme]);

  return (
    <div className="not-prose my-3 overflow-hidden rounded-lg border border-border bg-card">
      <div className="border-b border-border px-3 py-1.5 font-mono text-[11px] uppercase tracking-wide text-muted-foreground">
        {language}
      </div>
      {html ? (
        <div
          className="overflow-x-auto text-[13px] leading-relaxed [&_pre]:m-0 [&_pre]:bg-transparent [&_pre]:p-3"
          dangerouslySetInnerHTML={{ __html: html }}
        />
      ) : (
        <pre className="overflow-x-auto p-3 font-mono text-xs leading-relaxed whitespace-pre-wrap break-words">
          <code>{code}</code>
        </pre>
      )}
    </div>
  );
}

export function MarkdownInlineCode({
  children,
  className,
}: {
  children: ReactNode;
  className?: string;
}) {
  return (
    <code className={cn("rounded bg-muted px-1.5 py-0.5 font-mono text-[0.85em] text-foreground", className)}>
      {children}
    </code>
  );
}
