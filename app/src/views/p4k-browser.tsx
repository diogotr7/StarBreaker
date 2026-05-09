import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  listDir,
  p4kSearch,
  extractP4kFolder,
  extractP4kFile,
  extractP4kPaths,
  type DirEntry,
  type P4kSearchResult,
} from "../lib/commands";
import { VirtualizedSearchList } from "../components/virtualized-search-list";
import { buildTreeFromRows, flattenForVirtualization, type VisibleRow } from "../lib/search-tree";
import { ContextMenu, useContextMenu, type ContextMenuItem } from "../components/context-menu";
import { ExtractProgress } from "../components/extract-progress";
import { useAppStore } from "../stores/app-store";
import { ResizeHandle } from "../components/resize-handle";
import { GeometryPreview } from "../components/geometry-preview";
import { XmlPreview } from "../components/xml-preview";
import { DdsPreview } from "../components/dds-preview";
import { ImagePreview } from "../components/image-preview";

/** Default cap on results materialized per keystroke. The "Load all" button
 *  in the toolbar fires an uncapped fetch on demand. */
const DEFAULT_SEARCH_LIMIT = 5_000;

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024)
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

function formatModified(unixSeconds: number): string {
  if (!unixSeconds) return "—";
  const d = new Date(unixSeconds * 1000);
  const pad = (n: number) => n.toString().padStart(2, "0");
  return `${d.getUTCFullYear()}-${pad(d.getUTCMonth() + 1)}-${pad(d.getUTCDate())} ${pad(d.getUTCHours())}:${pad(d.getUTCMinutes())}`;
}

type SortColumn = "name" | "size" | "modified";
type SortDirection = "asc" | "desc";
interface SortState {
  column: SortColumn;
  direction: SortDirection;
}
const DEFAULT_DIRECTION: Record<SortColumn, SortDirection> = {
  name: "asc",
  size: "desc",
  modified: "desc",
};

function SortHeaderButton({
  label,
  column,
  sort,
  onSort,
  className = "",
}: {
  label: string;
  column: SortColumn;
  sort: SortState;
  onSort: (col: SortColumn) => void;
  className?: string;
}) {
  const active = sort.column === column;
  const arrow = active ? (sort.direction === "asc" ? "▲" : "▼") : "";
  return (
    <button
      type="button"
      onClick={() => onSort(column)}
      className={`text-left text-[11px] uppercase tracking-wide hover:text-text transition-colors ${
        active ? "text-text" : "text-text-dim"
      } ${className}`}
    >
      {label}
      {arrow && <span className="ml-1 text-[9px]">{arrow}</span>}
    </button>
  );
}

function SearchColumnHeader({
  sort,
  onSort,
}: {
  sort: SortState;
  onSort: (col: SortColumn) => void;
}) {
  return (
    <div className="flex items-center border-b border-border bg-bg-alt h-7 shrink-0">
      <SortHeaderButton label="Name" column="name" sort={sort} onSort={onSort} className="flex-1 px-3" />
      <SortHeaderButton label="Size" column="size" sort={sort} onSort={onSort} className="w-20 text-right" />
      <SortHeaderButton label="Modified" column="modified" sort={sort} onSort={onSort} className="w-32 text-right pr-3" />
    </div>
  );
}

const GEOMETRY_EXTENSIONS = [".skin", ".skinm", ".cgf", ".cgfm", ".cga"];

function isGeometryFile(path: string): boolean {
  const lower = path.toLowerCase();
  return GEOMETRY_EXTENSIONS.some((ext) => lower.endsWith(ext));
}

const XML_EXTENSIONS = [".xml", ".mtl", ".chrparams", ".cdf", ".adb", ".comb"];

function isXmlFile(path: string): boolean {
  const lower = path.toLowerCase();
  return XML_EXTENSIONS.some((ext) => lower.endsWith(ext));
}

function isDdsFile(path: string): boolean {
  return path.toLowerCase().endsWith(".dds");
}

const IMAGE_EXTENSIONS = [".png", ".jpg", ".jpeg", ".gif", ".bmp"];

function isImageFile(path: string): boolean {
  const lower = path.toLowerCase();
  return IMAGE_EXTENSIONS.some((ext) => lower.endsWith(ext));
}

interface TreeNode {
  name: string;
  path: string;
  isDir: boolean;
  size?: number;
  children?: TreeNode[];
  loaded: boolean;
  expanded: boolean;
  loading: boolean;
}

function TreeItem({
  node,
  depth,
  onToggle,
  selectedPath,
  onSelect,
  onContextMenu,
}: {
  node: TreeNode;
  depth: number;
  onToggle: (path: string) => void;
  selectedPath: string;
  onSelect: (path: string) => void;
  onContextMenu: (e: React.MouseEvent, node: TreeNode) => void;
}) {
  const isSelected = selectedPath === node.path;
  const [showSpinner, setShowSpinner] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout>>(undefined);

  useEffect(() => {
    if (node.loading) {
      timerRef.current = setTimeout(() => setShowSpinner(true), 200);
    } else {
      clearTimeout(timerRef.current);
      setShowSpinner(false);
    }
    return () => clearTimeout(timerRef.current);
  }, [node.loading]);

  return (
    <div>
      <div
        role="button"
        tabIndex={0}
        onClick={() => {
          if (node.isDir) onToggle(node.path);
          onSelect(node.path);
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            if (node.isDir) onToggle(node.path);
            onSelect(node.path);
          }
        }}
        onContextMenu={(e) => onContextMenu(e, node)}
        className={`
          w-full text-left px-2 py-1 text-sm flex items-center gap-1.5 cursor-pointer
          hover:bg-surface/50 transition-colors
          ${isSelected ? "bg-primary/15 text-text" : "text-text"}
        `}
        style={{ paddingLeft: `${depth * 16 + 8}px` }}
      >
        {/* Chevron / spinner / spacer */}
        <span className="w-4 shrink-0 flex items-center justify-center">
          {node.isDir ? (
            showSpinner ? (
              <svg
                className="animate-spin w-3.5 h-3.5 text-text-faint"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2.5"
              >
                <path d="M12 2a10 10 0 0 1 10 10" strokeLinecap="round" />
              </svg>
            ) : (
              <svg
                className={`w-3.5 h-3.5 text-text-faint transition-transform duration-150 ${node.expanded ? "rotate-90" : ""}`}
                viewBox="0 0 24 24"
                fill="currentColor"
              >
                <path d="M9 6l8 6-8 6V6z" />
              </svg>
            )
          ) : null}
        </span>

        <span className="flex-1 truncate">{node.name}</span>

        {!node.isDir && node.size != null && (
          <span className="text-xs text-text-dim shrink-0 tabular-nums">
            {formatSize(node.size)}
          </span>
        )}
      </div>

      {node.isDir &&
        node.expanded &&
        node.children?.map((child) => (
          <TreeItem
            key={child.path}
            node={child}
            depth={depth + 1}
            onToggle={onToggle}
            selectedPath={selectedPath}
            onSelect={onSelect}
            onContextMenu={onContextMenu}
          />
        ))}
    </div>
  );
}

function entriesToNodes(parentPath: string, entries: DirEntry[]): TreeNode[] {
  const dirs: TreeNode[] = [];
  const files: TreeNode[] = [];

  for (const e of entries) {
    const path = parentPath ? `${parentPath}\\${e.name}` : e.name;
    if (e.kind === "directory") {
      dirs.push({
        name: e.name,
        path,
        isDir: true,
        loaded: false,
        expanded: false,
        loading: false,
      });
    } else {
      files.push({
        name: e.name,
        path,
        isDir: false,
        size: e.uncompressed_size,
        loaded: true,
        expanded: false,
        loading: false,
      });
    }
  }

  // Directories first, then files
  return [...dirs, ...files];
}


export function P4kBrowser() {
  const hasData = useAppStore((s) => s.hasData);
  const [tree, setTree] = useState<TreeNode[]>([]);
  const [selectedPath, setSelectedPath] = useState("");
  const [searchQuery, setSearchQuery] = useState("");
  const [searchResults, setSearchResults] = useState<P4kSearchResult[]>([]);
  const [searchTotal, setSearchTotal] = useState(0);
  const [searching, setSearching] = useState(false);
  const [loadingAll, setLoadingAll] = useState(false);
  const [sort, setSort] = useState<SortState>({ column: "name", direction: "asc" });
  const [treeWidth, setTreeWidth] = useState(360);
  const [extracting, setExtracting] = useState(false);
  const searchSeqRef = useRef(0);
  const [treeMode, setTreeMode] = useState(false);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const ctxMenu = useContextMenu();

  // ── Action helpers (used by context menus + toolbar buttons) ─────────────

  const saveFile = useCallback(async (path: string) => {
    const { save } = await import("@tauri-apps/plugin-dialog");
    const filename = path.split("\\").pop() ?? "file";
    const outputPath = await save({ title: `Save "${filename}"`, defaultPath: filename });
    if (!outputPath) return;
    try {
      await extractP4kFile(path, outputPath);
    } catch (err) {
      console.error("File extract failed:", err);
    }
  }, []);

  const extractFolder = useCallback(async (folderPath: string, folderName: string) => {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const dir = await open({
      title: `Extract "${folderName}"`,
      directory: true,
      multiple: false,
    });
    if (!dir) return;
    setExtracting(true);
    try {
      await extractP4kFolder(folderPath, dir);
    } catch (err) {
      console.error("P4k folder extract failed:", err);
    } finally {
      setExtracting(false);
    }
  }, []);

  const extractAllMatches = useCallback(async (paths: string[]) => {
    if (paths.length === 0) return;
    const { open } = await import("@tauri-apps/plugin-dialog");
    const dir = await open({
      title: `Extract ${paths.length.toLocaleString()} files`,
      directory: true,
      multiple: false,
    });
    if (!dir) return;
    setExtracting(true);
    try {
      await extractP4kPaths(paths, dir);
    } catch (err) {
      console.error("P4k batch extract failed:", err);
    } finally {
      setExtracting(false);
    }
  }, []);

  const copyPath = useCallback(async (path: string) => {
    try {
      await navigator.clipboard.writeText(path);
    } catch (err) {
      console.error("Clipboard write failed:", err);
    }
  }, []);

  const buildTreeMenu = useCallback(
    (node: TreeNode): ContextMenuItem[] =>
      node.isDir
        ? [
            { label: "Extract folder…", onClick: () => extractFolder(node.path, node.name) },
            { label: "Copy path", onClick: () => copyPath(node.path) },
          ]
        : [
            { label: "Save as…", onClick: () => saveFile(node.path) },
            { label: "Copy path", onClick: () => copyPath(node.path) },
          ],
    [extractFolder, saveFile, copyPath],
  );

  const buildSearchFileMenu = useCallback(
    (path: string): ContextMenuItem[] => [
      { label: "Save as…", onClick: () => saveFile(path) },
      { label: "Copy path", onClick: () => copyPath(path) },
    ],
    [saveFile, copyPath],
  );

  // Load root entries on mount
  useEffect(() => {
    if (!hasData) return;
    listDir("").then((entries) => {
      setTree(entriesToNodes("", entries));
    });
  }, [hasData]);

  useEffect(() => {
    const query = searchQuery.trim();
    searchSeqRef.current += 1;
    setCollapsed(new Set());
    const seq = searchSeqRef.current;

    if (!hasData || query.length === 0) {
      setSearchResults([]);
      setSearchTotal(0);
      setSearching(false);
      return;
    }

    setSearching(true);
    const timeout = setTimeout(() => {
      p4kSearch(query, DEFAULT_SEARCH_LIMIT)
        .then((response) => {
          if (searchSeqRef.current === seq) {
            setSearchResults(response.results);
            setSearchTotal(response.total);
            setSearching(false);
          }
        })
        .catch((err) => {
          if (searchSeqRef.current === seq) {
            console.error("P4k search failed:", err);
            setSearchResults([]);
            setSearchTotal(0);
            setSearching(false);
          }
        });
    }, 150);

    return () => clearTimeout(timeout);
  }, [hasData, searchQuery]);

  const hasSearch = searchQuery.trim().length > 0;

  const sortedResults = useMemo(() => {
    if (treeMode) return searchResults;
    const sign = sort.direction === "asc" ? 1 : -1;
    const arr = [...searchResults];
    arr.sort((a, b) => {
      switch (sort.column) {
        case "name":
          return sign * a.path.localeCompare(b.path);
        case "size":
          return sign * (a.uncompressed_size - b.uncompressed_size);
        case "modified":
          return sign * (a.modified_unix - b.modified_unix);
      }
    });
    return arr;
  }, [searchResults, sort, treeMode]);

  const handleSortClick = useCallback((column: SortColumn) => {
    setSort((prev) =>
      prev.column === column
        ? { column, direction: prev.direction === "asc" ? "desc" : "asc" }
        : { column, direction: DEFAULT_DIRECTION[column] },
    );
  }, []);

  const visibleRows = useMemo(() => {
    if (!hasSearch || !treeMode) return null;
    const tree = buildTreeFromRows(searchResults, (r) => r.path.split("\\").filter(Boolean));
    return flattenForVirtualization(tree, collapsed);
  }, [hasSearch, treeMode, searchResults, collapsed]);

  const handleToggle = useCallback(
    async (path: string) => {
      const markLoading = (nodes: TreeNode[]): TreeNode[] =>
        nodes.map((node) => {
          if (node.path === path) {
            if (node.loaded) return { ...node, expanded: !node.expanded };
            return { ...node, loading: true };
          }
          if (node.children) {
            return { ...node, children: markLoading(node.children) };
          }
          return node;
        });

      const marked = markLoading(tree);
      setTree(marked);

      const findNode = (nodes: TreeNode[]): TreeNode | null => {
        for (const n of nodes) {
          if (n.path === path) return n;
          if (n.children) {
            const found = findNode(n.children);
            if (found) return found;
          }
        }
        return null;
      };

      const target = findNode(marked);
      if (!target || target.loaded) return;

      // Load all children (dirs + files)
      const entries = await listDir(path);
      const children = entriesToNodes(path, entries);

      const finishLoad = (nodes: TreeNode[]): TreeNode[] =>
        nodes.map((node) => {
          if (node.path === path) {
            return {
              ...node,
              loaded: true,
              expanded: true,
              loading: false,
              children,
            };
          }
          if (node.children) {
            return { ...node, children: finishLoad(node.children) };
          }
          return node;
        });

      setTree((prev) => finishLoad(prev));
    },
    [tree],
  );

  if (!hasData) {
    return (
      <div className="flex-1 flex items-center justify-center text-text-dim">
        Load a P4k to browse files
      </div>
    );
  }

  return (
    <div className="flex-1 flex flex-col overflow-hidden relative">
      <ContextMenu state={ctxMenu.state} onClose={ctxMenu.close} />
      <ExtractProgress active={extracting} onDone={() => setExtracting(false)} />
      {/* Toolbar */}
      <div className="px-3 flex items-center gap-2 border-b border-border bg-bg-alt shrink-0" style={{ height: "var(--toolbar-height)" }}>
        <input
          type="text"
          placeholder="Search files..."
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
          className="flex-1 bg-surface rounded-md px-3 py-1.5 text-sm text-text placeholder:text-text-faint outline-none focus:ring-1 focus:ring-ring"
        />
        {hasSearch && (
          <span className="text-xs text-text-dim shrink-0">
            {searching
              ? "Searching..."
              : searchTotal > searchResults.length
                ? `${searchResults.length.toLocaleString()} of ${searchTotal.toLocaleString()}`
                : `${searchResults.length.toLocaleString()} results`}
          </span>
        )}
        {hasSearch && !searching && searchTotal > searchResults.length && (
          <button
            type="button"
            disabled={loadingAll}
            onClick={() => {
              const query = searchQuery.trim();
              if (!query) return;
              const seq = ++searchSeqRef.current;
              setLoadingAll(true);
              p4kSearch(query, undefined)
                .then((response) => {
                  if (searchSeqRef.current === seq) {
                    setSearchResults(response.results);
                    setSearchTotal(response.total);
                  }
                })
                .catch((err) => {
                  console.error("P4k search (load all) failed:", err);
                })
                .finally(() => {
                  if (searchSeqRef.current === seq) setLoadingAll(false);
                });
            }}
            className="px-2 py-1 text-xs rounded bg-surface text-text-dim hover:text-text hover:bg-surface-hi shrink-0 disabled:opacity-50"
          >
            {loadingAll ? "Loading…" : `Load all ${searchTotal.toLocaleString()}`}
          </button>
        )}
        {hasSearch && (
          <button
            type="button"
            onClick={() => setTreeMode((v) => !v)}
            title={treeMode ? "Switch to flat list" : "Switch to tree view"}
            className="px-2 py-1 text-xs rounded bg-surface text-text-dim hover:text-text hover:bg-surface-hi shrink-0"
          >
            {treeMode ? "Flat" : "Tree"}
          </button>
        )}
        {hasSearch && !searching && searchResults.length > 0 && (
          <button
            type="button"
            onClick={() => extractAllMatches(searchResults.map((r) => r.path))}
            title={`Extract the ${searchResults.length.toLocaleString()} currently shown files. Use "Load all" first if you want every match.`}
            className="px-2 py-1 text-xs rounded bg-surface text-text-dim hover:text-text hover:bg-surface-hi shrink-0"
          >
            Extract all matches…
          </button>
        )}
      </div>

      <div className="flex-1 flex overflow-hidden">
      {/* Tree panel */}
      <div className="border-r border-border shrink-0 flex flex-col min-h-0" style={{ width: treeWidth }}>
        {hasSearch ? (
          treeMode && visibleRows ? (
            <VirtualizedSearchList<VisibleRow<P4kSearchResult>>
              items={visibleRows}
              rowHeight={24}
              getKey={(row) => row.key}
              renderRow={(row) => {
                if (row.kind === "folder") {
                  return (
                    <button
                      type="button"
                      onClick={() =>
                        setCollapsed((prev) => {
                          const next = new Set(prev);
                          const path = row.key.slice(2);
                          if (next.has(path)) next.delete(path);
                          else next.add(path);
                          return next;
                        })
                      }
                      className="w-full h-full text-left flex items-center text-sm text-text-dim hover:bg-surface/50 transition-colors"
                      style={{ paddingLeft: row.depth * 16 + 8 }}
                    >
                      <span className="w-4">{row.collapsed ? "▶" : "▼"}</span>
                      <span className="flex-1 truncate">{row.name}</span>
                    </button>
                  );
                }
                return (
                  <button
                    type="button"
                    onClick={() => setSelectedPath(row.data!.path)}
                    onContextMenu={(e) =>
                      ctxMenu.open(e, buildSearchFileMenu(row.data!.path))
                    }
                    className={`w-full h-full text-left flex items-center text-sm hover:bg-surface/50 transition-colors ${
                      selectedPath === row.data!.path ? "bg-primary/15 text-text" : "text-text"
                    }`}
                    style={{ paddingLeft: row.depth * 16 + 24 }}
                  >
                    <span className="flex-1 truncate font-mono text-xs">{row.name}</span>
                    <span className="text-xs text-text-dim shrink-0 tabular-nums pr-2">
                      {formatSize(row.data!.uncompressed_size)}
                    </span>
                  </button>
                );
              }}
            />
          ) : (
            <>
              <SearchColumnHeader sort={sort} onSort={handleSortClick} />
              <VirtualizedSearchList<P4kSearchResult>
                items={sortedResults}
                rowHeight={28}
                getKey={(item) => item.path}
                renderRow={(item) => (
                  <button
                    type="button"
                    onClick={() => setSelectedPath(item.path)}
                    onContextMenu={(e) => ctxMenu.open(e, buildSearchFileMenu(item.path))}
                    className={`w-full h-full text-left text-sm flex items-center hover:bg-surface/50 transition-colors ${
                      selectedPath === item.path ? "bg-primary/15 text-text" : "text-text"
                    }`}
                  >
                    <span className="flex-1 truncate font-mono text-xs px-3">{item.path}</span>
                    <span className="w-20 text-right text-xs text-text-dim shrink-0 tabular-nums">
                      {formatSize(item.uncompressed_size)}
                    </span>
                    <span className="w-32 text-right text-xs text-text-dim shrink-0 tabular-nums pr-3">
                      {formatModified(item.modified_unix)}
                    </span>
                  </button>
                )}
              />
            </>
          )
        ) : (
          <div className="py-1 flex-1 overflow-y-auto">
            {tree.map((node) => (
              <TreeItem
                key={node.path}
                node={node}
                depth={0}
                onToggle={handleToggle}
                selectedPath={selectedPath}
                onSelect={setSelectedPath}
                onContextMenu={(e, n) => ctxMenu.open(e, buildTreeMenu(n))}
              />
            ))}
          </div>
        )}
      </div>
      <ResizeHandle width={treeWidth} onResize={setTreeWidth} side="right" min={200} max={600} />

      {/* Preview panel */}
      <div className="flex-1 flex items-center justify-center text-text-dim overflow-hidden">
        {selectedPath && isGeometryFile(selectedPath) ? (
          <GeometryPreview path={selectedPath} />
        ) : selectedPath && isXmlFile(selectedPath) ? (
          <XmlPreview path={selectedPath} />
        ) : selectedPath && isDdsFile(selectedPath) ? (
          <DdsPreview path={selectedPath} />
        ) : selectedPath && isImageFile(selectedPath) ? (
          <ImagePreview path={selectedPath} />
        ) : selectedPath ? (
          <div className="text-center">
            <p className="text-sm font-mono break-all px-8">{selectedPath}</p>
          </div>
        ) : (
          <p className="text-sm">Select a file to preview</p>
        )}
      </div>
      </div>
    </div>
  );
}
