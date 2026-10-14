# StarBreaker Copilot instructions

The project rules live in [`AGENTS.md`](../AGENTS.md) at the repo root; read it
first. Read [`blender_addon/AGENTS.md`](../blender_addon/AGENTS.md) before any
Blender addon work, and `crates/starbreaker-ui/AGENTS.md` before UI work. Those
files take precedence over this one; this file only adds Copilot-specific
workflow.

Commands not covered there:

```bash
# Tauri app (app/package.json)
cd app && npm ci && npm run build && npm run lint

# Documentation website (website/package.json)
cd website && npm ci && npm run build
```

## Visual Fix Loop (VFL)

Use VFL when correctness is visual, interactive or subjective — Blender
lighting, UVs, materials, decals, transforms, scene assembly, addon UI/import
behaviour, Tauri UI, or perceived performance. When any of these apply, VFL is
required:

1. **Blender MCP first.** If a candidate fix can be tested or approximated in
   the connected Blender instance (rotate an empty, change a node input, try an
   alternate matrix), prove it there before changing Rust or addon code. Only
   rebuild or re-export when needed to make the fix persistent or to test what
   can't be exercised live. Reset scenes and sync the addon as described in
   `blender_addon/AGENTS.md`.
2. **One item at a time.** Research, plan, make one focused fix, validate it
   (MCP, or rebuild/re-export/reload when MCP can't), then ask the user to
   verify that item before moving on.
3. **Every fix is a hypothesis until the user confirms it.** Don't claim it is
   done, and don't commit it, until the user confirms or asks for an
   intermediate commit.
4. **Ask with the question tool**, never in plain text: one clear question, a
   success choice (e.g. `Yes, this is fixed`), and a free-text option.
5. **Slowdowns are part of the task.** If the change makes import, refresh or
   UI interaction slower, fix that before exiting (`tools/profile_aurora_blend.sh`
   profiles the decomposed `.blend` export; it defaults to a Linux Wine/Proton
   `Data.p4k` path, so set `SC_DATA_P4K` elsewhere).
6. **Blender crashes stop the attempt.** Find and address the cause before
   resuming; if Blender must restart, ask the user with the question tool and
   wait.

Exit VFL only when the user confirms all scoped issues are fixed or tells you
to stop, pause, commit an intermediate state, or move on. Unrelated issues the
user raises go in the plan; ask (question tool) whether to add them to the
current scope.

## Planning questions

Ask planning and next-step questions (which option, whether to switch tasks,
whether to take on a discovered issue) with the question tool too, offering the
options as choices plus a free-text option.
