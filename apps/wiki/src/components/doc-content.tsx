import Link from "next/link";
import type { BreadcrumbItem } from "@/lib/paths";
import { Markdown } from "@/lib/markdown";

type DocContentProps = {
  title: string;
  summary?: string;
  content: string;
  breadcrumbs: BreadcrumbItem[];
};

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function stripLeadingTitleHeading(content: string, title: string): string {
  const escapedTitle = escapeRegExp(title.trim());
  const pattern = new RegExp(`^#\\s+${escapedTitle}\\s*\\r?\\n+`, "i");
  return content.replace(pattern, "").trimStart();
}

export function DocContent({ title, summary, content, breadcrumbs }: DocContentProps) {
  const bodyContent = stripLeadingTitleHeading(content, title);

  return (
    <article className="wiki-article">
      <nav className="wiki-breadcrumbs" aria-label="Breadcrumb">
        {breadcrumbs.map((item, index) => (
          <span key={item.href}>
            {index > 0 ? <span className="wiki-breadcrumb-sep">/</span> : null}{" "}
            <Link href={item.href}>{item.label}</Link>
          </span>
        ))}
      </nav>

      <header className="wiki-article-header">
        <h1>{title}</h1>
        {summary ? <p className="wiki-article-summary">{summary}</p> : null}
      </header>

      <div className="wiki-article-body">
        <Markdown content={bodyContent} />
      </div>

      <footer className="wiki-footer-note">This page is rendered directly from repository Markdown.</footer>
    </article>
  );
}
