// Lazy directory-tree view of the p4k's socpak filesystem.
//
// Replaces the eager catalog list in the Maps tab. The user starts
// with one prefix (e.g. `Data\ObjectContainers\`); we render the
// immediate children, and each directory expands on click via a
// fresh `listSocpakDir` call. Loaded children are cached per branch
// so collapsing and re-expanding is free.
//
// Why this exists: the eager `enumerateScenes` call walked every
// `.socpak` under `Data\ObjectContainers\`, parsed each one's child
// XML, and built a graph -- 1700+ socpaks worth of zip-within-zip
// reads on every cold call. That locked up the Tauri webview before
// returning. The lazy tree pays only for what the user expands.

import { useCallback, useEffect, useState, type JSX } from "react";
import { ChevronRight, Folder, Box, Loader2, AlertCircle } from "lucide-react";
import { listSocpakDir, type SocpakDirEntry } from "../lib/commands";

interface SocpakTreeProps {
  /** Initial top-level prefix; e.g. `Data\ObjectContainers\`. */
  rootPrefix: string;
  /** Called when the user clicks a `.socpak` leaf. */
  onLeafClick: (entry: SocpakDirEntry) => void;
  /** When set, the leaf at this path renders with an active-branch highlight. */
  activePath?: string | null;
  /**
   * Flat list of every `.socpak` path in the p4k. Drives the
   * "search everywhere" mode: when `search` is non-empty AND this is
   * non-null, the tree hides its lazy-branch view and shows a flat
   * filtered list of matches against the index. When this is null
   * (still loading or unavailable), search input falls back to the
   * legacy "filter visible branches" behaviour and a "Indexing..."
   * hint replaces the empty-results message.
   */
  globalIndex?: string[] | null;
}

interface BranchState {
  status: "loading" | "loaded" | "error";
  /** Cached children once loaded; preserved across collapse/expand. */
  children?: SocpakDirEntry[];
  /** True when the branch is currently expanded in the tree. */
  expanded: boolean;
  /** Free-text error message when `status === "error"`. */
  error?: string;
}

type TreeState = Map<string, BranchState>;

/**
 * Format a byte count as a short human string. Mirrors the helper in
 * scene-view.tsx; redeclared here because the file is small enough
 * that pulling it into a shared module would be more code than it
 * saves.
 */
function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  let value = bytes;
  let idx = 0;
  while (value >= 1000 && idx < units.length - 1) {
    value /= 1000;
    idx += 1;
  }
  const digits = idx === 0 || value >= 100 ? 0 : value >= 10 ? 1 : 2;
  return `${value.toFixed(digits)} ${units[idx]}`;
}

/**
 * Apply a search filter to a list of children. The filter is
 * substring-on-display-name, case-insensitive. Returns the input
 * unchanged when the query is empty so the caller does not have to
 * re-allocate.
 *
 * Exported (named) for unit tests.
 */
export function filterTreeChildren(
  children: SocpakDirEntry[],
  query: string,
): SocpakDirEntry[] {
  const q = query.trim().toLowerCase();
  if (q.length === 0) return children;
  return children.filter((c) => c.display_name.toLowerCase().includes(q));
}

/**
 * Cap on results returned by a global-index search. The full archive
 * carries ~1700 socpaks and a 1- or 2-letter query can match most of
 * them; rendering them all is wasteful and would defeat the purpose of
 * a browser. 200 is enough that any plausible query has its real hits
 * inside the cap; the user can refine if they hit the cap.
 */
export const GLOBAL_INDEX_RESULT_CAP = 200;

/** One row in the flat search-results list. */
export interface GlobalIndexHit {
  /** Full p4k path (matches the path the lazy tree leaf would carry). */
  path: string;
  /** Trailing path segment, suitable for the row's main label. */
  display_name: string;
  /**
   * Parent directory in display form, e.g.
   * `PU/loc/mod/pyro/station/...`. Backslashes normalised to forward
   * slashes; the leading `Data\` prefix is preserved so the user can
   * tell at a glance where the file lives.
   */
  parent_display: string;
}

/**
 * Apply a substring search to a flat list of socpak paths. Returns at
 * most {@link GLOBAL_INDEX_RESULT_CAP} hits. Empty query returns an
 * empty list (the caller renders the tree in that case).
 *
 * Search target is the full path: a query like `pyro` matches any
 * socpak under a `pyro` directory, even if the filename does not
 * mention pyro. This is the behaviour the user actually wants -- the
 * lazy tree's display_name filter only ever matched leaf filenames.
 *
 * Exported (named) for unit tests.
 */
export function filterGlobalIndex(
  paths: string[],
  query: string,
  cap: number = GLOBAL_INDEX_RESULT_CAP,
): GlobalIndexHit[] {
  const q = query.trim().toLowerCase();
  if (q.length === 0) return [];
  const out: GlobalIndexHit[] = [];
  for (const path of paths) {
    if (!path.toLowerCase().includes(q)) continue;
    out.push(toGlobalIndexHit(path));
    if (out.length >= cap) break;
  }
  return out;
}

/**
 * Split a full p4k path into a `display_name` (last segment) and a
 * `parent_display` (everything before that, with backslashes turned
 * into forward slashes for legibility). Exported (named) for unit
 * tests.
 */
export function toGlobalIndexHit(path: string): GlobalIndexHit {
  // Find the last separator (either flavour) so we are correct on
  // either path style.
  let cut = -1;
  for (let i = path.length - 1; i >= 0; i -= 1) {
    const ch = path[i];
    if (ch === "\\" || ch === "/") {
      cut = i;
      break;
    }
  }
  if (cut < 0) {
    return { path, display_name: path, parent_display: "" };
  }
  const display = path.slice(cut + 1);
  const parent = path.slice(0, cut).replace(/\\/g, "/");
  return { path, display_name: display, parent_display: parent };
}

/**
 * Pure helper: return a new TreeState with the branch at `path`
 * patched to `next`. Map mutation in React state is a footgun
 * (reference equality fails to trigger re-render); this builds a
 * fresh map each call.
 *
 * Exported (named) for unit tests.
 */
export function setBranchState(
  state: TreeState,
  path: string,
  next: BranchState,
): TreeState {
  const out = new Map(state);
  out.set(path, next);
  return out;
}

interface BranchProps {
  entry: SocpakDirEntry;
  depth: number;
  state: TreeState;
  setState: (updater: (prev: TreeState) => TreeState) => void;
  onLeafClick: (entry: SocpakDirEntry) => void;
  activePath?: string | null;
}

/**
 * One row in the tree -- either a directory (with chevron + child
 * subtree) or a socpak leaf. Recursive: a directory branch renders
 * one `Branch` per child once expanded.
 */
function Branch({
  entry,
  depth,
  state,
  setState,
  onLeafClick,
  activePath,
}: BranchProps): JSX.Element {
  const branch = state.get(entry.path);

  const indentStyle = { paddingLeft: `${depth * 12 + 8}px` };

  // Socpak leaf: clickable, optionally highlighted as the active load.
  if (entry.kind === "socpak_file") {
    const isActive = activePath === entry.path;
    return (
      <div
        onClick={() => onLeafClick(entry)}
        style={indentStyle}
        className={`
          group flex items-center gap-1.5 pr-3 py-1 text-xs cursor-pointer
          transition-colors border-l-2
          ${
            isActive
              ? "bg-primary/10 text-text border-l-accent"
              : "text-text-sub hover:bg-surface/40 border-l-transparent"
          }
        `}
        title={entry.path}
      >
        <Box size={12} strokeWidth={1.75} className="text-text-faint shrink-0" />
        <span className="flex-1 min-w-0 truncate">{entry.display_name}</span>
        <span className="shrink-0 text-[10px] text-text-faint tabular-nums">
          {formatBytes(entry.size_or_count)}
        </span>
      </div>
    );
  }

  // Directory branch.
  const expanded = branch?.expanded ?? false;
  const status = branch?.status;

  const toggle = useCallback(async () => {
    const current = state.get(entry.path);
    if (current?.expanded) {
      // Collapse -- preserve children for instant re-expand.
      setState((prev) => setBranchState(prev, entry.path, {
        ...(prev.get(entry.path) ?? { status: "loaded", expanded: true }),
        expanded: false,
      }));
      return;
    }
    if (current?.status === "loaded" && current.children !== undefined) {
      // Already loaded -- just flip the expanded flag.
      setState((prev) => setBranchState(prev, entry.path, {
        ...current,
        expanded: true,
      }));
      return;
    }
    // Cold expand: fetch children.
    setState((prev) => setBranchState(prev, entry.path, {
      status: "loading",
      expanded: true,
    }));
    try {
      const children = await listSocpakDir(entry.path);
      setState((prev) => setBranchState(prev, entry.path, {
        status: "loaded",
        expanded: true,
        children,
      }));
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setState((prev) => setBranchState(prev, entry.path, {
        status: "error",
        expanded: true,
        error: msg,
      }));
    }
  }, [entry.path, state, setState]);

  const retry = useCallback(async () => {
    setState((prev) => setBranchState(prev, entry.path, {
      status: "loading",
      expanded: true,
    }));
    try {
      const children = await listSocpakDir(entry.path);
      setState((prev) => setBranchState(prev, entry.path, {
        status: "loaded",
        expanded: true,
        children,
      }));
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setState((prev) => setBranchState(prev, entry.path, {
        status: "error",
        expanded: true,
        error: msg,
      }));
    }
  }, [entry.path, setState]);

  return (
    <>
      <div
        onClick={toggle}
        style={indentStyle}
        className="group flex items-center gap-1.5 pr-3 py-1 text-xs cursor-pointer transition-colors text-text-sub hover:bg-surface/40 border-l-2 border-l-transparent select-none"
        title={entry.path}
      >
        <ChevronRight
          size={12}
          strokeWidth={1.75}
          className={`shrink-0 text-text-faint transition-transform ${expanded ? "rotate-90" : ""}`}
        />
        <Folder size={12} strokeWidth={1.75} className="text-text-faint shrink-0" />
        <span className="flex-1 min-w-0 truncate">{entry.display_name}</span>
        {status === "loading" && (
          <Loader2
            size={12}
            strokeWidth={1.75}
            className="shrink-0 text-text-faint animate-spin"
          />
        )}
        {status === "error" && (
          <AlertCircle
            size={12}
            strokeWidth={1.75}
            className="shrink-0 text-warning"
          />
        )}
        {entry.size_or_count > 0 && (
          <span className="shrink-0 text-[10px] text-text-faint tabular-nums">
            {entry.size_or_count}
          </span>
        )}
      </div>
      {expanded && status === "error" && (
        <div
          style={{ paddingLeft: `${(depth + 1) * 12 + 8}px` }}
          className="py-1 pr-3 text-[11px] text-warning flex items-center gap-2"
        >
          <span className="flex-1 min-w-0 break-words font-mono text-[10px]">
            {branch?.error ?? "unknown error"}
          </span>
          <button
            onClick={(e) => {
              e.stopPropagation();
              void retry();
            }}
            className="shrink-0 px-2 py-0.5 rounded bg-surface hover:bg-surface-hi text-text-sub hover:text-text text-[10px]"
          >
            Retry
          </button>
        </div>
      )}
      {expanded && status === "loaded" && branch?.children !== undefined && (
        <>
          {branch.children.length === 0 && (
            <div
              style={{ paddingLeft: `${(depth + 1) * 12 + 8}px` }}
              className="py-1 pr-3 text-[10px] text-text-faint italic"
            >
              empty
            </div>
          )}
          {branch.children.map((child) => (
            <Branch
              key={child.path}
              entry={child}
              depth={depth + 1}
              state={state}
              setState={setState}
              onLeafClick={onLeafClick}
              activePath={activePath}
            />
          ))}
        </>
      )}
    </>
  );
}

/**
 * Lazy socpak directory tree. Loads the root prefix on mount; each
 * directory branch fetches its own children on click. Children are
 * cached per branch path so collapse/re-expand is instant.
 *
 * Search semantics depend on whether `globalIndex` is populated:
 *
 * - **Global-index mode** (search non-empty AND `globalIndex != null`):
 *   the tree is hidden and a flat list of matches against the global
 *   path index is rendered instead. Matches anywhere in the path
 *   (`pyro`, `hangar`, etc.), capped at 200 entries. Clicking a hit
 *   loads that scene exactly the same way a tree-leaf click would.
 *   The tree's expanded state is preserved underneath, so clearing
 *   the search restores the user's prior navigation.
 * - **Tree mode** (search empty, OR `globalIndex == null`): the lazy
 *   tree drives the view. Search input filters the immediate
 *   top-level children by display name as before -- handy for
 *   narrowing the root list while the global index is still
 *   loading.
 *
 * When the index is still loading and the user types, the empty-state
 * message reads "Indexing... results will appear when ready." rather
 * than the misleading "no visible branches match" string.
 */
export function SocpakTree({
  rootPrefix,
  onLeafClick,
  activePath,
  globalIndex,
}: SocpakTreeProps): JSX.Element {
  const [state, setState] = useState<TreeState>(() => new Map());
  const [rootChildren, setRootChildren] = useState<SocpakDirEntry[] | null>(null);
  const [rootStatus, setRootStatus] = useState<"loading" | "loaded" | "error">(
    "loading",
  );
  const [rootError, setRootError] = useState<string | null>(null);
  const [search, setSearch] = useState("");

  const loadRoot = useCallback(async () => {
    setRootStatus("loading");
    setRootError(null);
    try {
      const children = await listSocpakDir(rootPrefix);
      setRootChildren(children);
      setRootStatus("loaded");
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setRootError(msg);
      setRootStatus("error");
    }
  }, [rootPrefix]);

  useEffect(() => {
    void loadRoot();
  }, [loadRoot]);

  const trimmedSearch = search.trim();
  const inSearchMode = trimmedSearch.length > 0;
  // When search is non-empty AND the index has landed, switch to the
  // flat global-index view. Otherwise we fall back to the legacy
  // "filter visible branches" behaviour against the loaded root.
  const useGlobalIndex = inSearchMode && globalIndex != null;
  const globalHits = useGlobalIndex
    ? filterGlobalIndex(globalIndex!, trimmedSearch)
    : [];
  const visibleChildren =
    rootChildren && !useGlobalIndex
      ? filterTreeChildren(rootChildren, search)
      : [];

  return (
    <div className="flex flex-col h-full min-h-0">
      <div className="px-3 py-2 border-b border-border/60 flex items-center gap-2 shrink-0">
        <input
          type="text"
          placeholder={
            globalIndex != null ? "Search all socpaks..." : "Filter visible branches..."
          }
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          className="flex-1 bg-transparent outline-none text-sm text-text placeholder:text-text-faint"
        />
      </div>
      <div className="flex-1 overflow-y-auto">
        {/* Global-index search mode. */}
        {useGlobalIndex && globalHits.length === 0 && (
          <div className="px-4 py-6 text-xs text-text-dim leading-relaxed">
            No socpaks match.
          </div>
        )}
        {useGlobalIndex && globalHits.length > 0 && (
          <>
            {globalHits.length >= GLOBAL_INDEX_RESULT_CAP && (
              <div className="px-3 py-1.5 text-[10px] text-text-faint border-b border-border/60">
                Showing first {GLOBAL_INDEX_RESULT_CAP} hits. Refine the
                query to narrow.
              </div>
            )}
            {globalHits.map((hit) => {
              const isActive = activePath === hit.path;
              return (
                <div
                  key={hit.path}
                  onClick={() =>
                    onLeafClick({
                      path: hit.path,
                      display_name: hit.display_name,
                      kind: "socpak_file",
                      size_or_count: 0,
                    })
                  }
                  className={`
                    flex items-start gap-1.5 px-3 py-1.5 text-xs cursor-pointer
                    transition-colors border-l-2
                    ${
                      isActive
                        ? "bg-primary/10 text-text border-l-accent"
                        : "text-text-sub hover:bg-surface/40 border-l-transparent"
                    }
                  `}
                  title={hit.path}
                >
                  <Box
                    size={12}
                    strokeWidth={1.75}
                    className="text-text-faint shrink-0 mt-0.5"
                  />
                  <div className="flex-1 min-w-0">
                    <div className="truncate">{hit.display_name}</div>
                    {hit.parent_display.length > 0 && (
                      <div className="truncate text-[10px] text-text-faint">
                        in {hit.parent_display}
                      </div>
                    )}
                  </div>
                </div>
              );
            })}
          </>
        )}

        {/* Tree mode. */}
        {!useGlobalIndex && rootStatus === "loading" && (
          <div className="px-4 py-6 text-xs text-text-dim flex items-center gap-2">
            <Loader2 size={14} strokeWidth={1.75} className="animate-spin" />
            <span>Listing {rootPrefix}...</span>
          </div>
        )}
        {!useGlobalIndex && rootStatus === "error" && (
          <div className="px-4 py-3 text-[11px] text-warning leading-snug border-b border-border/60">
            <div className="flex items-center gap-1.5 mb-1">
              <AlertCircle size={12} strokeWidth={1.75} />
              <span>Failed to list {rootPrefix}</span>
            </div>
            <div className="text-text-faint break-words font-mono text-[10px] mb-2">
              {rootError}
            </div>
            <button
              onClick={() => void loadRoot()}
              className="px-2 py-0.5 rounded bg-surface hover:bg-surface-hi text-text-sub hover:text-text text-[10px]"
            >
              Retry
            </button>
          </div>
        )}
        {!useGlobalIndex &&
          rootStatus === "loaded" &&
          visibleChildren.length === 0 && (
            <div className="px-4 py-6 text-xs text-text-dim leading-relaxed">
              {inSearchMode && globalIndex == null
                ? "Indexing... results will appear when ready."
                : inSearchMode
                  ? "No visible branches match the filter."
                  : "Empty directory."}
            </div>
          )}
        {!useGlobalIndex &&
          rootStatus === "loaded" &&
          visibleChildren.map((entry) => (
            <Branch
              key={entry.path}
              entry={entry}
              depth={0}
              state={state}
              setState={setState}
              onLeafClick={onLeafClick}
              activePath={activePath}
            />
          ))}
      </div>
    </div>
  );
}
