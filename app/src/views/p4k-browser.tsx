import { useCallback, useEffect, useRef, useState } from "react";
import { Download } from "lucide-react";
import { listDir, extractP4kFolder, extractP4kFile, type DirEntry } from "../lib/commands";
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
      <button
        onClick={() => {
          if (node.isDir) onToggle(node.path);
          onSelect(node.path);
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
      </button>

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
  const [treeWidth, setTreeWidth] = useState(360);

  // Load root entries on mount
  useEffect(() => {
    if (!hasData) return;
    listDir("").then((entries) => {
      setTree(entriesToNodes("", entries));
    });
  }, [hasData]);

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

  const [extracting, setExtracting] = useState(false);
  const [extractFilter, setExtractFilter] = useState("");

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
      <div className="border-r border-border overflow-y-auto shrink-0" style={{ width: treeWidth }}>
        <div className="py-1">
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
