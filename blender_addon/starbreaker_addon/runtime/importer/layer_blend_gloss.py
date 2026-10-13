from __future__ import annotations


def _math(nodes, operation, first, second=0.0, *, clamp=False):
    node = nodes.new("ShaderNodeMath")
    node.operation = operation
    node.use_clamp = clamp
    node.label = "StarBreaker LayerBlend Gloss"
    for target, source in zip(node.inputs, (first, second)):
        if isinstance(source, (int, float)):
            target.default_value = source
        else:
            node.id_data.links.new(source, target)
    return node.outputs[0]


def _layer_smoothness(importer, nodes, layer, row):
    texture = layer.roughness_texture
    path = (texture.export_path if texture else None) or layer.roughness_export_path
    is_smoothness = texture is not None and texture.alpha_semantic == "smoothness"
    if not path:
        texture = next((t for t in layer.texture_slots if t.alpha_semantic == "smoothness" and t.export_path), None)
        path = texture.export_path if texture else None
        is_smoothness = texture is not None
    image = importer._image_node(nodes, path, x=-1100, y=row, is_color=False)
    smoothness = 1.0
    if image is not None:
        importer._apply_uv_tiling(nodes, image.id_data.links,
            image, layer.uv_tiling if layer.uv_tiling is not None else 1.0, x=-1300, y=row)
        if is_smoothness:
            smoothness = image.outputs[1]
        else:
            squared = _math(nodes, "MULTIPLY", image.outputs[0], image.outputs[0])
            smoothness = _math(nodes, "SUBTRACT", 1.0, squared, clamp=True)
    gloss = layer.gloss_mult if layer.gloss_mult is not None else 1.0
    shininess = layer.layer_snapshot.get("shininess")
    if shininess is not None:
        gloss *= max(0.0, min(1.0, float(shininess) / 255.0))
    return _math(nodes, "MULTIPLY", smoothness, gloss, clamp=True)


def _blend(importer, nodes, layers, channels, row):
    values = [_layer_smoothness(importer, nodes, layer, row - index * 180) if layer is not None else None for index, layer in enumerate(layers)]
    result = values[3]
    for index in (2, 1, 0):
        if values[index] is None:
            continue
        mix = nodes.new("ShaderNodeMix")
        mix.data_type = "FLOAT"
        mix.label = "StarBreaker LayerBlend Gloss Mask"
        links = mix.id_data.links
        links.new(channels[index], mix.inputs[0])
        links.new(result, mix.inputs[2])
        links.new(values[index], mix.inputs[3])
        result = mix.outputs[0]
    return importer._invert_value_socket(nodes, result, x=-100, y=row)


def build_layer_blend_roughness(importer, nodes, submaterial):
    if submaterial.shader_family != "LayerBlend_V2":
        return None
    indexed = {layer.name: layer for layer in submaterial.layer_manifest}
    base_layers = [indexed.get(f"BaseLayer{i}") for i in range(1, 5)]
    wear_layers = [indexed.get(f"WearLayer{i}") for i in range(1, 5)]
    if base_layers[3] is None:
        return None
    mask = next((t for t in submaterial.texture_slots if t.role == "blend_mask" and t.export_path), None)
    if mask is None:
        return None
    image = importer._image_node(nodes, mask.export_path, x=-1500, y=-500, is_color=False)
    if image is None:
        return None
    separate = nodes.new("ShaderNodeSeparateColor")
    separate.mode = "RGB"
    separate.id_data.links.new(image.outputs[0], separate.inputs[0])
    importer._apply_uv_tiling(nodes, separate.id_data.links, image,
        float(submaterial.public_params.get("BlendMapTiling", 1.0)), x=-1700, y=-500)
    base = _blend(importer, nodes, base_layers, separate.outputs, -1000)
    wear = None
    if wear_layers[3] is not None:
        wear = _blend(importer, nodes, wear_layers, separate.outputs, -1800)
    return base, wear
