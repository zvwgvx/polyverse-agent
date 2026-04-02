export type BreadcrumbItem = {
  label: string;
  href: string;
};

function titleFromSegment(segment: string): string {
  return segment
    .replace(/[-_]+/g, " ")
    .replace(/\b\w/g, (match) => match.toUpperCase());
}

export function routeFromSlug(slug: string[]): string {
  return slug.length === 0 ? "/" : `/${slug.join("/")}`;
}

export function buildBreadcrumbs(slug: string[], currentTitle: string): BreadcrumbItem[] {
  const breadcrumbs: BreadcrumbItem[] = [{ label: "Wiki", href: "/" }];

  slug.forEach((segment, index) => {
    const href = routeFromSlug(slug.slice(0, index + 1));
    const label = index === slug.length - 1 ? currentTitle : titleFromSegment(segment);
    breadcrumbs.push({ label, href });
  });

  return breadcrumbs;
}
