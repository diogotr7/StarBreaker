//! Emit a material contract: an authored `.mtl` joined against its shader
//! family's reflected interface, read from the user's own `Data.p4k`.
//!
//! Every field carries an explicit status so consumers never mistake a guess
//! for a fact:
//!   - `exact`                    — proven by reflection or a name match
//!   - `dead`                     — authored but compiled out of this build
//!   - `default-required`         — read by the shader, never authored
//!   - `needs-layer-descriptor`    — the mapping runs through a layered material
//!   - `unmatched`                — no binding found for the semantic role
//!
//! ```text
//! cargo run -p starbreaker-shadercache --example material_map -- \
//!     "C:\...\Data.p4k" "Data\Objects\...\aegs_gladius_a.mtl" HardSurface out.json
//! ```

use serde_json::{Value, json};
use starbreaker_p4k::MappedP4k;
use starbreaker_shadercache::{EXT_PAK, GenMasks, ResourceClass, ShaderPak, Stage, cfxb, read_pak_entries};

/// Normalise a name for matching: case-insensitive, separators removed.
fn norm(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn role_tokens(role: starbreaker_3d::mtl::TextureSemanticRole) -> &'static [&'static str] {
    use starbreaker_3d::mtl::TextureSemanticRole as R;
    match role {
        R::BaseColor | R::AlternateBaseColor => &["diffuse", "basecolor", "albedo"],
        R::NormalGloss => &["normal", "gloss"],
        R::Height => &["height"],
        R::BlendMask => &["blend"],
        R::WearMask => &["wear"],
        R::Iridescence => &["iridescence"],
        R::Stencil => &["stencil"],
        _ => &[],
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let p4k_path = args
        .next()
        .expect("usage: material_map <Data.p4k> <mtl> [family] [out.json]");
    let mtl_path = args.next().expect("mtl path");
    let want_family = args.next().filter(|s| !s.is_empty());
    let out_path = args.next();

    let p4k = MappedP4k::open(&p4k_path)?;
    let bytes = p4k.read_file(&mtl_path)?;
    let mtl = starbreaker_3d::mtl::parse_mtl(&bytes)?;
    let sub = mtl
        .materials
        .iter()
        .filter(|m| m.shader != "NoDraw")
        .filter(|m| want_family.as_deref().is_none_or(|w| m.shader.contains(w)))
        .max_by_key(|m| m.public_params.len())
        .or_else(|| mtl.materials.first())
        .ok_or("material file has no submaterials")?;

    let family = sub.shader_family();
    let engine_family = family.as_str();

    // Flag -> bit table, from the `.ext` files shipped in the p4k.
    let decoded = sub.decoded_string_gen_mask();
    let ext = read_pak_entries(&p4k_path, EXT_PAK)?;
    let ext_files: Vec<(String, String)> = ext
        .iter()
        .map(|(name, data)| (name.clone(), String::from_utf8_lossy(data).into_owned()))
        .collect();
    let masks = GenMasks::from_ext_files(ext_files.iter().map(|(n, t)| (n.as_str(), t.as_str())));
    let genmask = masks
        .family(engine_family)
        .ok_or_else(|| format!("no .ext for shader family {}", engine_family))?;
    let (want, unknown_flags) = genmask.mask_for(&decoded.tokens);

    // Union the interface across stages.
    let pak = ShaderPak::open_in_p4k(&p4k_path)?;
    let mut uniforms: Vec<(String, String, Option<u32>, ResourceClass, Stage)> = Vec::new();
    let mut textures: Vec<(String, String)> = Vec::new();
    let mut permutations = serde_json::Map::new();
    let mut seen_uniform = std::collections::BTreeSet::new();
    let mut seen_texture = std::collections::BTreeSet::new();

    // Pixel anchors the material pass; its vertex shader must link to it.
    // The material-buffer compute pass stands alone.
    let pixel = pak.select_stage(engine_family, Stage::Pixel, want)?;
    let vertex = match pixel {
        Some((ps, ..)) => pak.select_linked(engine_family, Stage::Vertex, want, ps)?,
        None => None,
    };
    let compute = pak.select_stage(engine_family, Stage::Compute, want)?;
    for (stage, chosen) in [(Stage::Pixel, pixel), (Stage::Vertex, vertex), (Stage::Compute, compute)] {
        let Some((perm, missing, extra)) = chosen else {
            continue;
        };
        permutations.insert(
            format!("{stage:?}"),
            json!({
                "entry": perm.entry,
                "gl": format!("{:#x}", perm.masks.gl.unwrap_or(0)),
                "missing_bits": missing,
                "extra_bits": extra,
            }),
        );
        let reflection = pak.reflect(perm)?;
        for class in [
            ResourceClass::PerMaterial,
            ResourceClass::PerView,
            ResourceClass::PerMesh,
            ResourceClass::PerObject,
        ] {
            for block in reflection.uniform_blocks(class) {
                for m in &block.members {
                    if seen_uniform.insert(norm(&m.name)) {
                        uniforms.push((m.name.clone(), m.ty.clone(), m.offset, class, stage));
                    }
                }
            }
        }
        for b in reflection.textures(ResourceClass::PerMaterial) {
            if seen_texture.insert(norm(&b.name)) {
                textures.push((b.name.clone(), b.ty.clone()));
            }
        }
    }

    // Parameters: authored -> uniform.
    let mut used = vec![false; uniforms.len()];
    let mut parameters = Vec::new();
    for param in &sub.public_params {
        let key = norm(&param.name);
        let hit = uniforms.iter().position(|(name, _, _, _, _)| norm(name) == key);
        let entry = match hit {
            Some(i) => {
                used[i] = true;
                let (name, ty, offset, class, stage) = &uniforms[i];
                json!({
                    "name": param.name,
                    "value": param.value,
                    "status": "exact",
                    "maps_to": {
                        "uniform": name, "type": ty, "offset": offset,
                        "class": format!("{class:?}"), "stage": format!("{stage:?}"),
                    }
                })
            }
            None => json!({
                "name": param.name,
                "value": param.value,
                "status": "dead",
                "maps_to": Value::Null,
            }),
        };
        parameters.push(entry);
    }

    let unset: Vec<Value> = uniforms
        .iter()
        .enumerate()
        .filter(|(i, (_, _, _, class, _))| !used[*i] && *class == ResourceClass::PerMaterial)
        .map(|(_, (name, ty, offset, _, stage))| {
            json!({
                "uniform": name, "type": ty, "offset": offset,
                "stage": format!("{stage:?}"), "status": "default-required",
            })
        })
        .collect();

    let layered = !sub.layers.is_empty();
    let texture_bindings: Vec<Value> = sub
        .semantic_texture_slots()
        .iter()
        .map(|slot| {
            let tokens = role_tokens(slot.role);
            let hit = textures
                .iter()
                .find(|(t, _)| {
                    let normalized = norm(t);
                    tokens.iter().any(|k| normalized.contains(k))
                })
                .map(|(t, _)| t.clone());
            match hit {
                Some(binding) => json!({
                    "slot": slot.slot, "role": slot.role.as_str(),
                    "binding": binding, "status": "exact",
                }),
                None => json!({
                    "slot": slot.slot, "role": slot.role.as_str(),
                    "binding": Value::Null,
                    "status": if layered { "needs-layer-descriptor" } else { "unmatched" },
                }),
            }
        })
        .collect();

    let layer_descriptor = uniforms
        .iter()
        .find(|(name, ..)| name == "Primary")
        .map(|(name, ty, offset, ..)| {
            json!({ "uniform": name, "type": ty, "offset": offset })
        });

    // Layer entries -> per-layer binding sets. The shader declares one binding
    // group per layer (`Primary_Diffuse`, `Wear_Normal`, …); the layer name in
    // the `.mtl` matches the binding prefix. The set of layer/role bindings is
    // taken from the family's own `.cfxb` string table, so nothing is guessed.
    let layer_roles = ["Diffuse", "Normal", "Gloss", "Blend", "Detail", "Specular"];
    let mut layer_prefixes: Vec<String> = Vec::new();
    let mut layer_bindings: std::collections::BTreeSet<String> = Default::default();
    if let Ok(bytes) = pak.read_shader_bin(engine_family)
        && let Ok(bin) = cfxb::parse(&bytes)
    {
        for name in bin.table.values() {
            if let Some((prefix, role)) = name.rsplit_once('_')
                && !prefix.is_empty()
                && layer_roles.contains(&role)
            {
                layer_bindings.insert(name.clone());
                if !layer_prefixes.iter().any(|p| p == prefix) {
                    layer_prefixes.push(prefix.to_owned());
                }
            }
        }
    }

    let layers: Vec<Value> = sub
        .layers
        .iter()
        .enumerate()
        .map(|(i, layer)| {
            // Match the authored layer name against the shader's layer prefixes;
            // fall back to layer order when the names differ.
            let prefix = layer_prefixes
                .iter()
                .find(|p| p.eq_ignore_ascii_case(&layer.name))
                .cloned()
                .or_else(|| layer_prefixes.get(i).cloned());

            let mut bindings = serde_json::Map::new();
            if let Some(prefix) = &prefix {
                for role in layer_roles {
                    let name = format!("{prefix}_{role}");
                    if layer_bindings.contains(&name) {
                        bindings.insert(
                            role.to_ascii_lowercase(),
                            json!({ "binding": name, "status": "exact" }),
                        );
                    }
                }
            }
            let status = if bindings.is_empty() {
                "needs-layer-descriptor"
            } else {
                "exact"
            };
            json!({
                "index": i,
                "name": layer.name,
                "binding_prefix": prefix,
                "mtl": layer.path,
                "sub_material": layer.sub_material,
                "tint_color": layer.tint_color,
                "wear_tint": layer.wear_tint,
                "palette_channel": layer.palette_tint,
                "gloss_mult": layer.gloss_mult,
                "wear_gloss": layer.wear_gloss,
                "uv_tiling": layer.uv_tiling,
                "height_scale": layer.height_scale,
                "height_bias": layer.height_bias,
                "bindings": Value::Object(bindings),
                "status": status,
            })
        })
        .collect();

    let available_texture_bindings: Vec<Value> = textures
        .iter()
        .map(|(name, ty)| json!({ "name": name, "type": ty }))
        .collect();

    let doc = json!({
        "schema": "starbreaker.material-map/1",
        "source": { "mtl": mtl_path, "p4k_build": p4k.path().display().to_string() },
        "material": { "name": sub.name, "shader": sub.shader, "family": engine_family },
        "flags": {
            "tokens": decoded.tokens,
            "mask": format!("{want:#x}"),
            "unmapped": unknown_flags,
        },
        "permutations": permutations,
        "layer_count": sub.layers.len(),
        "layer_descriptor": layer_descriptor,
        "parameters": parameters,
        "unset_per_material": unset,
        "textures": texture_bindings,
        "layers": layers,
        "available_texture_bindings": available_texture_bindings,
    });

    let exact = doc["parameters"]
        .as_array()
        .map(|a| a.iter().filter(|p| p["status"] == "exact").count())
        .unwrap_or(0);
    println!(
        "{} ({}): {exact}/{} params exact, {} dead, {} unset, {} textures ({} layered)",
        sub.name,
        engine_family,
        sub.public_params.len(),
        doc["parameters"]
            .as_array()
            .map(|a| a.iter().filter(|p| p["status"] == "dead").count())
            .unwrap_or(0),
        unset.len(),
        texture_bindings.len(),
        sub.layers.len(),
    );

    if let Some(path) = out_path {
        std::fs::write(&path, serde_json::to_string_pretty(&doc)?)?;
        println!("wrote {path}");
    } else {
        println!("{}", serde_json::to_string_pretty(&doc)?);
    }
    Ok(())
}








