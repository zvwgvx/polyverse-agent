"use client";

import Link from "next/link";
import { useEffect, useMemo, useState, type CSSProperties, type ReactElement } from "react";
import type { DocTreeNode } from "@/lib/docs";

type SidebarProps = {
  nodes: DocTreeNode[];
  currentRoute: string;
};

function isCurrentBranch(node: DocTreeNode, currentRoute: string): boolean {
  return node.route !== "/" && (currentRoute === node.route || currentRoute.startsWith(`${node.route}/`));
}

function collectBranchRoutes(nodes: DocTreeNode[]): Set<string> {
  const routes = new Set<string>();

  function visit(node: DocTreeNode): void {
    if (node.children.length > 0) {
      routes.add(node.route);
    }

    node.children.forEach(visit);
  }

  nodes.forEach(visit);
  return routes;
}

function collectForcedOpenRoutes(nodes: DocTreeNode[], currentRoute: string): Set<string> {
  const routes = new Set<string>();

  function visit(node: DocTreeNode): void {
    if (node.children.length > 0 && isCurrentBranch(node, currentRoute)) {
      routes.add(node.route);
    }

    node.children.forEach(visit);
  }

  nodes.forEach(visit);
  return routes;
}

function treePadding(depth: number, branch: boolean): CSSProperties {
  return {
    paddingLeft: `${depth * 12 + (branch ? 0 : 16)}px`
  };
}

function renderNode(
  node: DocTreeNode,
  currentRoute: string,
  depth: number,
  openRoutes: Set<string>,
  forcedOpenRoutes: Set<string>,
  onToggle: (route: string) => void
): ReactElement {
  const hasChildren = node.children.length > 0;
  const href = node.page?.route ?? node.route;

  if (!hasChildren) {
    return (
      <Link
        key={href}
        href={href}
        aria-current={currentRoute === href ? "page" : undefined}
        className={`wiki-tree-item${currentRoute === href ? " is-active" : ""}`}
        style={treePadding(depth, false)}
      >
        {node.page?.title ?? node.title}
      </Link>
    );
  }

  const isOpen = openRoutes.has(node.route);
  const isCurrent = forcedOpenRoutes.has(node.route);

  return (
    <div key={node.route} className="wiki-tree-branch">
      <button
        type="button"
        aria-expanded={isOpen}
        className={`wiki-tree-summary${isCurrent ? " is-current" : ""}${isOpen ? " is-open" : ""}`}
        style={treePadding(depth, true)}
        onClick={() => onToggle(node.route)}
      >
        <span>{node.title}</span>
      </button>

      {isOpen ? (
        <div className="wiki-tree-children">
          {node.page ? (
            <Link
              href={href}
              aria-current={currentRoute === href ? "page" : undefined}
              className={`wiki-tree-item wiki-tree-index${currentRoute === href ? " is-active" : ""}`}
              style={treePadding(depth + 1, false)}
            >
              Overview
            </Link>
          ) : null}

          {node.children.map((child) => renderNode(child, currentRoute, depth + 1, openRoutes, forcedOpenRoutes, onToggle))}
        </div>
      ) : null}
    </div>
  );
}

const globalOpenRoutes = new Set<string>();

export function Sidebar({ nodes, currentRoute }: SidebarProps) {
  const branchRoutes = useMemo(() => collectBranchRoutes(nodes), [nodes]);
  const forcedOpenRoutes = useMemo(() => collectForcedOpenRoutes(nodes, currentRoute), [nodes, currentRoute]);

  const [openRoutes, setOpenRoutes] = useState<Set<string>>(() => {
    const initial = new Set<string>();
    globalOpenRoutes.forEach((route) => initial.add(route));
    forcedOpenRoutes.forEach((route) => {
      initial.add(route);
      globalOpenRoutes.add(route);
    });
    return initial;
  });

  useEffect(() => {
    setOpenRoutes((previous) => {
      const next = new Set<string>();

      branchRoutes.forEach((route) => {
        if (previous.has(route) || forcedOpenRoutes.has(route)) {
          next.add(route);
          globalOpenRoutes.add(route);
        }
      });

      return next;
    });
  }, [branchRoutes, forcedOpenRoutes]);

  function toggleBranch(route: string): void {
    setOpenRoutes((previous) => {
      const next = new Set(previous);

      if (next.has(route)) {
        next.delete(route);
        globalOpenRoutes.delete(route);
      } else {
        next.add(route);
        globalOpenRoutes.add(route);
      }

      return next;
    });
  }

  return (
    <aside className="wiki-sidebar">
      <div className="wiki-sidebar-inner">
        <div className="wiki-brand">
          <h1>Polyverse Wiki</h1>
          <p>Repository-driven technical documentation.</p>
        </div>

        <nav className="wiki-tree" aria-label="Wiki navigation">
          {nodes.map((node) => renderNode(node, currentRoute, 0, openRoutes, forcedOpenRoutes, toggleBranch))}
        </nav>
      </div>
    </aside>
  );
}
