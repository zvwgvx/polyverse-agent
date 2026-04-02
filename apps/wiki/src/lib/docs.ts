import fs from "node:fs";
import path from "node:path";
import matter from "gray-matter";

export type DocFrontmatter = {
  title?: string;
  summary?: string;
  order?: number;
};

export type DocPage = {
  slug: string[];
  route: string;
  title: string;
  summary?: string;
  order?: number;
  content: string;
  filePath: string;
};

export type DocTreeNode = {
  title: string;
  route: string;
  summary?: string;
  order?: number;
  children: DocTreeNode[];
  page?: Pick<DocPage, "title" | "route" | "summary" | "order">;
};

const DOCS_ROOT = path.join(process.cwd(), "..", "..", "docs", "wiki");
const README_BASENAME = "README.md";

function titleFromSegment(segment: string): string {
  return segment
    .replace(/[-_]+/g, " ")
    .replace(/\b\w/g, (match) => match.toUpperCase());
}

function normalizeSlashes(input: string): string {
  return input.split(path.sep).join("/");
}

function routeFromSlug(slug: string[]): string {
  return slug.length === 0 ? "/" : `/${slug.join("/")}`;
}

function parseDoc(filePath: string, slug: string[]): DocPage {
  const raw = fs.readFileSync(filePath, "utf8");
  const parsed = matter(raw);
  const data = (parsed.data ?? {}) as DocFrontmatter;
  const fallbackTitle = slug.length === 0 ? "Wiki" : titleFromSegment(slug[slug.length - 1]);

  return {
    slug,
    route: routeFromSlug(slug),
    title: data.title?.trim() || fallbackTitle,
    summary: data.summary?.trim() || undefined,
    order: typeof data.order === 'number' ? data.order : undefined,
    content: parsed.content.trim(),
    filePath
  };
}

function getSectionDirs(root: string): string[] {
  if (!fs.existsSync(root)) {
    return [];
  }

  return fs
    .readdirSync(root, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name)
    .sort((left, right) => left.localeCompare(right));
}

function getMarkdownFiles(dir: string): string[] {
  return fs
    .readdirSync(dir, { withFileTypes: true })
    .filter((entry) => entry.isFile() && entry.name.endsWith(".md") && entry.name !== README_BASENAME)
    .map((entry) => entry.name)
    .sort((left, right) => left.localeCompare(right));
}

function buildTreeForDir(root: string, relativeDir: string): DocTreeNode {
  const absoluteDir = path.join(root, relativeDir);
  const segments = relativeDir === "." ? [] : normalizeSlashes(relativeDir).split("/").filter(Boolean);
  const readmePath = path.join(absoluteDir, README_BASENAME);
  const page = fs.existsSync(readmePath) ? parseDoc(readmePath, segments) : undefined;

  const childDirs = fs
    .readdirSync(absoluteDir, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name)
    .sort((left, right) => left.localeCompare(right));

  const childPages = getMarkdownFiles(absoluteDir).map((fileName) => {
    const slug = [...segments, fileName.replace(/\.md$/i, "")];
    const page = parseDoc(path.join(absoluteDir, fileName), slug);
    return {
      title: page.title,
      route: page.route,
      summary: page.summary,
      order: page.order,
      children: []
    } satisfies DocTreeNode;
  });

  const childSections = childDirs.map((dirName) => buildTreeForDir(root, path.join(relativeDir, dirName)));
  const title = page?.title ?? (segments.length === 0 ? "Wiki" : titleFromSegment(segments[segments.length - 1]));

  const children = [...childSections, ...childPages].sort((a, b) => {
    const orderA = a.order ?? 999;
    const orderB = b.order ?? 999;
    if (orderA !== orderB) {
      return orderA - orderB;
    }
    return a.title.localeCompare(b.title);
  });

  return {
    title,
    route: routeFromSlug(segments),
    summary: page?.summary,
    order: page?.order,
    page: page ? { title: page.title, route: page.route, summary: page.summary, order: page.order } : undefined,
    children
  };
}

function collectPagesFromDir(root: string, relativeDir: string, acc: DocPage[]): void {
  const absoluteDir = path.join(root, relativeDir);
  const segments = relativeDir === "." ? [] : normalizeSlashes(relativeDir).split("/").filter(Boolean);
  const readmePath = path.join(absoluteDir, README_BASENAME);

  if (fs.existsSync(readmePath)) {
    acc.push(parseDoc(readmePath, segments));
  }

  for (const fileName of getMarkdownFiles(absoluteDir)) {
    const slug = [...segments, fileName.replace(/\.md$/i, "")];
    acc.push(parseDoc(path.join(absoluteDir, fileName), slug));
  }

  const childDirs = fs
    .readdirSync(absoluteDir, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name)
    .sort((left, right) => left.localeCompare(right));

  for (const dirName of childDirs) {
    collectPagesFromDir(root, path.join(relativeDir, dirName), acc);
  }
}

export function getDocsRoot(): string {
  return DOCS_ROOT;
}

export function getAllDocsPages(): DocPage[] {
  if (!fs.existsSync(DOCS_ROOT)) {
    return [];
  }

  const pages: DocPage[] = [];
  collectPagesFromDir(DOCS_ROOT, ".", pages);
  return pages.sort((left, right) => left.route.localeCompare(right.route));
}

export function getDocBySlug(slug: string[]): DocPage | null {
  const normalized = slug.filter(Boolean);
  const route = routeFromSlug(normalized);
  return getAllDocsPages().find((page) => page.route === route) ?? null;
}

export function getDocsTree(): DocTreeNode {
  if (!fs.existsSync(DOCS_ROOT)) {
    return { title: "Wiki", route: "/", summary: undefined, children: [] };
  }

  return buildTreeForDir(DOCS_ROOT, ".");
}

export function getTopLevelSections(): DocTreeNode[] {
  const root = getDocsTree();
  return root.children.filter((child) => child.children.length > 0 || child.page);
}

export function getStaticSlugs(): string[][] {
  return getAllDocsPages().map((page) => page.slug);
}

export function getSectionNames(): string[] {
  return getSectionDirs(DOCS_ROOT);
}
