import { useCallback, useEffect, useRef, useState } from "react";
import { Download } from "lucide-react";
import { useDataCoreStore } from "../stores/datacore-store";
import { ResizeHandle } from "../components/resize-handle";
import { ExtractProgress } from "../components/extract-progress";
import {
  dcSearch,
  dcListTree,
  dcGetRecord,
  dcGetBacklinks,
  dcExportJson,
  dcExportXml,
  dcExportFolder,
  type TreeEntryDto,
  type SearchResultDto,
  type BacklinkDto,
} from "../lib/commands";

export function DataCoreBrowser() {
  const [navWidth, setNavWidth] = useState(350);
  const [extracting, setExtracting] = useState(false);
  const searchQuery = useDataCoreStore((s) => s.searchQuery);
  const setSearchQuery = useDataCoreStore((s) => s.setSearchQuery);
  const searching = useDataCoreStore((s) => s.searching);

  return (
    <div className="flex-1 flex flex-col overflow-hidden relative">
      <ExtractProgress active={extracting} onDone={() => setExtracting(false)} />
      {/* Toolbar */}
      <div className="flex items-center gap-2 px-3 border-b border-border bg-bg-alt shrink-0" style={{ height: "var(--toolbar-height)" }}>
        <input
          type="text"
          placeholder="Search records..."
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
          className="flex-1 bg-surface rounded-md px-3 py-1.5 text-sm text-text placeholder:text-text-faint outline-none focus:ring-1 focus:ring-ring"
        />
        {searching && (
          <span className="text-xs text-text-dim shrink-0">Searching...</span>
        )}
      </div>
      <div className="flex-1 flex overflow-hidden">
        <NavPanel width={navWidth} onExtractStart={() => setExtracting(true)} onExtractEnd={() => setExtracting(false)} />
        <ResizeHandle width={navWidth} onResize={setNavWidth} side="right" min={200} max={600} />
        <InspectorPanel />
      </div>
    </div>
  );
}

// ── Left panel: combined tree + search ──────────────────────────────────────

function NavPanel({ width, onExtractStart, onExtractEnd }: {
  width: number;
  onExtractStart: () => void;
  onExtractEnd: () => void;
}) {
  const searchQuery = useDataCoreStore((s) => s.searchQuery);
  const hasSearch = searchQuery.trim().length > 0;

  return (
    <div className="flex flex-col border-r border-border overflow-hidden shrink-0" style={{ width }}>
      {hasSearch ? <SearchResults /> : <TreePanel onExtractStart={onExtractStart} onExtractEnd={onExtractEnd} />}
    </div>
  );
}

// ── Search results (flat list when typing) ──────────────────────────────────

function SearchResults() {
  const searchQuery = useDataCoreStore((s) => s.searchQuery);
  const searchResults = useDataCoreStore((s) => s.searchResults);
  const setSearchResults = useDataCoreStore((s) => s.setSearchResults);
  const searching = useDataCoreStore((s) => s.searching);
  const setSearching = useDataCoreStore((s) => s.setSearching);
  const debounceRef = useRef<ReturnType<typeof setTimeout>>(null);

  const doSearch = useCallback(
    (query: string) => {
      setSearching(true);
      dcSearch(query)
        .then((results) => setSearchResults(results))
        .catch((err) => {
          console.error("Search failed:", err);
          setSearchResults([]);
        });
    },
    [setSearchResults, setSearching],
  );

  useEffect(() => {
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => doSearch(searchQuery), 150);
    return () => { if (debounceRef.current) clearTimeout(debounceRef.current); };
  }, [searchQuery, doSearch]);

  return (
    <>
      <div className="px-2.5 py-1 text-[11px] text-text-dim">
        {searching ? "Searching..." : `${searchResults.length} results`}
      </div>
      <div className="flex-1 overflow-y-auto">
        {searchResults.map((result) => (
          <SearchResultRow key={result.id} result={result} />
        ))}
      </div>
    </>
  );
}

function SearchResultRow({ result }: { result: SearchResultDto }) {
  const selectRecord = useSelectRecord();

  return (
    <button
      type="button"
      onClick={() => selectRecord(result.id)}
      className="w-full text-left px-2.5 py-1 hover:bg-surface transition-colors"
    >
      <div className="flex items-center justify-between gap-2">
        <span className="text-[13px] text-text truncate">{result.name}</span>
        <span className="text-[10px] text-primary shrink-0">{result.struct_type}</span>
      </div>
      <div className="text-[11px] text-text-dim truncate">{result.path}</div>
    </button>
  );
}

// ── Tree panel (browse when search is empty) ────────────────────────────────

function TreePanel({ onExtractStart, onExtractEnd }: {
  onExtractStart: () => void;
  onExtractEnd: () => void;
}) {
  return (
    <div className="flex-1 overflow-y-auto">
      <TreeLevel path="" depth={0} onExtractStart={onExtractStart} onExtractEnd={onExtractEnd} />
    </div>
  );
}

function TreeLevel({ path, depth, onExtractStart, onExtractEnd }: {
  path: string;
  depth: number;
  onExtractStart: () => void;
  onExtractEnd: () => void;
}) {
  const [entries, setEntries] = useState<TreeEntryDto[]>([]);
  const [loading, setLoading] = useState(true);
  const [expandedFolders, setExpandedFolders] = useState<Set<string>>(new Set());
  const selectRecord = useSelectRecord();

  useEffect(() => {
    setLoading(true);
    dcListTree(path)
      .then(setEntries)
      .catch((err) => console.error("Failed to list tree:", err))
      .finally(() => setLoading(false));
  }, [path]);

  const toggleFolder = (name: string) => {
    setExpandedFolders((prev) => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });
  };

  if (loading && entries.length === 0) {
    return (
      <div style={{ paddingLeft: depth * 16 }} className="text-[11px] text-text-faint px-2 py-0.5">
        Loading...
      </div>
    );
  }

  return (
    <>
      {entries.map((entry) => {
        if (entry.kind === "folder") {
          const expanded = expandedFolders.has(entry.name);
          const childPath = path ? `${path}/${entry.name}` : entry.name;
          return (
            <div key={`f:${entry.name}`}>
              <FolderRow
                name={entry.name}
                path={childPath}
                depth={depth}
                expanded={expanded}
                onToggle={() => toggleFolder(entry.name)}
                onExtractStart={onExtractStart}
                onExtractEnd={onExtractEnd}
              />
              {expanded && <TreeLevel path={childPath} depth={depth + 1} onExtractStart={onExtractStart} onExtractEnd={onExtractEnd} />}
            </div>
          );
        }
        return (
          <button
            key={`r:${entry.id}`}
            type="button"
            onClick={() => selectRecord(entry.id)}
            className="w-full text-left flex items-center h-6 hover:bg-surface transition-colors group"
            style={{ paddingLeft: depth * 16 + 22 }}
          >
            <span className="text-[13px] text-text-sub truncate flex-1">{entry.name}</span>
            <span className="text-[10px] text-text-faint pr-2 shrink-0">{entry.struct_type}</span>
          </button>
        );
      })}
    </>
  );
}

function FolderRow({ name, path, depth, expanded, onToggle, onExtractStart, onExtractEnd }: {
  name: string;
  path: string;
  depth: number;
  expanded: boolean;
  onToggle: () => void;
  onExtractStart: () => void;
  onExtractEnd: () => void;
}) {
  const handleExport = async (e: React.MouseEvent) => {
    e.stopPropagation();
    const { open } = await import("@tauri-apps/plugin-dialog");
    const dir = await open({ title: `Export "${name}" folder`, directory: true, multiple: false });
    if (!dir) return;

    onExtractStart();
    try {
      await dcExportFolder(path, "json", dir);
    } catch (err) {
      console.error("Folder export failed:", err);
    } finally {
      onExtractEnd();
    }
  };

  return (
    <button
      type="button"
      onClick={onToggle}
      className="w-full text-left flex items-center h-6 hover:bg-surface transition-colors group"
      style={{ paddingLeft: depth * 16 + 6 }}
    >
      <span className="text-[10px] w-4 text-text-dim">
        {expanded ? "\u25BC" : "\u25B6"}
      </span>
      <span className="text-[13px] text-text flex-1">{name}</span>
      <button
        type="button"
        onClick={handleExport}
        title={`Export all records in ${name}/`}
        className="hidden group-hover:flex items-center justify-center w-5 h-5 mr-1 rounded
                   text-text-dim hover:text-text hover:bg-surface-hi transition-colors
                   disabled:opacity-50"
      >
        <Download size={12} />
      </button>
    </button>
  );
}

// ── Right panel: Record inspector ───────────────────────────────────────────

function InspectorPanel() {
  const selectedRecord = useDataCoreStore((s) => s.selectedRecord);
  const loadingRecord = useDataCoreStore((s) => s.loadingRecord);
  const canGoBack = useDataCoreStore((s) => s.canGoBack);
  const canGoForward = useDataCoreStore((s) => s.canGoForward);
  const saving = useDataCoreStore((s) => s.saving);
  const setSaving = useDataCoreStore((s) => s.setSaving);

  const handleBack = useHandleNav("back");
  const handleForward = useHandleNav("forward");

  const handleExport = async (format: "json" | "xml") => {
    if (!selectedRecord) return;
    const { save } = await import("@tauri-apps/plugin-dialog");
    const path = await save({
      title: `Export ${format.toUpperCase()}`,
      defaultPath: `${selectedRecord.name}.${format}`,
      filters: [
        format === "json"
          ? { name: "JSON", extensions: ["json"] }
          : { name: "XML", extensions: ["xml"] },
      ],
    });
    if (!path) return;

    setSaving(true);
    try {
      if (format === "json") {
        await dcExportJson(selectedRecord.id, path);
      } else {
        await dcExportXml(selectedRecord.id, path);
      }
    } catch (err) {
      console.error(`Export ${format} failed:`, err);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="flex-1 flex flex-col overflow-hidden">
      {/* Nav bar */}
      <div className="flex items-center gap-1.5 px-2.5 py-1.5 border-b border-border">
        <NavButton label={"\u2190 Back"} enabled={canGoBack()} onClick={handleBack} />
        <NavButton label={"Forward \u2192"} enabled={canGoForward()} onClick={handleForward} />
        <div className="flex-1" />
        {selectedRecord ? (
          <span className="text-[13px] text-text truncate">
            {selectedRecord.name}
          </span>
        ) : (
          <span className="text-[13px] text-text-dim">No record selected</span>
        )}
      </div>

      {/* Body */}
      {loadingRecord ? (
        <div className="flex-1 flex items-center justify-center">
          <span className="text-text-dim text-sm">Loading record...</span>
        </div>
      ) : selectedRecord ? (
        <>
          {/* Record path */}
          <div className="px-2.5 py-1 text-[11px] text-text-faint border-b border-border truncate">
            {selectedRecord.path}
          </div>
          {/* Scrollable content: JSON tree + backlinks */}
          <div className="flex-1 overflow-y-auto">
            <div className="px-1">
              <JsonTree json={selectedRecord.json} />
            </div>
            <BacklinksSection recordId={selectedRecord.id} />
          </div>
          {/* Export bar */}
          <div className="flex items-center gap-2 px-2.5 py-1.5 border-t border-border">
            <button
              type="button"
              disabled={saving}
              onClick={() => handleExport("json")}
              className="px-3 py-1 text-xs bg-surface hover:bg-surface-hi text-text rounded-md transition-colors disabled:opacity-50"
            >
              {saving ? "Saving..." : "Export JSON"}
            </button>
            <button
              type="button"
              disabled={saving}
              onClick={() => handleExport("xml")}
              className="px-3 py-1 text-xs bg-surface hover:bg-surface-hi text-text rounded-md transition-colors disabled:opacity-50"
            >
              {saving ? "Saving..." : "Export XML"}
            </button>
          </div>
        </>
      ) : (
        <div className="flex-1 flex items-center justify-center">
          <span className="text-text-dim text-sm">Select a record to inspect</span>
        </div>
      )}
    </div>
  );
}

function NavButton({ label, enabled, onClick }: {
  label: string;
  enabled: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      disabled={!enabled}
      onClick={onClick}
      className={`px-2.5 py-1 text-xs rounded-md transition-colors ${
        enabled
          ? "bg-surface hover:bg-surface-hi text-text"
          : "text-text-faint cursor-default"
      }`}
    >
      {label}
    </button>
  );
}

// ── Backlinks section ───────────────────────────────────────────────────────

function BacklinksSection({ recordId }: { recordId: string }) {
  const [backlinks, setBacklinks] = useState<BacklinkDto[]>([]);
  const [loading, setLoading] = useState(true);
  const [expanded, setExpanded] = useState(false);
  const selectRecord = useSelectRecord();

  useEffect(() => {
    setLoading(true);
    setExpanded(false);
    dcGetBacklinks(recordId)
      .then(setBacklinks)
      .catch((err) => console.error("Failed to get backlinks:", err))
      .finally(() => setLoading(false));
  }, [recordId]);

  if (loading) {
    return (
      <div className="border-t border-border px-2.5 py-2">
        <span className="text-[11px] text-text-faint">Loading references...</span>
      </div>
    );
  }

  if (backlinks.length === 0) {
    return (
      <div className="border-t border-border px-2.5 py-2">
        <span className="text-[11px] text-text-faint">No incoming references</span>
      </div>
    );
  }

  return (
    <div className="border-t border-border">
      <button
        type="button"
        onClick={() => setExpanded(!expanded)}
        className="w-full text-left flex items-center gap-1.5 px-2.5 py-1.5 hover:bg-surface transition-colors"
      >
        <span className="text-[10px] text-text-dim">{expanded ? "\u25BC" : "\u25B6"}</span>
        <span className="text-xs text-text-sub">
          Referenced by ({backlinks.length})
        </span>
      </button>
      {expanded && (
        <div className="pb-1">
          {backlinks.map((bl) => (
            <button
              key={bl.id}
              type="button"
              onClick={() => selectRecord(bl.id)}
              className="w-full text-left px-4 py-0.5 text-[12px] text-primary hover:underline hover:bg-surface transition-colors truncate"
            >
              {bl.name}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

// ── JSON Tree viewer ────────────────────────────────────────────────────────

const GUID_REGEX = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

function JsonTree({ json }: { json: string }) {
  const parsed = useRef<unknown>(null);
  const [, setTick] = useState(0);

  if (parsed.current === null) {
    try {
      parsed.current = JSON.parse(json);
    } catch {
      return <div className="p-2 text-danger text-sm">Failed to parse record JSON</div>;
    }
  }

  const rerender = () => setTick((t) => t + 1);

  return (
    <div className="py-1 font-mono text-[12px]">
      <JsonNode value={parsed.current} name={null} depth={0} rerender={rerender} />
    </div>
  );
}

function JsonNode({ value, name, depth, rerender }: {
  value: unknown;
  name: string | null;
  depth: number;
  rerender: () => void;
}) {
  const selectRecord = useSelectRecord();

  if (value === null || value === undefined) {
    return (
      <div className="leading-[22px]" style={{ paddingLeft: depth * 16 }}>
        {name != null && <span className="text-text-sub">{name}: </span>}
        <span className="text-text-faint">null</span>
      </div>
    );
  }

  if (typeof value === "boolean") {
    return (
      <div className="leading-[22px]" style={{ paddingLeft: depth * 16 }}>
        {name != null && <span className="text-text-sub">{name}: </span>}
        <span className="text-accent">{value.toString()}</span>
      </div>
    );
  }

  if (typeof value === "number") {
    return (
      <div className="leading-[22px]" style={{ paddingLeft: depth * 16 }}>
        {name != null && <span className="text-text-sub">{name}: </span>}
        <span className="text-accent">{value}</span>
      </div>
    );
  }

  if (typeof value === "string") {
    const isClickableGuid = GUID_REGEX.test(value) && name === "_RecordId_";

    return (
      <div className="leading-[22px]" style={{ paddingLeft: depth * 16 }}>
        {name != null && <span className="text-text-sub">{name}: </span>}
        {isClickableGuid ? (
          <button
            type="button"
            onClick={() => selectRecord(value)}
            className="text-primary hover:underline"
          >
            {value}
          </button>
        ) : (
          <span className="text-success">&quot;{value}&quot;</span>
        )}
      </div>
    );
  }

  if (Array.isArray(value)) {
    return <CollapsibleNode name={name} value={value} depth={depth} rerender={rerender} isArray />;
  }

  if (typeof value === "object") {
    return <CollapsibleNode name={name} value={value as Record<string, unknown>} depth={depth} rerender={rerender} isArray={false} />;
  }

  return null;
}

function CollapsibleNode({ name, value, depth, rerender, isArray }: {
  name: string | null;
  value: unknown[] | Record<string, unknown>;
  depth: number;
  rerender: () => void;
  isArray: boolean;
}) {
  const [expanded, setExpanded] = useState(depth < 2);
  const selectRecord = useSelectRecord();

  const entries = isArray
    ? (value as unknown[]).map((v, i) => [String(i), v] as const)
    : Object.entries(value as Record<string, unknown>);

  const count = entries.length;

  // Check if this object is a reference (has _RecordId_ field)
  const recordId = !isArray && typeof (value as Record<string, unknown>)._RecordId_ === "string"
    ? (value as Record<string, unknown>)._RecordId_ as string
    : null;

  const recordName = !isArray && typeof (value as Record<string, unknown>)._RecordName_ === "string"
    ? (value as Record<string, unknown>)._RecordName_ as string
    : null;

  const typeName = !isArray && typeof (value as Record<string, unknown>)._Type_ === "string"
    ? (value as Record<string, unknown>)._Type_ as string
    : null;

  const toggle = () => {
    setExpanded(!expanded);
    rerender();
  };

  const summary = isArray
    ? `[${count}]`
    : typeName || `{${count}}`;

  return (
    <div>
      <div
        className="leading-[22px] flex items-center gap-0.5 cursor-pointer hover:bg-surface/50 transition-colors"
        style={{ paddingLeft: depth * 16 }}
        onClick={toggle}
        onKeyDown={(e) => e.key === "Enter" && toggle()}
        role="button"
        tabIndex={0}
      >
        <span className="text-[10px] w-3.5 text-text-dim shrink-0">
          {expanded ? "\u25BC" : "\u25B6"}
        </span>
        {name != null && <span className="text-text-sub">{name} </span>}
        <span className="text-text-faint">{summary}</span>
        {recordId && (
          <button
            type="button"
            onClick={(e) => { e.stopPropagation(); selectRecord(recordId); }}
            className="ml-2 text-[10px] text-primary hover:underline"
          >
            {recordName ? `\u2192 ${recordName}` : "\u2192 open"}
          </button>
        )}
      </div>
      {expanded &&
        entries.map(([key, val]) => (
          <JsonNode key={key} name={key} value={val} depth={depth + 1} rerender={rerender} />
        ))}
    </div>
  );
}

// ── Hooks ───────────────────────────────────────────────────────────────────

function useSelectRecord() {
  const setSelectedRecord = useDataCoreStore((s) => s.setSelectedRecord);
  const setLoadingRecord = useDataCoreStore((s) => s.setLoadingRecord);
  const navigateTo = useDataCoreStore((s) => s.navigateTo);

  return useCallback(
    (recordId: string) => {
      setLoadingRecord(true);
      navigateTo(recordId);
      dcGetRecord(recordId)
        .then((record) => setSelectedRecord(record))
        .catch((err) => {
          console.error("Failed to load record:", err);
          setLoadingRecord(false);
        });
    },
    [setSelectedRecord, setLoadingRecord, navigateTo],
  );
}

function useHandleNav(direction: "back" | "forward") {
  const goBack = useDataCoreStore((s) => s.goBack);
  const goForward = useDataCoreStore((s) => s.goForward);
  const setSelectedRecord = useDataCoreStore((s) => s.setSelectedRecord);
  const setLoadingRecord = useDataCoreStore((s) => s.setLoadingRecord);

  return useCallback(() => {
    const id = direction === "back" ? goBack() : goForward();
    if (id) {
      setLoadingRecord(true);
      dcGetRecord(id)
        .then((record) => setSelectedRecord(record))
        .catch((err) => {
          console.error("Nav failed:", err);
          setLoadingRecord(false);
        });
    }
  }, [direction, goBack, goForward, setSelectedRecord, setLoadingRecord]);
}
