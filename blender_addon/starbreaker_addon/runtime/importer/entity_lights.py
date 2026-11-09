"""Import resolved entity lights in their owning instance or animated helper frame."""
from dataclasses import replace


def import_entity_lights(importer, record, anchor, nodes):
    node_index = {name.casefold(): obj for name, obj in importer._index_nodes(nodes).items() if name}
    for light in record.lights:
        parent = node_index.get((light.parent_node_name or "").casefold())
        if parent is not None and light.parent_local_position is not None and light.parent_local_rotation is not None:
            # The exported direction is entity-local. After attaching to a helper,
            # let create_light derive direction from the helper-local quaternion.
            local_light = replace(light, position=light.parent_local_position,
                                  rotation=light.parent_local_rotation, direction_sc=None)
            importer.create_light(local_light, parent)
        else:
            importer.create_light(light, anchor)
