"use client";

import type { ReactNode } from "react";
import { useSelectedLayoutSegments } from "next/navigation";
import type { DocTreeNode } from "@/lib/docs";
import { Sidebar } from "@/components/sidebar";

type SidebarLayoutProps = {
  nodes: DocTreeNode[];
  children: ReactNode;
};

export function SidebarLayout({ nodes, children }: SidebarLayoutProps) {
  const segments = useSelectedLayoutSegments();

  // The segments might contain catch-all array, we need to filter out nullish/empty
  const filtered = segments.filter(s => s && !s.startsWith("("));
  const currentRoute = filtered.length === 0 ? "/" : `/${filtered.join("/")}`;

  return (
    <main className="wiki-page">
      <div className="wiki-shell">
        <Sidebar nodes={nodes} currentRoute={currentRoute} />
        <div className="wiki-divider" aria-hidden="true" />
        <section className="wiki-content">
          {children}
        </section>
      </div>
    </main>
  );
}