// Pure-helper coverage for the SocpakTree component. Skips the React
// rendering itself because the project's vitest setup runs in node mode
// without jsdom or @testing-library, and adding either would violate
// the no-new-deps constraint. The state-mutation helpers and the
// search filter are the load-bearing pieces; if they are right, the
// component only has wiring to get wrong.

import { describe, expect, it } from "vitest";
import {
  GLOBAL_INDEX_RESULT_CAP,
  filterGlobalIndex,
  filterTreeChildren,
  setBranchState,
  toGlobalIndexHit,
} from "../socpak-tree";
import type { SocpakDirEntry } from "../../lib/commands";

function dir(path: string, name: string, count = 0): SocpakDirEntry {
  return {
    path,
    display_name: name,
    kind: "directory",
    size_or_count: count,
  };
}

function leaf(path: string, name: string, size = 0): SocpakDirEntry {
  return {
    path,
    display_name: name,
    kind: "socpak_file",
    size_or_count: size,
  };
}

describe("filterTreeChildren", () => {
  const sample: SocpakDirEntry[] = [
    dir("Data\\OC\\PU\\", "PU", 5),
    dir("Data\\OC\\Stations\\", "Stations", 3),
    leaf("Data\\OC\\hangar.socpak", "hangar.socpak", 1024),
    leaf("Data\\OC\\dungeon.socpak", "dungeon.socpak", 2048),
  ];

  it("returns the input unchanged when the query is empty", () => {
    expect(filterTreeChildren(sample, "")).toBe(sample);
    expect(filterTreeChildren(sample, "   ")).toBe(sample);
  });

  it("filters by display_name substring, case-insensitively", () => {
    const result = filterTreeChildren(sample, "hangar");
    expect(result).toHaveLength(1);
    expect(result[0].display_name).toBe("hangar.socpak");
  });

  it("matches against directory names too", () => {
    const result = filterTreeChildren(sample, "stat");
    expect(result).toHaveLength(1);
    expect(result[0].display_name).toBe("Stations");
  });

  it("returns an empty array when nothing matches", () => {
    const result = filterTreeChildren(sample, "no_such_name");
    expect(result).toEqual([]);
  });

  it("trims whitespace before matching", () => {
    const result = filterTreeChildren(sample, "  PU  ");
    expect(result).toHaveLength(1);
    expect(result[0].display_name).toBe("PU");
  });
});

describe("setBranchState", () => {
  it("returns a NEW Map (reference inequality) so React detects the change", () => {
    const before = new Map();
    const after = setBranchState(before, "p", {
      status: "loaded",
      expanded: true,
      children: [],
    });
    expect(after).not.toBe(before);
  });

  it("inserts a new entry without mutating the input", () => {
    const before = new Map();
    setBranchState(before, "p", {
      status: "loading",
      expanded: true,
    });
    expect(before.size).toBe(0);
  });

  it("overwrites an existing entry at the same key", () => {
    let state = new Map();
    state = setBranchState(state, "p", {
      status: "loading",
      expanded: true,
    });
    state = setBranchState(state, "p", {
      status: "loaded",
      expanded: true,
      children: [leaf("p\\a.socpak", "a.socpak", 10)],
    });
    const branch = state.get("p");
    expect(branch?.status).toBe("loaded");
    expect(branch?.children).toHaveLength(1);
  });

  it("preserves siblings at other keys", () => {
    let state = new Map();
    state = setBranchState(state, "p", { status: "loaded", expanded: true });
    state = setBranchState(state, "q", { status: "loading", expanded: true });
    state = setBranchState(state, "p", { status: "loaded", expanded: false });
    expect(state.get("p")?.expanded).toBe(false);
    expect(state.get("q")?.status).toBe("loading");
  });
});

describe("toGlobalIndexHit", () => {
  it("splits a Windows-flavoured path into name + parent", () => {
    const hit = toGlobalIndexHit(
      "Data\\ObjectContainers\\PU\\loc\\mod\\pyro\\hangar.socpak",
    );
    expect(hit.path).toBe(
      "Data\\ObjectContainers\\PU\\loc\\mod\\pyro\\hangar.socpak",
    );
    expect(hit.display_name).toBe("hangar.socpak");
    // Backslashes get normalised to forward slashes for legibility.
    expect(hit.parent_display).toBe("Data/ObjectContainers/PU/loc/mod/pyro");
  });

  it("handles forward-slash paths", () => {
    const hit = toGlobalIndexHit("Data/ObjectContainers/PU/foo.socpak");
    expect(hit.display_name).toBe("foo.socpak");
    expect(hit.parent_display).toBe("Data/ObjectContainers/PU");
  });

  it("returns the input as display_name when no separator is present", () => {
    const hit = toGlobalIndexHit("flat.socpak");
    expect(hit.display_name).toBe("flat.socpak");
    expect(hit.parent_display).toBe("");
  });
});

describe("filterGlobalIndex", () => {
  const sample = [
    "Data\\ObjectContainers\\PU\\loc\\mod\\pyro\\station\\reststop\\executive_hangar.socpak",
    "Data\\ObjectContainers\\PU\\loc\\mod\\stanton\\hangar.socpak",
    "Data\\ObjectContainers\\Stations\\alpha\\dungeon.socpak",
    "Data\\ObjectContainers\\Stations\\alpha\\nested\\dungeon_b.socpak",
    "Data\\ObjectContainers\\PU\\misc\\pyro_other.socpak",
  ];

  it("returns an empty list when the query is empty", () => {
    expect(filterGlobalIndex(sample, "")).toEqual([]);
    expect(filterGlobalIndex(sample, "   ")).toEqual([]);
  });

  it("matches case-insensitive substring on the full path", () => {
    // `pyro` matches both the directory name AND the filename
    // `pyro_other.socpak`. Both should land in the result.
    const hits = filterGlobalIndex(sample, "PYRO");
    expect(hits.map((h) => h.display_name)).toEqual([
      "executive_hangar.socpak",
      "pyro_other.socpak",
    ]);
  });

  it("matches against directory segments, not just filenames", () => {
    // `Stations` is a parent directory; the lazy tree's
    // display_name filter would never find these.
    const hits = filterGlobalIndex(sample, "stations");
    expect(hits).toHaveLength(2);
    expect(hits[0].display_name).toBe("dungeon.socpak");
    expect(hits[1].display_name).toBe("dungeon_b.socpak");
  });

  it("respects an explicit cap", () => {
    const hits = filterGlobalIndex(sample, "socpak", 2);
    expect(hits).toHaveLength(2);
  });

  it("returns at most GLOBAL_INDEX_RESULT_CAP hits by default", () => {
    const big = Array.from(
      { length: 500 },
      (_, i) => `Data\\ObjectContainers\\PU\\zone_${i}.socpak`,
    );
    const hits = filterGlobalIndex(big, "zone");
    expect(hits).toHaveLength(GLOBAL_INDEX_RESULT_CAP);
  });

  it("returns each hit with display_name + parent_display populated", () => {
    const hits = filterGlobalIndex(sample, "executive");
    expect(hits).toHaveLength(1);
    expect(hits[0].display_name).toBe("executive_hangar.socpak");
    expect(hits[0].parent_display).toBe(
      "Data/ObjectContainers/PU/loc/mod/pyro/station/reststop",
    );
  });
});
