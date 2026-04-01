import type { DocPage, DocTreeNode } from "@/lib/docs";
import type { BreadcrumbItem } from "@/lib/paths";
import { Sidebar } from "@/components/sidebar";
import { DocContent } from "@/components/doc-content";

type WikiShellProps = {
  nodes: DocTreeNode[];
  currentRoute: string;
  page: DocPage;
  breadcrumbs: BreadcrumbItem[];
};

export function WikiShell({ nodes, currentRoute, page, breadcrumbs }: WikiShellProps) {
  return (
    <main className="wiki-page">
      <div className="wiki-shell">
        <Sidebar nodes={nodes} currentRoute={currentRoute} />
        <div className="wiki-divider" aria-hidden="true" />
        <section className="wiki-content">
          <DocContent
            title={page.title}
            summary={page.summary}
            content={page.content}
            breadcrumbs={breadcrumbs}
          />
        </section>
      </div>
    </main>
  );
}
