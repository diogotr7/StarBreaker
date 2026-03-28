use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Subcommand;
use serde_json::Value;

#[derive(Subcommand)]
pub enum GlbCommand {
    /// Inspect a GLB file: node tree, meshes, materials, lights
    Inspect {
        /// Input .glb file
        input: PathBuf,
        /// Show full node tree (default: summary only)
        #[arg(long)]
        tree: bool,
    },
}

impl GlbCommand {
    pub fn run(self) -> Result<()> {
        match self {
            Self::Inspect { input, tree } => inspect(&input, tree),
        }
    }
}

fn inspect(path: &std::path::Path, show_tree: bool) -> Result<()> {
    let data = std::fs::read(path).context("failed to read GLB file")?;

    // Parse GLB container: magic(4) + version(4) + length(4) + json_chunk_len(4) + json_chunk_type(4) + json
    if data.len() < 20 {
        anyhow::bail!("file too small for GLB");
    }
    let magic = u32::from_le_bytes(data[0..4].try_into().unwrap());
    if magic != 0x46546C67 {
        anyhow::bail!("not a GLB file (bad magic)");
    }
    let _version = u32::from_le_bytes(data[4..8].try_into().unwrap());
    let _total_len = u32::from_le_bytes(data[8..12].try_into().unwrap());
    let json_len = u32::from_le_bytes(data[12..16].try_into().unwrap()) as usize;
    let _json_type = u32::from_le_bytes(data[16..20].try_into().unwrap());

    let json_bytes = &data[20..20 + json_len];
    let root: Value = serde_json::from_slice(json_bytes).context("failed to parse GLB JSON")?;

    let bin_offset = 20 + json_len;
    let bin_size = if bin_offset + 8 <= data.len() {
        u32::from_le_bytes(data[bin_offset..bin_offset + 4].try_into().unwrap()) as usize
    } else {
        0
    };

    // Summary
    let nodes = root["nodes"].as_array();
    let meshes = root["meshes"].as_array();
    let materials = root["materials"].as_array();
    let accessors = root["accessors"].as_array();
    let images = root["images"].as_array();
    let textures = root["textures"].as_array();

    let node_count = nodes.map(|n| n.len()).unwrap_or(0);
    let mesh_count = meshes.map(|m| m.len()).unwrap_or(0);
    let material_count = materials.map(|m| m.len()).unwrap_or(0);
    let accessor_count = accessors.map(|a| a.len()).unwrap_or(0);
    let image_count = images.map(|i| i.len()).unwrap_or(0);
    let texture_count = textures.map(|t| t.len()).unwrap_or(0);

    // Lights from KHR_lights_punctual
    let lights = root["extensions"]["KHR_lights_punctual"]["lights"].as_array();
    let light_count = lights.map(|l| l.len()).unwrap_or(0);

    println!("=== GLB Summary ===");
    println!("File:       {}", path.display());
    println!("Binary:     {}", format_size(bin_size));
    println!("Nodes:      {node_count}");
    println!("Meshes:     {mesh_count}");
    println!("Materials:  {material_count}");
    println!("Textures:   {texture_count}");
    println!("Images:     {image_count}");
    println!("Accessors:  {accessor_count}");
    println!("Lights:     {light_count}");
    println!();

    // Mesh stats
    if let Some(meshes) = meshes {
        let total_prims: usize = meshes
            .iter()
            .map(|m| m["primitives"].as_array().map(|p| p.len()).unwrap_or(0))
            .sum();
        println!("=== Meshes ({mesh_count} meshes, {total_prims} primitives) ===");

        // Count vertices via POSITION accessor
        let mut total_verts = 0u64;
        for mesh in meshes {
            let name = mesh["name"].as_str().unwrap_or("?");
            let prims = mesh["primitives"].as_array();
            let mut mesh_verts = 0u64;
            if let Some(prims) = prims {
                for prim in prims {
                    if let Some(pos_idx) = prim["attributes"]["POSITION"].as_u64() {
                        if let Some(acc) = accessors.and_then(|a| a.get(pos_idx as usize)) {
                            mesh_verts += acc["count"].as_u64().unwrap_or(0);
                        }
                    }
                }
            }
            total_verts += mesh_verts;
            if show_tree {
                println!("  {name}: {mesh_verts} verts, {} prims",
                    prims.map(|p| p.len()).unwrap_or(0));
            }
        }
        println!("Total vertices: {total_verts}");
        println!();
    }

    // Materials
    if let Some(materials) = materials {
        println!("=== Materials ({material_count}) ===");
        for (i, mat) in materials.iter().enumerate() {
            let name = mat["name"].as_str().unwrap_or("unnamed");
            let mut tex_info = Vec::new();

            // Check common texture slots
            let tex_slots: &[(&str, &[&str])] = &[
                ("diffuse", &["pbrMetallicRoughness", "baseColorTexture", "index"]),
                ("metalRough", &["pbrMetallicRoughness", "metallicRoughnessTexture", "index"]),
                ("normal", &["normalTexture", "index"]),
                ("emissive", &["emissiveTexture", "index"]),
                ("occlusion", &["occlusionTexture", "index"]),
            ];
            for (slot, path) in tex_slots {
                let val = path.iter().fold(Some(mat), |acc: Option<&Value>, key| {
                    acc.and_then(|v| v.get(*key))
                });
                if let Some(idx) = val.and_then(|v| v.as_u64()) {
                    tex_info.push(format!("{slot}={idx}"));
                }
            }

            // Check extensions
            let has_transmission = mat.get("extensions")
                .and_then(|e| e.get("KHR_materials_transmission"))
                .is_some();
            let has_ior = mat.get("extensions")
                .and_then(|e| e.get("KHR_materials_ior"))
                .is_some();

            let mut extras = Vec::new();
            if has_transmission { extras.push("transmission"); }
            if has_ior { extras.push("ior"); }

            let tex_str = if tex_info.is_empty() {
                String::new()
            } else {
                format!(" [{}]", tex_info.join(", "))
            };
            let ext_str = if extras.is_empty() {
                String::new()
            } else {
                format!(" +{}", extras.join("+"))
            };
            println!("  [{i:3}] {name}{tex_str}{ext_str}");
        }
        println!();
    }

    // Images with sizes
    if let Some(images) = images {
        println!("=== Images ({image_count}) ===");
        let buffer_views = root["bufferViews"].as_array();
        let mut total_img_bytes = 0usize;
        for (i, img) in images.iter().enumerate() {
            let name = img["name"].as_str().unwrap_or("");
            let mime = img["mimeType"].as_str().unwrap_or("?");
            let size = img["bufferView"]
                .as_u64()
                .and_then(|bv_idx| buffer_views?.get(bv_idx as usize))
                .and_then(|bv| bv["byteLength"].as_u64())
                .unwrap_or(0) as usize;
            total_img_bytes += size;
            if show_tree {
                println!("  [{i:3}] {name} ({mime}, {})", format_size(size));
            }
        }
        println!("Total image data: {}", format_size(total_img_bytes));
        println!();
    }

    // Lights
    if let Some(lights) = lights {
        println!("=== Lights ({light_count}) ===");

        // Also find light node positions
        let light_nodes: Vec<_> = nodes
            .map(|ns| {
                ns.iter()
                    .filter_map(|n| {
                        let light_idx = n.get("extensions")?
                            .get("KHR_lights_punctual")?
                            .get("light")?
                            .as_u64()?;
                        Some((light_idx as usize, n))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        for (i, light) in lights.iter().enumerate() {
            let name = light["name"].as_str().unwrap_or("unnamed");
            let type_ = light["type"].as_str().unwrap_or("?");
            let intensity = light["intensity"].as_f64().unwrap_or(0.0);
            let range = light["range"].as_f64();
            let color = light["color"].as_array().map(|c| {
                format!(
                    "[{:.2}, {:.2}, {:.2}]",
                    c.first().and_then(|v| v.as_f64()).unwrap_or(0.0),
                    c.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0),
                    c.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0),
                )
            });

            // Find corresponding node position
            let pos = light_nodes.iter().find(|(idx, _)| *idx == i).map(|(_, n)| {
                let t = n["translation"].as_array();
                t.map(|t| {
                    format!(
                        "[{:.2}, {:.2}, {:.2}]",
                        t.first().and_then(|v| v.as_f64()).unwrap_or(0.0),
                        t.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0),
                        t.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0),
                    )
                })
                .unwrap_or_else(|| "[matrix]".to_string())
            });

            let range_str = range.map(|r| format!(" range={r:.1}")).unwrap_or_default();
            let color_str = color.map(|c| format!(" color={c}")).unwrap_or_default();
            let pos_str = pos.map(|p| format!(" pos={p}")).unwrap_or_default();

            println!(
                "  [{i:3}] {name}: {type_} intensity={intensity:.0}{range_str}{color_str}{pos_str}"
            );
        }
        println!();
    }

    // Node tree (optional)
    if show_tree {
        println!("=== Node Tree ===");
        if let Some(scenes) = root["scenes"].as_array() {
            for scene in scenes {
                if let Some(roots) = scene["nodes"].as_array() {
                    for root_idx in roots {
                        if let Some(idx) = root_idx.as_u64() {
                            print_node_tree(nodes.unwrap(), idx as usize, 0);
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

fn print_node_tree(nodes: &[Value], idx: usize, depth: usize) {
    let node = &nodes[idx];
    let indent = "  ".repeat(depth);
    let name = node["name"].as_str().unwrap_or("?");

    let mut info = Vec::new();
    if node.get("mesh").is_some() {
        info.push("M".to_string());
    }
    if node.get("extensions").and_then(|e| e.get("KHR_lights_punctual")).is_some() {
        info.push("L".to_string());
    }
    if node.get("matrix").is_some() {
        info.push("xform".to_string());
    } else {
        if node.get("translation").is_some() {
            info.push("T".to_string());
        }
        if node.get("rotation").is_some() {
            info.push("R".to_string());
        }
        if node.get("scale").is_some() {
            info.push("S".to_string());
        }
    }

    let info_str = if info.is_empty() {
        String::new()
    } else {
        format!(" [{}]", info.join(","))
    };

    println!("{indent}{name}{info_str}");

    if let Some(children) = node["children"].as_array() {
        for child in children {
            if let Some(child_idx) = child.as_u64() {
                print_node_tree(nodes, child_idx as usize, depth + 1);
            }
        }
    }
}

fn format_size(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}
