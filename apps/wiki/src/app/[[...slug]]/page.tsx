import { notFound } from "next/navigation";
import type { Metadata } from "next";
import { getDocBySlug, getStaticSlugs } from "@/lib/docs";
import { buildBreadcrumbs } from "@/lib/paths";
import { DocContent } from "@/components/doc-content";

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
    <DocContent
      title={page.title}
      summary={page.summary}
      content={page.content}
      breadcrumbs={buildBreadcrumbs(slug, page.title)}
    />
  );
}
