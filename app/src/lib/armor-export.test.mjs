import { test } from "node:test";
import assert from "node:assert/strict";
import { armorCategories, selectedExportEntities } from "./armor-export.ts";

const item = (id, armor_type) => ({ id, name: id, display_name: null, is_npc_or_internal: false, armor_type });

test("armor browsing excludes other entities and groups by data-derived type", () => {
  const groups = armorCategories([{ name: "Other", entities: [item("vessel", null), item("head", "Helmet"), item("body", "Torso")] }]);
  assert.deepEqual(groups.map((group) => group.name), ["All armor", "Helmet", "Torso"]);
  assert.deepEqual(groups[0].entities.map((entity) => entity.id), ["body", "head"]);
});

test("armor export excludes hidden selections and exports grouped items only once", () => {
  const groups = armorCategories([{ name: "Other", entities: [item("vessel", null), item("head", "Helmet")] }]);
  assert.deepEqual(selectedExportEntities(groups, new Set(["vessel", "head"])).map((entity) => entity.id), ["head"]);
});

test("an archive without armor retains an empty all category", () => {
  assert.deepEqual(armorCategories([]), [{ name: "All armor", entities: [] }]);
});
