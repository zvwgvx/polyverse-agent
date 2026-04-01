import { notFound } from "next/navigation";
import type { Metadata } from "next";
import { getDocBySlug, getStaticSlugs, getTopLevelSections } from "@/lib/docs";
import { buildBreadcrumbs, routeFromSlug } from "@/lib/paths";
import { WikiShell } from "@/components/wiki-shell";

export function generateStaticParams() {
  return getStaticSlugs().map((slug) => ({ slug }));
}

export async function generateMetadata({
  params
}: {
  params: Promise<{ slug?: string[] }>;
}): Promise<Metadata> {
  const { slug = [] } = await params;
  const page = getDocBySlug(slug);

  if (!page) {
    return {
      title: "Not Found | Polyverse Wiki"
    };
  }

  return {
    title: `${page.title} | Polyverse Wiki`,
    description: page.summary ?? "Technical documentation wiki"
  };
}

export default async function Page({
  params
}: {
  params: Promise<{ slug?: string[] }>;
}) {
  const { slug = [] } = await params;
  const page = getDocBySlug(slug);

  if (!page) {
    notFound();
  }

  return (
    <WikiShell
      nodes={getTopLevelSections()}
      currentRoute={routeFromSlug(slug)}
      page={page}
      breadcrumbs={buildBreadcrumbs(slug, page.title)}
    />
  );
}
