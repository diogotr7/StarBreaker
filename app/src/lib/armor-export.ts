import type { CategoryDto } from "./commands";

export function armorCategories(categories: CategoryDto[]): CategoryDto[] {
  const entities = categories.flatMap((category) => category.entities)
    .filter((entity) => entity.armor_type !== null);
  const groups = new Map<string, CategoryDto["entities"]>();
  for (const entity of entities) {
    const name = entity.armor_type!;
    const group = groups.get(name) ?? [];
    group.push(entity);
    groups.set(name, group);
  }
  const sortEntities = (items: CategoryDto["entities"]) => items.sort((a, b) =>
    (a.display_name ?? a.name).localeCompare(b.display_name ?? b.name));
  return [
    { name: "All armor", entities: sortEntities(entities) },
    ...[...groups].sort(([a], [b]) => a.localeCompare(b))
      .map(([name, items]) => ({ name, entities: sortEntities(items) })),
  ];
}

export function selectedExportEntities(categories: CategoryDto[], selected: Set<string>) {
  return [...new Map(categories.flatMap((category) => category.entities)
    .filter((entity) => selected.has(entity.id))
    .map((entity) => [entity.id, entity])).values()];
}
