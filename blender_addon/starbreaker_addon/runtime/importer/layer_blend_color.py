from __future__ import annotations


PRINCIPLED_DIELECTRIC_F0 = ((1.5 - 1.0) / (1.5 + 1.0)) ** 2


def layer_finish_values(layer):
    snapshot = layer.layer_snapshot
    diffuse = tuple(float(v) for v in snapshot.get("diffuse", (1.0, 1.0, 1.0)))
    specular = tuple(float(v) for v in snapshot.get("specular", (.04, .04, .04)))
    metallic = max(0.0, min(1.0, float(snapshot.get("metallic", 0.0))))
    return diffuse, specular, metallic


def _connect(node, target, value):
    if isinstance(value, (tuple, list)):
        target.default_value = (*value[:3], 1.0)
    elif isinstance(value, (int, float)):
        target.default_value = value
    else:
        node.id_data.links.new(value, target)


def _color_mix(nodes, factor, first, second, *, multiply=False):
    node = nodes.new("ShaderNodeMixRGB")
    node.blend_type = "MULTIPLY" if multiply else "MIX"
    node.label = "StarBreaker LayerBlend Color"
    for socket, value in zip(node.inputs, (factor, first, second)):
        _connect(node, socket, value)
    return node.outputs[0]


def _float_mix(nodes, factor, first, second):
    node = nodes.new("ShaderNodeMix")
    node.data_type = "FLOAT"
    node.label = "StarBreaker LayerBlend Metallic"
    for socket, value in zip((node.inputs[0], node.inputs[2], node.inputs[3]), (factor, first, second)):
        _connect(node, socket, value)
    return node.outputs[0]


def _layer_color(importer, nodes, layer, palette, row):
    diffuse, specular, metallic = layer_finish_values(layer)
    tint = layer.tint_color if layer.tint_color is not None else (1.0, 1.0, 1.0)
    color = _color_mix(nodes, 1.0, diffuse, tint, multiply=True)
    if layer.palette_channel is not None and palette is not None:
        palette_color = importer._palette_color_socket(nodes, palette, layer.palette_channel.name, x=-1600, y=row)
        color = _color_mix(nodes, 1.0, color, palette_color, multiply=True)
    image = importer._image_node(nodes, layer.diffuse_export_path, x=-1400, y=row, is_color=True)
    if image is not None:
        importer._apply_uv_tiling(nodes, image.id_data.links, image,
            layer.uv_tiling if layer.uv_tiling is not None else 1.0, x=-1800, y=row)
        color = _color_mix(nodes, 1.0, color, image.outputs[0], multiply=True)
    color = _color_mix(nodes, metallic, color, specular)
    specular_tint = tuple(value / PRINCIPLED_DIELECTRIC_F0 for value in specular)
    specular_tint = _color_mix(nodes, metallic, specular_tint, (1.0, 1.0, 1.0))
    return color, specular_tint, metallic


def _blend_layers(importer, nodes, layers, channels, palette, row):
    values = [_layer_color(importer, nodes, layer, palette, row - i * 220) if layer is not None else None for i, layer in enumerate(layers)]
    color, specular, metallic = values[3]
    for index in (2, 1, 0):
        if values[index] is None:
            continue
        color = _color_mix(nodes, channels[index], color, values[index][0])
        specular = _color_mix(nodes, channels[index], specular, values[index][1])
        metallic = _float_mix(nodes, channels[index], metallic, values[index][2])
    return color, specular, metallic


def apply_layer_blend_color(importer, nodes, submaterial, palette, wear_factor, principled):
    if submaterial.shader_family != "LayerBlend_V2":
        return False
    indexed = {layer.name: layer for layer in submaterial.layer_manifest}
    base_layers = [indexed.get(f"BaseLayer{i}") for i in range(1, 5)]
    wear_layers = [indexed.get(f"WearLayer{i}") for i in range(1, 5)]
    if base_layers[3] is None:
        return False
    mask = next((t for t in submaterial.texture_slots if t.role == "blend_mask" and t.export_path), None)
    if mask is None:
        return False
    image = importer._image_node(nodes, mask.export_path, x=-1800, y=-500, is_color=False)
    if image is None:
        return False
    separate = nodes.new("ShaderNodeSeparateColor")
    separate.mode = "RGB"
    separate.id_data.links.new(image.outputs[0], separate.inputs[0])
    importer._apply_uv_tiling(nodes, separate.id_data.links, image,
        float(submaterial.public_params.get("BlendMapTiling", 1.0)), x=-2000, y=-500)
    color, specular, metallic = _blend_layers(importer, nodes, base_layers, separate.outputs, palette, -2200)
    if wear_factor is not None and wear_layers[3] is not None:
        wear_color, wear_specular, wear_metallic = _blend_layers(importer, nodes, wear_layers, separate.outputs, palette, -3200)
        color = _color_mix(nodes, wear_factor, color, wear_color)
        specular = _color_mix(nodes, wear_factor, specular, wear_specular)
        metallic = _float_mix(nodes, wear_factor, metallic, wear_metallic)
    for name, socket in (("Base Color", color), ("Specular Tint", specular), ("Metallic", metallic)):
        principled.id_data.links.new(socket, principled.inputs[name])
    return True
