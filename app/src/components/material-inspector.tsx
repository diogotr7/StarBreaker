import { useMemo, useRef, useState, type ReactNode } from "react";
import * as THREE from "three";
import type { SubmaterialRecord } from "../lib/decomposed-loader";
import { writeDiagFile, listDiagDir } from "../lib/commands";

/** A single ray-hit row produced by the picker. The picker walks
 *  the bindings list at click time and assembles one entry per mesh
 *  intersected by the camera ray, sorted near-to-far. */
export interface PickHit {
  /** Hit ordering (0 = closest to camera). Preserved in the export
   *  so the user can reason about occlusion: the front-most surface
   *  is what they "saw" when they clicked, the rest are layered
   *  behind it. */
  hitIndex: number;
  /** Distance from camera in scene units. */
  distance: number;
  /** The intersected mesh. Held by reference so the inspector can
   *  reach `mesh.name`, `mesh.material`, `mesh.userData`, etc. */
  mesh: THREE.Mesh;
  /** Hit point in scene world-space coordinates (Star Citizen basis
   *  if `applySCBasisToObject` was applied to the parent group). */
  point: THREE.Vector3;
  /** Resolved submaterial driving the mesh. Null when the mesh isn't
   *  in `bindingsRef` (interior placeholders, ground plane). */
  submaterial: SubmaterialRecord | null;
  /** Sidecar path that owned `submaterial` at the time of the pick. */
  sidecarKey: string | null;
  /** Sidecar path the GLB material originally resolved against. Same
   *  as `sidecarKey` unless a livery rebuild swapped the binding. */
  defaultSidecarKey: string | null;
}

/** Directory where diagnostic JSON captures are saved. */
const DIAG_SUBDIR = "diag";

interface Props {
  hits: PickHit[];
  selectedHitIndex: number | null;
  onSelectHit: (idx: number | null) => void;
  onClose: () => void;
  /** Slug derived from the loaded package name (e.g. "rsi_aurora_mk2").
   *  Included in the saved filename. Falls back to "unknown". */
  packageSlug?: string;
  /** Active livery identifier. "nolivery" when none active. */
  liveryState?: string;
}

/** Floating panel anchored top-left of the viewer. Shows the
 *  ray-hit list and (when a hit is selected) a details drawer with the
 *  full submaterial record + live Three.js material state. Both panels
 *  expose Copy JSON and Save JSON buttons; key order is preserved across
 *  serialization so the diffability of the dump is not lost to alpha
 *  sort. */
export function MaterialInspector({
  hits,
  selectedHitIndex,
  onSelectHit,
  onClose,
  packageSlug = "unknown",
  liveryState = "nolivery",
}: Props){
  const selected = useMemo(() => {
    if (selectedHitIndex == null) return null;
    return hits.find((h) => h.hitIndex === selectedHitIndex) ?? null;
  }, [hits, selectedHitIndex]);

  // Session-scoped counter for saved filenames. Persists across hits
  // within a session; the user can rename files as needed.
  const sessionCounterRef = useRef(0);
  const [hitsConfirm, setHitsConfirm] = useState<string | null>(null);
  const [detailConfirm, setDetailConfirm] = useState<string | null>(null);

  const saveHitsJson = async () => {
    const payload = {
      kind: "starbreaker-viewer.ray-hits",
      hit_count: hits.length,
      hits: hits.map((h) => buildHitPayload(h, false)),
    };
    const json = JSON.stringify(payload, null, 2);
    const filename = await buildFilename(packageSlug, "ray_hits", liveryState);
    try {
      const absPath = await writeDiagFile(DIAG_SUBDIR, filename, json);
      sessionCounterRef.current += 1;
      setHitsConfirm(absPath);
      // Copy path to clipboard as confirmation.
      try { await navigator.clipboard.writeText(absPath); } catch { /* ignore */ }
      setTimeout(() => setHitsConfirm(null), 2000);
    } catch (err) {
      console.error("[save-json] write_diag_file failed:", err);
    }
  };

  const saveDetailJson = async (hit: PickHit) => {
    const payload = {
      kind: "starbreaker-viewer.hit-detail",
      hit: buildHitPayload(hit, true),
    };
    const json = JSON.stringify(payload, null, 2);
    const meshName = hit.mesh.name ? slugify(hit.mesh.name) : "mesh";
    const filename = await buildFilename(packageSlug, meshName, liveryState);
    try {
      const absPath = await writeDiagFile(DIAG_SUBDIR, filename, json);
      sessionCounterRef.current += 1;
      setDetailConfirm(absPath);
      try { await navigator.clipboard.writeText(absPath); } catch { /* ignore */ }
      setTimeout(() => setDetailConfirm(null), 2000);
    } catch (err) {
      console.error("[save-json] write_diag_file failed:", err);
    }
  };

  if (hits.length === 0) return null;

  return (
    <div className="absolute top-2 left-2 z-20 flex flex-col gap-2 max-h-[85vh] w-[420px]">
      {/* Hit list panel */}
      <div className="rounded-md bg-bg-alt/95 border border-border shadow-lg flex flex-col min-h-0">
        <div className="flex items-center justify-between px-3 py-1.5 border-b border-border">
          <p className="text-xs font-mono text-text-sub">
            Ray hits ({hits.length})
          </p>
          <div className="flex gap-1 items-center">
            {hitsConfirm && (
              <span className="text-[10px] text-success font-mono truncate max-w-[160px]" title={hitsConfirm}>
                Saved
              </span>
            )}
            <button
              type="button"
              className="text-[10px] px-2 py-0.5 rounded bg-bg/50 hover:bg-bg/80 text-text-sub border border-border"
              onClick={() => copyHitsJson(hits)}
              title="Copy ordered JSON of all ray hits to clipboard"
            >
              Copy JSON
            </button>
            <button
              type="button"
              className="text-[10px] px-2 py-0.5 rounded bg-bg/50 hover:bg-bg/80 text-text-sub border border-border"
              onClick={saveHitsJson}
              title={`Save ray hits JSON to ${DIAG_SUBDIR}`}
            >
              Save JSON
            </button>
            <button
              type="button"
              className="text-[10px] px-2 py-0.5 rounded bg-bg/50 hover:bg-bg/80 text-text-sub border border-border"
              onClick={onClose}
              title="Close inspector"
            >
              x
            </button>
          </div>
        </div>
        <div className="overflow-y-auto max-h-[35vh]">
          {hits.map((h) => (
            <button
              type="button"
              key={h.hitIndex}
              onClick={() =>
                onSelectHit(h.hitIndex === selectedHitIndex ? null : h.hitIndex)
              }
              className={`block w-full text-left px-3 py-1 font-mono text-[11px] border-b border-border/40 hover:bg-bg/40 ${
                h.hitIndex === selectedHitIndex
                  ? "bg-accent/10 text-text"
                  : "text-text-sub"
              }`}
            >
              <div className="flex items-baseline gap-2">
                <span className="text-text-dim w-6 text-right">
                  {h.hitIndex}
                </span>
                <span className="text-text-dim w-16">
                  {h.distance.toFixed(1)}u
                </span>
                <span className="truncate flex-1">
                  {hitLabel(h)}
                </span>
              </div>
              <div className="text-[10px] text-text-dim pl-8 truncate">
                {h.submaterial?.shader_family ?? "-"} {" "}
                {h.mesh.name || "(unnamed mesh)"}
              </div>
            </button>
          ))}
        </div>
      </div>
      {/* Details panel */}
      {selected && (
        <div className="rounded-md bg-bg-alt/95 border border-border shadow-lg flex flex-col min-h-0">
          <div className="flex items-center justify-between px-3 py-1.5 border-b border-border">
            <p className="text-xs font-mono text-text-sub truncate">
              Detail #{selected.hitIndex}: {hitLabel(selected)}
            </p>
            <div className="flex gap-1 items-center shrink-0">
              {detailConfirm && (
                <span className="text-[10px] text-success font-mono" title={detailConfirm}>
                  Saved
                </span>
              )}
              <button
                type="button"
                className="text-[10px] px-2 py-0.5 rounded bg-bg/50 hover:bg-bg/80 text-text-sub border border-border"
                onClick={() => copyDetailJson(selected)}
                title="Copy full ordered JSON of this submaterial + material to clipboard"
              >
                Copy JSON
              </button>
              <button
                type="button"
                className="text-[10px] px-2 py-0.5 rounded bg-bg/50 hover:bg-bg/80 text-text-sub border border-border"
                onClick={() => saveDetailJson(selected)}
                title={`Save detail JSON to ${DIAG_SUBDIR}`}
              >
                Save JSON
              </button>
            </div>
          </div>
          <div className="overflow-y-auto max-h-[40vh] px-3 py-2 font-mono text-[11px] text-text-sub leading-snug">
            <DetailBody hit={selected} />
          </div>
        </div>
      )}
    </div>
  );
}

function hitLabel(hit: PickHit): string {
  return (
    hit.submaterial?.blender_material_name ??
    hit.submaterial?.submaterial_name ??
    hit.mesh.name ??
    "(unbound)"
  );
}

function DetailBody({ hit }: { hit: PickHit }) {
  const sm = hit.submaterial;
  const mat = hit.mesh.material as THREE.Material | THREE.Material[] | null;
  const matSummary = summarizeThreeMaterial(mat);
  return (
    <div className="space-y-2">
      <Section title="Mesh">
        <Kv k="name" v={hit.mesh.name || "—"} />
        <Kv k="distance" v={hit.distance.toFixed(2)} />
        <Kv
          k="point"
          v={`(${hit.point.x.toFixed(1)}, ${hit.point.y.toFixed(1)}, ${hit.point.z.toFixed(1)})`}
        />
        <Kv
          k="glbMaterialIndex"
          v={String(hit.mesh.userData?.glbMaterialIndex ?? "—")}
        />
        <Kv
          k="fallbackKind"
          v={String(
            (Array.isArray(hit.mesh.material)
              ? hit.mesh.material[0]?.userData?.fallbackKind
              : (hit.mesh.material as THREE.Material | null)?.userData
                  ?.fallbackKind) ?? "—",
          )}
        />
        <Kv k="active sidecar" v={hit.sidecarKey ?? "—"} />
        <Kv k="default sidecar" v={hit.defaultSidecarKey ?? "—"} />
      </Section>
      {sm ? (
        <>
          <Section title="Submaterial">
            <Kv k="submaterial_name" v={sm.submaterial_name ?? "—"} />
            <Kv k="blender_material_name" v={sm.blender_material_name ?? "—"} />
            <Kv k="index" v={String(sm.index ?? "—")} />
            <Kv k="shader" v={String(sm.shader ?? "—")} />
            <Kv k="shader_family" v={String(sm.shader_family ?? "—")} />
          </Section>
          {sm.decoded_feature_flags && (
            <Section title="decoded_feature_flags">
              {sm.decoded_feature_flags.tokens && (
                <Kv k="tokens" v={sm.decoded_feature_flags.tokens.join(" ")} />
              )}
              {Object.entries(sm.decoded_feature_flags)
                .filter(([k]) => k !== "tokens")
                .map(([k, v]) => (
                  <Kv key={k} k={k} v={String(v)} />
                ))}
            </Section>
          )}
          {sm.tint_palette && (
            <Section title="tint_palette">
              <Kv k="palette_id" v={sm.tint_palette.palette_id} />
              <Kv
                k="palette_source_name"
                v={sm.tint_palette.palette_source_name ?? "—"}
              />
              <Kv
                k="assigned_channel"
                v={sm.tint_palette.assigned_channel ?? "(null — heuristic-eligible)"}
              />
              {sm.tint_palette.entries?.length ? (
                <div className="mt-1 space-y-0.5">
                  {sm.tint_palette.entries.map((e, i) => (
                    <div key={i} className="text-[10px] text-text-dim pl-2">
                      [{i}] {e.channel} · tint=
                      {fmtTriplet(e.tint_color)} · spec=
                      {e.spec_color ? fmtTriplet(e.spec_color) : "—"} · gloss=
                      {e.glossiness?.toFixed(2) ?? "—"}
                    </div>
                  ))}
                </div>
              ) : (
                <Kv k="entries" v="(empty)" />
              )}
            </Section>
          )}
          {sm.palette_routing && (
            <Section title="palette_routing">
              <pre className="text-[10px] text-text-dim whitespace-pre-wrap break-all">
                {JSON.stringify(sm.palette_routing, null, 2)}
              </pre>
            </Section>
          )}
          {sm.layer_manifest?.length ? (
            <Section title={`layer_manifest (${sm.layer_manifest.length})`}>
              <pre className="text-[10px] text-text-dim whitespace-pre-wrap break-all">
                {JSON.stringify(sm.layer_manifest, null, 2)}
              </pre>
            </Section>
          ) : null}
          {sm.texture_slots?.length ? (
            <Section title={`texture_slots (${sm.texture_slots.length})`}>
              {sm.texture_slots.map((slot, i) => (
                <div key={i} className="text-[10px] text-text-dim">
                  [{i}] {slot.slot ?? "?"} · {slot.role ?? "?"}
                  {slot.alpha_semantic ? ` · α=${slot.alpha_semantic}` : ""}
                  {slot.export_path ? (
                    <div className="pl-2 break-all">{slot.export_path}</div>
                  ) : null}
                </div>
              ))}
            </Section>
          ) : null}
          {sm.direct_textures?.length ? (
            <Section title={`direct_textures (${sm.direct_textures.length})`}>
              <pre className="text-[10px] text-text-dim whitespace-pre-wrap break-all">
                {JSON.stringify(sm.direct_textures, null, 2)}
              </pre>
            </Section>
          ) : null}
        </>
      ) : (
        <Section title="Submaterial">
          <p className="text-[10px] text-text-dim italic">
            Mesh has no recorded MaterialBinding (interior, light, ground, or
            unbound auxiliary geometry).
          </p>
        </Section>
      )}
      <Section title="Three.js material">
        <pre className="text-[10px] text-text-dim whitespace-pre-wrap break-all">
          {JSON.stringify(matSummary, null, 2)}
        </pre>
      </Section>
    </div>
  );
}

function Section({
  title,
  children,
}: {
  title: string;
  children: ReactNode;
}) {
  return (
    <div>
      <p className="text-[10px] uppercase tracking-wide text-text-dim mb-0.5">
        {title}
      </p>
      <div className="pl-1">{children}</div>
    </div>
  );
}

function Kv({ k, v }: { k: string; v: string }) {
  return (
    <div className="flex gap-2 text-[11px]">
      <span className="text-text-dim w-32 shrink-0">{k}</span>
      <span className="text-text-sub break-all">{v}</span>
    </div>
  );
}

function fmtTriplet(t: readonly [number, number, number] | number[]): string {
  if (!t || t.length < 3) return "—";
  return `(${t[0].toFixed(2)}, ${t[1].toFixed(2)}, ${t[2].toFixed(2)})`;
}

/** Build a JSON-friendly summary of a Three.js material with insertion-
 *  ordered fields. Includes only the properties relevant to the
 *  CryEngine port: PBR scalars, the texture maps that were assigned,
 *  ClearCoat/iridescence/sheen/transmission state. The texture maps
 *  emit the source export path when available (we stash it in
 *  userData during loadTexture) and fall back to "(no path tracked)"
 *  otherwise. */
function summarizeThreeMaterial(
  mat: THREE.Material | THREE.Material[] | null,
): Record<string, unknown> {
  if (mat == null) return { type: "(none)" };
  if (Array.isArray(mat)) {
    return { type: "(array)", length: mat.length };
  }
  const out: Record<string, unknown> = {};
  out.type = mat.type;
  out.name = mat.name;
  out.transparent = mat.transparent;
  out.opacity = mat.opacity;
  out.side = mat.side;
  if ("color" in mat && mat.color instanceof THREE.Color) {
    out.color = colorTriplet(mat.color);
  }
  if ("metalness" in mat) out.metalness = (mat as THREE.MeshStandardMaterial).metalness;
  if ("roughness" in mat) out.roughness = (mat as THREE.MeshStandardMaterial).roughness;
  if ("envMapIntensity" in mat) out.envMapIntensity = (mat as THREE.MeshStandardMaterial).envMapIntensity;
  out.clearcoat = (mat as any).clearcoat ?? null;
  out.clearcoatRoughness = (mat as any).clearcoatRoughness ?? null;
  out.iridescence = (mat as any).iridescence ?? null;
  out.sheen = (mat as any).sheen ?? null;
  if ("emissive" in mat && (mat as THREE.MeshStandardMaterial).emissive instanceof THREE.Color) {
    out.emissive = colorTriplet((mat as THREE.MeshStandardMaterial).emissive);
    out.emissiveIntensity = (mat as THREE.MeshStandardMaterial).emissiveIntensity;
  }
  // Texture map references — emit role names with the source path if
  // we tracked it, otherwise "(set)" so the user can see which slots
  // were actually populated. The order here mirrors the assignment
  // order in buildHardSurfaceMaterial so a JSON diff against another
  // submaterial reads visually.
  const matAsRecord = mat as unknown as Record<string, unknown>;
  for (const key of [
    "map",
    "normalMap",
    "roughnessMap",
    "metalnessMap",
    "emissiveMap",
    "alphaMap",
    "aoMap",
    "displacementMap",
  ] as const) {
    const tex = matAsRecord[key];
    if (tex instanceof THREE.Texture) {
      out[key] = textureSummary(tex);
    }
  }
  // ClearCoat / iridescence / sheen / transmission live on
  // MeshPhysicalMaterial. Only emit the field when it's present and
  // non-zero so the dump stays focused.
  const phys = mat as THREE.MeshPhysicalMaterial;
  if (typeof phys.clearcoat === "number" && phys.clearcoat > 0) {
    out.clearcoat = phys.clearcoat;
    out.clearcoatRoughness = phys.clearcoatRoughness;
  }
  if (typeof phys.iridescence === "number" && phys.iridescence > 0) {
    out.iridescence = phys.iridescence;
    out.iridescenceIOR = phys.iridescenceIOR;
    out.iridescenceThicknessRange = phys.iridescenceThicknessRange;
  }
  if (typeof phys.sheen === "number" && phys.sheen > 0) {
    out.sheen = phys.sheen;
    out.sheenColor = colorTriplet(phys.sheenColor);
    out.sheenRoughness = phys.sheenRoughness;
  }
  if (typeof phys.transmission === "number" && phys.transmission > 0) {
    out.transmission = phys.transmission;
    out.thickness = phys.thickness;
    out.ior = phys.ior;
  }
  if (typeof phys.anisotropy === "number" && phys.anisotropy > 0) {
    out.anisotropy = phys.anisotropy;
    out.anisotropyRotation = phys.anisotropyRotation;
  }
  // userData snapshots whatever buildMaterial / scene-viewer stashed
  // (glbMaterialIndex, gltfExtensions). Include verbatim — the user
  // can grep it.
  if (mat.userData && Object.keys(mat.userData).length > 0) {
    out.userData = mat.userData;
  }
  return out;
}

function colorTriplet(c: THREE.Color): [number, number, number] {
  return [c.r, c.g, c.b];
}

function textureSummary(tex: THREE.Texture): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  if (tex.name) out.name = tex.name;
  if (tex.userData?.export_path) out.export_path = tex.userData.export_path;
  out.colorSpace = tex.colorSpace;
  out.wrapS = tex.wrapS;
  out.wrapT = tex.wrapT;
  out.repeat = [tex.repeat.x, tex.repeat.y];
  out.offset = [tex.offset.x, tex.offset.y];
  return out;
}

/** Replace non-alphanumeric chars with underscores and lowercase. */
function slugify(s: string): string {
  return s.toLowerCase().replace(/[^a-z0-9]+/g, "_").replace(/^_|_$/g, "");
}

/**
 * Build a filename for a diagnostic JSON capture.
 * Format: {counter}-{ship_slug}-{geom_or_kind}-{state}.json
 * Counter auto-increments from the highest existing counter in DIAG_SUBDIR.
 */
async function buildFilename(
  shipSlug: string,
  geomOrKind: string,
  state: string,
): Promise<string> {
  let nextCounter = 1;
  try {
    const files = await listDiagDir(DIAG_SUBDIR);
    for (const f of files) {
      const m = f.match(/^(\d+)-/);
      if (m) {
        const n = parseInt(m[1], 10);
        if (n >= nextCounter) nextCounter = n + 1;
      }
    }
  } catch {
    // If listing fails, start at 001.
  }
  const counterStr = String(nextCounter).padStart(3, "0");
  const slug = slugify(shipSlug) || "unknown";
  const geom = slugify(geomOrKind) || "hit";
  const st = slugify(state) || "nolivery";
  return `${counterStr}-${slug}-${geom}-${st}.json`;
}

/** Build the ordered JSON for the entire ray-hit list and copy it to
 *  the clipboard. */
async function copyHitsJson(hits: PickHit[]): Promise<void> {
  const payload = {
    kind: "starbreaker-viewer.ray-hits",
    hit_count: hits.length,
    hits: hits.map((h) => buildHitPayload(h, false)),
  };
  await writeClipboardJson(payload);
}

/** Build the ordered JSON for a single hit (full submaterial +
 *  material details) and copy it to the clipboard. */
async function copyDetailJson(hit: PickHit): Promise<void> {
  const payload = {
    kind: "starbreaker-viewer.hit-detail",
    hit: buildHitPayload(hit, true),
  };
  await writeClipboardJson(payload);
}

/** Construct the ordered JSON object for a hit. Field order is
 *  insertion order, which JSON.stringify preserves on plain objects
 *  (for non-numeric string keys), so what you see in the export is
 *  what the loader saw at pick time. */
function buildHitPayload(
  hit: PickHit,
  full: boolean,
): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  out.hit_index = hit.hitIndex;
  out.distance = hit.distance;
  out.point = [hit.point.x, hit.point.y, hit.point.z];
  out.mesh = {
    name: hit.mesh.name,
    glbMaterialIndex: hit.mesh.userData?.glbMaterialIndex,
    geometry_uuid: hit.mesh.geometry?.uuid,
    geometry_attributes: hit.mesh.geometry?.attributes
      ? Object.keys(hit.mesh.geometry.attributes)
      : [],
  };
  out.sidecar = {
    active: hit.sidecarKey,
    default: hit.defaultSidecarKey,
    overridden: hit.sidecarKey !== hit.defaultSidecarKey,
  };
  if (full && hit.submaterial) {
    out.submaterial = orderedSubmaterial(hit.submaterial);
  } else if (hit.submaterial) {
    out.submaterial_name = hit.submaterial.submaterial_name;
    out.shader_family = hit.submaterial.shader_family;
  }
  if (full) {
    out.three_material = summarizeThreeMaterial(hit.mesh.material);
  }
  return out;
}

/** Re-emit the submaterial record with field order that mirrors the
 *  decomposed-export contract. JSON.stringify preserves insertion
 *  order, so this produces a stable diff-friendly serialization
 *  regardless of how the source object was constructed. Unknown extra
 *  fields are appended last to avoid silently dropping anything the
 *  Rust side added. */
function orderedSubmaterial(
  sm: SubmaterialRecord,
): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  const ordered: (keyof SubmaterialRecord)[] = [
    "index",
    "submaterial_name",
    "blender_material_name",
    "shader",
    "shader_family",
    "decoded_feature_flags",
    "tint_palette",
    "palette_routing",
    "layer_manifest",
    "texture_slots",
    "direct_textures",
  ];
  for (const k of ordered) {
    if (sm[k] !== undefined) out[k as string] = sm[k];
  }
  for (const k of Object.keys(sm)) {
    if (!(k in out)) out[k] = (sm as Record<string, unknown>)[k];
  }
  return out;
}

async function writeClipboardJson(payload: unknown): Promise<void> {
  const json = JSON.stringify(payload, null, 2);
  try {
    if (typeof navigator !== "undefined" && navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(json);
      console.info(`[picker] copied ${json.length} bytes of JSON to clipboard`);
      return;
    }
  } catch (err) {
    console.warn("[picker] clipboard write failed:", err);
  }
  // Fallback: log to console so the user can copy from there.
  console.info("[picker] JSON payload (no clipboard available):\n" + json);
}
