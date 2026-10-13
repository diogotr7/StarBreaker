from array import array


DECAL_COORDINATES = "starbreaker_decal_coordinates"
DECAL_LABEL = "StarBreaker LayerBlend Diffuse Decal"


def decode_decal_coordinates(rgba):
    r, g, b = (int(value * 255 + 0.5) for value in rgba[:3])
    packed = (r << 16) | (g << 8) | b
    return (((packed >> 12) & 4095) / 4095 * 4 - 0.5,
            (packed & 4095) / 4095 * 4 - 0.5, 0.0)


def diffuse_decal_texture(submaterial):
    if submaterial.shader_family != "LayerBlend_V2":
        return None
    tokens = {t.upper() for t in submaterial.decoded_feature_flags.tokens}
    if "DECALS" not in tokens or "WEAR_DIRT_AO_VERTEX" in tokens:
        return None
    return next((t for t in submaterial.texture_slots if t.slot == "TexSlot9" and t.export_path), None)


def prepare_decal_coordinates(obj, sidecar):
    if not any(diffuse_decal_texture(s) is not None for s in sidecar.submaterials):
        return False
    mesh = obj.data
    colors = mesh.color_attributes.get("Color")
    if colors is None:
        return False
    if mesh.library is not None:
        mesh = mesh.copy()
        obj.data = mesh
        colors = mesh.color_attributes["Color"]
    values = array("f", [0]) * (len(colors.data) * 4)
    colors.data.foreach_get("color_srgb", values)
    coords = array("f")
    for i in range(0, len(values), 4):
        coords.extend(decode_decal_coordinates(values[i:i + 4]))
    attribute = mesh.attributes.get(DECAL_COORDINATES)
    if attribute is None:
        attribute = mesh.attributes.new(DECAL_COORDINATES, "FLOAT_VECTOR", colors.domain)
    attribute.data.foreach_set("vector", coords)
    return True


def _connect(node, target, value):
    if isinstance(value, (int, float, tuple, list)):
        target.default_value = value
    else:
        node.id_data.links.new(value, target)


def _math(nodes, operation, first, second=0.0):
    node = nodes.new("ShaderNodeMath")
    node.operation = operation
    node.label = DECAL_LABEL
    _connect(node, node.inputs[0], first)
    _connect(node, node.inputs[1], second)
    return node.outputs[0]


def _mix(nodes, factor, first, second):
    node = nodes.new("ShaderNodeMixRGB")
    node.label = DECAL_LABEL
    for socket, value in zip(node.inputs[:3], (factor, first, second)):
        _connect(node, socket, value)
    return node.outputs[0]


def apply_layer_blend_decals(importer, nodes, submaterial, wear_factor, surface):
    texture = diffuse_decal_texture(submaterial)
    if texture is None:
        return False
    image = importer._image_node(nodes, texture.export_path, x=-800, y=-4200, is_color=True)
    if image is None:
        return False
    image.label = DECAL_LABEL
    image.image.alpha_mode = "STRAIGHT"
    attribute = nodes.new("ShaderNodeAttribute")
    attribute.attribute_name = DECAL_COORDINATES
    separate = nodes.new("ShaderNodeSeparateXYZ")
    links = surface.id_data.links
    links.new(attribute.outputs["Vector"], separate.inputs[0])
    u, v = separate.outputs[0], separate.outputs[1]
    diffuse_region = _math(nodes, "MULTIPLY", _math(nodes, "GREATER_THAN", u, 2),
                           _math(nodes, "SUBTRACT", 1, _math(nodes, "GREATER_THAN", u, 3)))
    stencil_region = _math(nodes, "MULTIPLY", _math(nodes, "GREATER_THAN", v, 2),
                           _math(nodes, "SUBTRACT", 1, _math(nodes, "GREATER_THAN", v, 3)))
    region = _math(nodes, "MULTIPLY", diffuse_region, _math(nodes, "SUBTRACT", 1, stencil_region))
    vector = nodes.new("ShaderNodeCombineXYZ")
    links.new(_math(nodes, "SUBTRACT", u, 2), vector.inputs[0])
    links.new(_math(nodes, "SUBTRACT", 1, v), vector.inputs[1])
    links.new(vector.outputs[0], image.inputs["Vector"])
    alpha = _math(nodes, "MULTIPLY", image.outputs["Alpha"], region)
    if wear_factor is not None:
        alpha = _math(nodes, "MULTIPLY", alpha, _math(nodes, "SUBTRACT", 1, wear_factor))
    base = surface.inputs["Base Color"]
    first = base.links[0].from_socket if base.is_linked else tuple(base.default_value)
    links.new(_mix(nodes, alpha, first, image.outputs["Color"]), base)
    return True
