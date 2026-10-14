//! Animation clips in the shared export model: what moves over time, by name,
//! independent of the writer (glTF animations, Blender actions).
//!
//! - [`BoneTrack`]: one bone's LOCAL (parent-relative) rotation `[w, x, y, z]`
//!   and translation keys in CryEngine space, by skeleton bone name.
//! - morph tracks: [`ScalarTrack`]s named after `Mesh.morph_targets`.
//! - [`MaterialParamTrack`]: a shader parameter vector over time (e.g.
//!   HumanSkin_V2's `PI_WrinklesMask`), with consumer-facing `info`.
//!
//! Times are seconds; [`Keys`] are sampled linearly ([`Keys::sample`]).

/// Keyframes: one value per time (seconds, ascending).
#[derive(Debug, Clone, PartialEq)]
pub struct Keys<T> {
    pub times: Vec<f32>,
    pub values: Vec<T>,
}

impl<T> Keys<T> {
    /// Times and values pair up and times ascend.
    pub fn is_valid(&self) -> bool {
        self.times.len() == self.values.len()
            && !self.times.is_empty()
            && self.times.windows(2).all(|w| w[0] <= w[1])
    }
}

impl Keys<f32> {
    /// Linear interpolation at `t`, held at the ends.
    pub fn sample(&self, t: f32) -> f32 {
        let i = self.times.partition_point(|&k| k <= t);
        match i {
            0 => self.values[0],
            n if n >= self.times.len() => self.values[self.values.len() - 1],
            n => {
                let (t0, t1) = (self.times[n - 1], self.times[n]);
                let f = if t1 > t0 { (t - t0) / (t1 - t0) } else { 0.0 };
                self.values[n - 1] + (self.values[n] - self.values[n - 1]) * f
            }
        }
    }
}

/// A bone's local TRS keys (CryEngine space).
#[derive(Debug, Clone, PartialEq)]
pub struct BoneTrack {
    pub bone: String,
    /// `[w, x, y, z]`, as `skeleton::Bone`.
    pub rotation: Option<Keys<[f32; 4]>>,
    pub translation: Option<Keys<[f32; 3]>>,
}

/// A named scalar over time (a morph-target weight).
#[derive(Debug, Clone, PartialEq)]
pub struct ScalarTrack {
    pub name: String,
    pub keys: Keys<f32>,
}

/// A shader parameter vector over time.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialParamTrack {
    /// Shader the parameter belongs to (e.g. `HumanSkin_V2 %WRINKLE_BLENDING_2`).
    pub shader: String,
    /// Parameter name (e.g. `PI_WrinklesMask`).
    pub param: String,
    /// What the components mean, for consumers (e.g. the wrinkle slot layout).
    pub info: serde_json::Value,
    pub keys: Keys<Vec<f32>>,
}

/// One animation clip.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Clip {
    pub name: String,
    pub bones: Vec<BoneTrack>,
    pub morphs: Vec<ScalarTrack>,
    pub material_params: Vec<MaterialParamTrack>,
}

impl Clip {
    /// Play `other` together with this clip: add its tracks for the bones,
    /// morphs and material parameters this clip does not animate (this
    /// clip's own track wins where both do). Each track keeps its own key
    /// times, so clips of different lengths combine (the shorter holds its
    /// last key).
    pub fn merge(&mut self, other: Clip) {
        let Clip { name: _, bones, morphs, material_params } = other;
        for track in bones {
            if !self.bones.iter().any(|b| b.bone == track.bone) {
                self.bones.push(track);
            }
        }
        for track in morphs {
            if !self.morphs.iter().any(|m| m.name == track.name) {
                self.morphs.push(track);
            }
        }
        for track in material_params {
            if !self.material_params.iter().any(|p| p.shader == track.shader && p.param == track.param) {
                self.material_params.push(track);
            }
        }
    }

    /// The first invalid track, if any (mismatched or unordered keys).
    pub fn invalid_track(&self) -> Option<String> {
        let bones = self.bones.iter().find_map(|b| {
            let bad = b.rotation.as_ref().is_some_and(|k| !k.is_valid())
                || b.translation.as_ref().is_some_and(|k| !k.is_valid());
            bad.then(|| format!("bone '{}'", b.bone))
        });
        let morphs = || self.morphs.iter().find(|m| !m.keys.is_valid()).map(|m| format!("morph '{}'", m.name));
        let params = || {
            self.material_params
                .iter()
                .find(|p| !p.keys.is_valid())
                .map(|p| format!("material parameter '{}'", p.param))
        };
        bones.or_else(morphs).or_else(params)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(times: &[f32], values: &[f32]) -> Keys<f32> {
        Keys { times: times.to_vec(), values: values.to_vec() }
    }

    #[test]
    fn sample_interpolates_and_holds_the_ends() {
        let k = keys(&[0.0, 1.0, 3.0], &[0.0, 1.0, 0.0]);
        assert_eq!(k.sample(-1.0), 0.0);
        assert!((k.sample(0.5) - 0.5).abs() < 1e-6);
        assert!((k.sample(2.0) - 0.5).abs() < 1e-6);
        assert_eq!(k.sample(1.0), 1.0);
        assert_eq!(k.sample(9.0), 0.0);
    }

    fn bone(name: &str, w: f32) -> BoneTrack {
        BoneTrack { bone: name.into(), rotation: Some(Keys { times: vec![0.0], values: vec![[w, 0.0, 0.0, 0.0]] }), translation: None }
    }

    /// Merging plays both clips as one: the union of their tracks; where both
    /// animate the same bone, morph or parameter, the receiving clip's track
    /// wins.
    #[test]
    fn merge_takes_the_union_and_keeps_its_own_tracks() {
        let mut body = Clip { name: "idle".into(), bones: vec![bone("Hips", 1.0), bone("Head", 1.0)], ..Default::default() };
        let face = Clip {
            name: "expression".into(),
            bones: vec![bone("Head", 0.5), bone("FACIAL_jaw", 1.0)],
            morphs: vec![ScalarTrack { name: "bs_000".into(), keys: keys(&[0.0], &[1.0]) }],
            ..Default::default()
        };
        body.merge(face);
        assert_eq!(body.name, "idle");
        let names: Vec<&str> = body.bones.iter().map(|b| b.bone.as_str()).collect();
        assert_eq!(names, ["Hips", "Head", "FACIAL_jaw"]);
        assert_eq!(body.bones[1].rotation.as_ref().unwrap().values[0][0], 1.0, "own Head track kept");
        assert_eq!(body.morphs.len(), 1);
    }

    #[test]
    fn invalid_tracks_are_named() {
        let mut clip = Clip { name: "c".into(), ..Default::default() };
        assert_eq!(clip.invalid_track(), None);
        clip.morphs.push(ScalarTrack { name: "bs_000".into(), keys: keys(&[0.0, 1.0], &[0.0]) });
        assert_eq!(clip.invalid_track().as_deref(), Some("morph 'bs_000'"));
        clip.morphs[0].keys = keys(&[1.0, 0.0], &[0.0, 1.0]);
        assert!(clip.invalid_track().is_some(), "times must ascend");
        clip.morphs[0].keys = keys(&[0.0, 1.0], &[0.0, 1.0]);
        clip.bones.push(BoneTrack {
            bone: "jaw".into(),
            rotation: Some(Keys { times: vec![], values: vec![] }),
            translation: None,
        });
        assert_eq!(clip.invalid_track().as_deref(), Some("bone 'jaw'"));
    }
}
