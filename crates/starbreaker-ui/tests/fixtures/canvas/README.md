# Real DataCore canvas fixtures

The five `*_<guid-prefix>.json` fixtures in this directory are real canvas
records from DataCore GUID lookups:

- `EC_PowerManagement_3228e5cc.json`
- `MC_S_Target_Master_b8d2d65c.json`
- `GEN_MC_S_Target_dd9ed6dc.json` (the generic target-MFD content canvas)
- `MC_S_Self_Master_680a71df.json`
- `BB_ScreenRadar_C_App_Starmap_68ff6d17.json`

`MC_S_Self_Master` and `BB_ScreenRadar_C_App_Starmap` were pruned because the
raw records were larger than 100KB. The pruned fields are localization-heavy
text/string tables and preview/runtime metadata that do not affect the static
scene hierarchy used by these tests.

Real records use a pointer-based flat scene graph: `scene[]` is a flat list of
`BuildingBlocks_*` nodes connected by `_Pointer_` / `_PointsTo_` references,
positions are authored as `{x, y, z}` in 1920×1080 space, and `canvas` may be a
`file://...` reference to another DataCore record.

Consumers: the `bb_layout` engine tests, and the `dump_representative_ir` and
`phase5_certification_dashboard` examples.
