# Sparse light class regression

Extracted from Star Citizen LIVE 4.10.2, `Data/Game2.dcb` record
`EntityClassDefinition.Light` → `Components[LightComponentParams]`, and
`Data/ObjectContainers/Ships/AEGS/Gladius/base_lightgroups.socpak`.
`gladius_projectors.xml` retains entities Light-088 and Light-089 from the
container's CryXML chunk. `light_component.json` is the complete queried
component, with no hand-written defaults. These are test evidence only.

The live regression re-reads the record and container when Data.p4k is
available, comparing the fixture and exported values with current data.
The synthetic partial-state test removes the default intensity to exercise
inheritance; explicit zero remains an override.
