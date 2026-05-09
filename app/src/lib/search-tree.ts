export interface TreeFolder<T = unknown> {
  kind: "folder";
  name: string;
  path: string;
  children: TreeNode<T>[];
}

export interface TreeLeaf<T = unknown> {
  kind: "leaf";
  name: string;
  data: T;
}

export type TreeNode<T = unknown> = TreeFolder<T> | TreeLeaf<T>;

export interface VisibleRow<T> {
  kind: "folder" | "leaf";
  depth: number;
  /** folder path or leaf key — used for keys and collapse-state lookup */
  key: string;
  name: string;
  /** present on folder rows */
  collapsed?: boolean;
  /** present on leaf rows */
  data?: T;
}

/**
 * Build a tree from rows. `getSegments` returns the path components of a row;
 * the last segment is the leaf name, prior segments are folder names.
 */
export function buildTreeFromRows<T>(
  rows: T[],
  getSegments: (row: T) => string[],
): TreeNode<T>[] {
  const root: TreeNode<T>[] = [];

  for (const row of rows) {
    const segments = getSegments(row);
    if (segments.length === 0) continue;

    const leafName = segments[segments.length - 1];
    const folderSegments = segments.slice(0, -1);

    let siblings = root;
    let currentPath = "";

    for (const segment of folderSegments) {
      currentPath = currentPath ? `${currentPath}/${segment}` : segment;
      let folder = siblings.find(
        (n): n is TreeFolder<T> => n.kind === "folder" && n.path === currentPath,
      );
      if (!folder) {
        folder = { kind: "folder", name: segment, path: currentPath, children: [] };
        siblings.push(folder);
      }
      siblings = folder.children;
    }

    siblings.push({ kind: "leaf", name: leafName, data: row });
  }

  const sortNodes = (nodes: TreeNode<T>[]) => {
    nodes.sort((a, b) => {
      if (a.kind !== b.kind) return a.kind === "folder" ? -1 : 1;
      return a.name.localeCompare(b.name);
    });
    for (const node of nodes) {
      if (node.kind === "folder") sortNodes(node.children);
    }
  };
  sortNodes(root);
  return root;
}

/**
 * Walk the tree in display order, honoring `collapsed` (set of folder paths
 * the user has collapsed). Default state is expanded.
 */
export function flattenForVirtualization<T>(
  tree: TreeNode<T>[],
  collapsed: ReadonlySet<string>,
): VisibleRow<T>[] {
  const out: VisibleRow<T>[] = [];

  const walk = (nodes: TreeNode<T>[], depth: number) => {
    for (const node of nodes) {
      if (node.kind === "folder") {
        const isCollapsed = collapsed.has(node.path);
        out.push({
          kind: "folder",
          depth,
          key: `f:${node.path}`,
          name: node.name,
          collapsed: isCollapsed,
        });
        if (!isCollapsed) walk(node.children, depth + 1);
      } else {
        out.push({
          kind: "leaf",
          depth,
          key: `l:${depth}:${node.name}:${out.length}`,
          name: node.name,
          data: node.data,
        });
      }
    }
  };

  walk(tree, 0);
  return out;
}
