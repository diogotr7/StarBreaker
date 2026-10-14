//! Animation clips (`clip::Clip`) → glTF animations, one per clip.
//!
//! - Bone tracks drive the node of the same (case-insensitive) name: rotation
//!   (`[w, x, y, z]` → glTF `[x, y, z, w]`) and translation, linear. Animated
//!   nodes are switched from `matrix` to TRS, as glTF requires.
//! - Morph tracks drive `weights` of every mesh node whose mesh carries any
//!   of the targets; the mesh's targets are sampled at the union of the
//!   tracks' times (a target without a track holds 0).
//! - Material parameter tracks are written to the animation's extras
//!   (`starbreaker_material_params`): glTF cannot animate shader parameters.
//!
//! A clip that resolves to no channel is dropped with a warning (glTF
//! animations need one); an invalid clip is an error.

use gltf_json as json;
use json::validation::Checked;

use super::GlbBuilder;
use crate::clip::Clip;
use crate::error::Error;

impl GlbBuilder {
    /// Add one glTF animation per clip (see the module docs).
    pub(crate) fn add_animations(&mut self, clips: &[Clip]) -> Result<(), Error> {
        for clip in clips {
            if let Some(bad) = clip.invalid_track() {
                return Err(Error::Other(format!("animation '{}': invalid {bad}", clip.name)));
            }
            let mut channels = Vec::new();
            let mut samplers = Vec::new();
            let mut add = |builder: &mut Self, node: u32, path, times: &[f32], output: Vec<f32>, type_| {
                let input = builder.time_accessor(times);
                let output = builder.float_accessor(&output, type_);
                let sampler = samplers.len() as u32;
                samplers.push(json::animation::Sampler {
                    input: json::Index::new(input),
                    interpolation: Checked::Valid(json::animation::Interpolation::Linear),
                    output: json::Index::new(output),
                    extensions: None,
                    extras: Default::default(),
                });
                channels.push(json::animation::Channel {
                    sampler: json::Index::new(sampler),
                    target: json::animation::Target {
                        node: json::Index::new(node),
                        path: Checked::Valid(path),
                        extensions: None,
                        extras: Default::default(),
                    },
                    extensions: None,
                    extras: Default::default(),
                });
            };

            for track in &clip.bones {
                let Some(&node) = self.node_name_to_idx.get(&track.bone.to_lowercase()) else {
                    log::warn!("animation '{}': no node for bone '{}'", clip.name, track.bone);
                    continue;
                };
                self.use_trs(node);
                if let Some(keys) = &track.rotation {
                    let out = keys.values.iter().flat_map(|q| [q[1], q[2], q[3], q[0]]).collect();
                    add(self, node, json::animation::Property::Rotation, &keys.times, out, json::accessor::Type::Vec4);
                }
                if let Some(keys) = &track.translation {
                    let out = keys.values.iter().flatten().copied().collect();
                    add(self, node, json::animation::Property::Translation, &keys.times, out, json::accessor::Type::Vec3);
                }
            }

            if !clip.morphs.is_empty() {
                for node in 0..self.nodes_json.len() as u32 {
                    let Some(mesh) = self.nodes_json[node as usize].mesh else { continue };
                    let Some(targets) = self.morph_names.get(&(mesh.value() as u32)).cloned() else { continue };
                    let tracks: Vec<_> = targets.iter().map(|t| clip.morphs.iter().find(|m| &m.name == t)).collect();
                    if tracks.iter().all(Option::is_none) {
                        continue;
                    }
                    let mut times: Vec<f32> = tracks.iter().flatten().flat_map(|m| m.keys.times.iter().copied()).collect();
                    times.sort_by(f32::total_cmp);
                    times.dedup();
                    let out = times
                        .iter()
                        .flat_map(|&t| tracks.iter().map(move |m| m.map_or(0.0, |m| m.keys.sample(t))))
                        .collect();
                    add(self, node, json::animation::Property::MorphTargetWeights, &times, out, json::accessor::Type::Scalar);
                }
            }

            if channels.is_empty() {
                log::warn!("animation '{}' moves nothing in this scene; not written", clip.name);
                continue;
            }
            let extras = (!clip.material_params.is_empty()).then(|| {
                let params: Vec<serde_json::Value> = clip
                    .material_params
                    .iter()
                    .map(|p| {
                        serde_json::json!({
                            "shader": p.shader,
                            "param": p.param,
                            "info": p.info,
                            "times": p.keys.times,
                            "values": p.keys.values,
                        })
                    })
                    .collect();
                serde_json::value::to_raw_value(&serde_json::json!({ "starbreaker_material_params": params }))
            });
            self.animations_json.push(json::Animation {
                name: Some(clip.name.clone()),
                channels,
                samplers,
                extensions: None,
                extras: extras.transpose().map_err(|e| Error::Other(e.to_string()))?,
            });
        }
        Ok(())
    }

    /// A SCALAR f32 accessor of key times, with the min/max glTF requires.
    fn time_accessor(&mut self, times: &[f32]) -> u32 {
        let index = self.float_accessor(times, json::accessor::Type::Scalar);
        let accessor = &mut self.accessors[index as usize];
        accessor.min = Some(serde_json::json!([times.first().copied().unwrap_or(0.0)]));
        accessor.max = Some(serde_json::json!([times.last().copied().unwrap_or(0.0)]));
        index
    }

    fn float_accessor(&mut self, values: &[f32], type_: json::accessor::Type) -> u32 {
        let width = match type_ {
            json::accessor::Type::Vec4 => 4,
            json::accessor::Type::Vec3 => 3,
            _ => 1,
        };
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        let offset = self.push_aligned(&bytes);
        self.push_accessor(offset, bytes.len(), values.len() / width, json::accessor::ComponentType::F32, type_, None, None)
    }

    /// Replace a node's `matrix` with the equivalent TRS (animated nodes must
    /// not use `matrix`).
    fn use_trs(&mut self, node: u32) {
        let node = &mut self.nodes_json[node as usize];
        let Some(matrix) = node.matrix.take() else { return };
        let (scale, rotation, translation) = glam::Mat4::from_cols_array(&matrix).to_scale_rotation_translation();
        node.translation = Some(translation.to_array());
        node.rotation = Some(json::scene::UnitQuaternion(rotation.to_array()));
        node.scale = Some(scale.to_array());
    }
}
