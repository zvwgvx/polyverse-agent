import type { ReactNode } from "react";
import { getTopLevelSections } from "@/lib/docs";
import { SidebarLayout } from "@/components/sidebar-layout";

export default function DocsLayout({ children }: { children: ReactNode }) {
  return (
    <SidebarLayout nodes={getTopLevelSections()}>
      {children}
    </SidebarLayout>
  );
}