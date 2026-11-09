"""Import entity light frames and apply authored dimensions/shadows for all lights."""
from dataclasses import replace


def apply_authored_light_fields(light_data, light):
    """Override legacy sizing only when the package supplies the authored field."""
    raw = getattr(light, "raw", {})
    # Shadows substitute for CryEngine vis-area containment in Blender.
    shadow = True if raw.get("affects_this_area_only") else raw.get("shadow_casting")
    if shadow is not None and hasattr(light_data, "use_shadow"):
        light_data.use_shadow = bool(shadow)
    bulb = raw.get("bulb_radius_m")
    if bulb is not None and hasattr(light_data, "shadow_soft_size"):
        light_data.shadow_soft_size = float(bulb)
    if light_data.type == "AREA":
        for key, attribute in (("plane_width_m", "size"), ("plane_height_m", "size_y")):
            value = raw.get(key)
            if value is not None and hasattr(light_data, attribute):
                light_data.shape = "RECTANGLE"
                setattr(light_data, attribute, float(value))


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
