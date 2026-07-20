import type { ReactNode } from "react";
import ReactMarkdown from "react-markdown";
import type { Components } from "react-markdown";
import remarkBreaks from "remark-breaks";
import remarkGfm from "remark-gfm";
import { MarkdownCodeBlock, MarkdownInlineCode } from "@/components/shared/markdown-code-block";
import { cn } from "@/lib/utils";

const markdownComponents: Partial<Components> = {
  pre({ children }) {
    return <>{children}</>;
  },
  code({ className, children }) {
    const text = String(children ?? "").replace(/\n$/, "");
    const isBlock = Boolean(className?.includes("language-")) || text.includes("\n");

    if (isBlock) {
      return <MarkdownCodeBlock className={className} code={text} />;
    }

    return <MarkdownInlineCode className={className}>{children}</MarkdownInlineCode>;
  },
  a({ href, children }) {
    return (
      <a href={href} target="_blank" rel="noreferrer noopener">
        {children}
      </a>
    );
  },
};

const proseClassName = cn(
  "prose prose-sm dark:prose-invert max-w-none text-foreground",
  "prose-headings:scroll-m-20 prose-headings:font-semibold prose-headings:tracking-tight prose-headings:text-foreground",
  "prose-p:leading-7 prose-p:text-foreground",
  "prose-strong:text-foreground prose-strong:font-semibold",
  "prose-blockquote:border-border prose-blockquote:text-muted-foreground",
  "prose-a:font-medium prose-a:text-primary prose-a:underline prose-a:underline-offset-4",
  "prose-code:before:content-none prose-code:after:content-none",
  "prose-pre:p-0 prose-pre:bg-transparent",
  "prose-table:text-sm",
  "prose-th:border-border prose-td:border-border",
  "prose-hr:border-border",
);

export function MarkdownContent({
  value,
  className,
}: {
  value: string;
  className?: string;
}) {
  return (
    <div className={cn(proseClassName, className)}>
      <ReactMarkdown remarkPlugins={[remarkGfm, remarkBreaks]} components={markdownComponents}>
        {value}
      </ReactMarkdown>
    </div>
  );
}

export function MarkdownContentFrame({
  value,
  embedded = false,
}: {
  value: string;
  embedded?: boolean;
}): ReactNode {
  if (embedded) {
    return <MarkdownContent value={value} />;
  }

  return (
    <div className="max-h-80 overflow-auto rounded-md border border-border bg-muted/20 p-3">
      <MarkdownContent value={value} />
    </div>
  );
}
