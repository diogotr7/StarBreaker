import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Download } from "lucide-react";
import {
  listDir,
  p4kSearch,
  extractP4kFolder,
  extractP4kFile,
  type DirEntry,
  type P4kSearchResult,
} from "../lib/commands";
import { VirtualizedSearchList } from "../components/virtualized-search-list";
import { buildTreeFromRows, flattenForVirtualization, type VisibleRow } from "../lib/search-tree";
import { ExtractProgress } from "../components/extract-progress";
import { useAppStore } from "../stores/app-store";
import { ResizeHandle } from "../components/resize-handle";
import { GeometryPreview } from "../components/geometry-preview";
import { XmlPreview } from "../components/xml-preview";
import { DdsPreview } from "../components/dds-preview";
import { ImagePreview } from "../components/image-preview";

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024)
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
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
  extractFilter,
  onExtractStart,
  onExtractEnd,
}: {
  node: TreeNode;
  depth: number;
  onToggle: (path: string) => void;
  selectedPath: string;
  onSelect: (path: string) => void;
  extractFilter: string;
  onExtractStart: () => void;
  onExtractEnd: () => void;
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

  const handleExtract = async (e: React.MouseEvent) => {
    e.stopPropagation();
    const { open } = await import("@tauri-apps/plugin-dialog");
    const dir = await open({ title: `Extract "${node.name}"`, directory: true, multiple: false });
    if (!dir) return;

    onExtractStart();
    try {
      await extractP4kFolder(node.path, dir, extractFilter || undefined);
    } catch (err) {
      console.error("P4k folder extract failed:", err);
    } finally {
      onExtractEnd();
    }
  };

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
        className={`
          w-full text-left px-2 py-1 text-sm flex items-center gap-1.5 cursor-pointer
          hover:bg-surface/50 transition-colors group
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

        {/* Extract button for folders */}
        {node.isDir && (
          <button
            type="button"
            onClick={handleExtract}
            title={`Extract ${node.name}/`}
            className="hidden group-hover:flex items-center justify-center w-5 h-5 rounded
                       text-text-dim hover:text-text hover:bg-surface-hi transition-colors
                       disabled:opacity-50"
          >
            <Download size={12} />
          </button>
        )}

        {/* File download + size */}
        {!node.isDir && (
          <>
            <button
              type="button"
              onClick={async (e) => {
                e.stopPropagation();
                const { save } = await import("@tauri-apps/plugin-dialog");
                const filename = node.name;
                const outputPath = await save({ title: `Save "${filename}"`, defaultPath: filename });
                if (!outputPath) return;
                try {
                  await extractP4kFile(node.path, outputPath);
                } catch (err) {
                  console.error("File extract failed:", err);
                }
              }}
              title={`Save ${node.name}`}
              className="hidden group-hover:flex items-center justify-center w-5 h-5 rounded
                         text-text-dim hover:text-text hover:bg-surface-hi transition-colors"
            >
              <Download size={12} />
            </button>
            {node.size != null && (
              <span className="text-xs text-text-dim shrink-0 tabular-nums">
                {formatSize(node.size)}
              </span>
            )}
          </>
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
            extractFilter={extractFilter}
            onExtractStart={onExtractStart}
            onExtractEnd={onExtractEnd}
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
  const [treeWidth, setTreeWidth] = useState(360);
  const [extracting, setExtracting] = useState(false);
  const [extractFilter, setExtractFilter] = useState("");
  const searchSeqRef = useRef(0);
  const [treeMode, setTreeMode] = useState(false);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());

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
      p4kSearch(query)
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
                ? `${searchResults.length.toLocaleString()} of ${searchTotal.toLocaleString()} (refine to see more)`
                : `${searchResults.length.toLocaleString()} results`}
          </span>
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
        <input
          type="text"
          placeholder="Extract filter (e.g. mtl,xml)"
          value={extractFilter}
          onChange={(e) => setExtractFilter(e.target.value)}
          title="Comma-separated file extensions to include when extracting folders. Leave empty for all files."
          className="w-48 bg-surface rounded-md px-3 py-1.5 text-sm text-text placeholder:text-text-faint outline-none focus:ring-1 focus:ring-ring shrink-0"
        />
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
            <VirtualizedSearchList<P4kSearchResult>
              items={searchResults}
              rowHeight={28}
              getKey={(item) => item.path}
              renderRow={(item) => (
                <button
                  type="button"
                  onClick={() => setSelectedPath(item.path)}
                  className={`w-full h-full text-left px-3 text-sm flex items-center gap-2 hover:bg-surface/50 transition-colors ${
                    selectedPath === item.path ? "bg-primary/15 text-text" : "text-text"
                  }`}
                >
                  <span className="flex-1 truncate font-mono text-xs">{item.path}</span>
                  <span className="text-xs text-text-dim shrink-0 tabular-nums">
                    {formatSize(item.uncompressed_size)}
                  </span>
                </button>
              )}
            />
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
                extractFilter={extractFilter}
                onExtractStart={() => setExtracting(true)}
                onExtractEnd={() => setExtracting(false)}
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
