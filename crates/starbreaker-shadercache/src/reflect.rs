//! SPIR-V reflection with `rspirv`: entry points, descriptor bindings,
//! uniform-block layouts, and located stage inputs/outputs (which stages of a
//! pass link to each other — [`Reflection::feeds`]).
//!
//! Star Citizen's Vulkan modules are fully decorated: every resource carries
//! `DescriptorSet` and `Binding`, where the set is the resource class
//! (0 = view/frame, 1 = material, 2 = mesh, 3 = object) and the binding is the
//! HLSL register index within that set. Uniform buffers additionally carry
//! `Offset` decorations on their members, so the exact cbuffer layout —
//! including packed layer descriptors — can be recovered per permutation.
//!
//! This is what turns "the material has a `MatDifColor`" into "`MatDifColor`
//! is the first `float4` of the per-material block at offset 0".

use std::collections::HashMap;

use rspirv::dr::Operand;
use rspirv::spirv::{Decoration, Dim, Op, StorageClass};

use crate::error::ShaderCacheError;

/// One member of a uniform block, with its byte offset and size.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Member {
    pub name: String,
    /// Byte offset from the block start (`OpMemberDecorate Offset`).
    pub offset: Option<u32>,
    /// HLSL-style type description, e.g. `float4`, `float4x4`, `LayerDefinition[4]`.
    pub ty: String,
    /// Size in bytes, best effort (structs sum their members).
    pub size: u32,
}

/// An entry point declared by the module.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EntryPoint {
    pub name: String,
    pub execution_model: String,
}

/// Resource class encoded in the descriptor set index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum ResourceClass {
    PerView,
    PerMaterial,
    PerMesh,
    PerObject,
    Other(u32),
}

impl ResourceClass {
    pub fn from_set(set: Option<u32>) -> Self {
        match set {
            Some(0) => Self::PerView,
            Some(1) => Self::PerMaterial,
            Some(2) => Self::PerMesh,
            Some(3) => Self::PerObject,
            Some(other) => Self::Other(other),
            None => Self::Other(u32::MAX),
        }
    }
}

/// A descriptor variable bound to `(set, binding)`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Binding {
    pub set: Option<u32>,
    pub binding: Option<u32>,
    pub class: ResourceClass,
    pub storage_class: String,
    pub ty: String,
    pub name: String,
    /// Members when the variable points at a struct (uniform blocks).
    pub members: Vec<Member>,
    /// Total block size in bytes (0 for non-struct resources).
    pub block_size: u32,
}

/// Everything the renderer or an exporter needs from one shader permutation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Reflection {
    pub entry_points: Vec<EntryPoint>,
    pub bindings: Vec<Binding>,
    /// Stage inputs with a `Location` (built-ins excluded), by location.
    pub inputs: Vec<StageVar>,
    /// Stage outputs with a `Location` (built-ins excluded), by location.
    pub outputs: Vec<StageVar>,
}

/// A located stage input or output — what links one pipeline stage to the next.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StageVar {
    pub location: u32,
    pub name: String,
    pub ty: String,
}

impl Reflection {
    /// Whether this stage's outputs supply every located input of `consumer`
    /// (the next stage of the same pass), matching location and type.
    pub fn feeds(&self, consumer: &Reflection) -> bool {
        consumer.inputs.iter().all(|input| {
            self.outputs
                .iter()
                .any(|out| out.location == input.location && out.ty == input.ty)
        })
    }

    /// Descriptor sets referenced by this module, ascending.
    pub fn sets(&self) -> Vec<u32> {
        let mut v: Vec<u32> = self.bindings.iter().filter_map(|b| b.set).collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// Bindings of one resource class, ordered by binding number.
    pub fn class(&self, class: ResourceClass) -> impl Iterator<Item = &Binding> {
        self.bindings.iter().filter(move |b| b.class == class)
    }

    /// The uniform blocks of a class (the ones a material exporter cares about).
    pub fn uniform_blocks(&self, class: ResourceClass) -> impl Iterator<Item = &Binding> {
        self.class(class)
            .filter(|b| b.storage_class == "Uniform" && !b.members.is_empty())
    }

    /// Image bindings of a class (textures, texel buffers, storage images).
    pub fn textures(&self, class: ResourceClass) -> impl Iterator<Item = &Binding> {
        self.class(class)
            .filter(|b| b.storage_class == "UniformConstant" && b.ty != "SamplerState")
    }

    /// Sampler bindings of a class.
    pub fn samplers(&self, class: ResourceClass) -> impl Iterator<Item = &Binding> {
        self.class(class)
            .filter(|b| b.storage_class == "UniformConstant" && b.ty == "SamplerState")
    }
}

fn id_of(op: &Operand) -> Option<u32> {
    match op {
        Operand::IdRef(id) => Some(*id),
        _ => None,
    }
}

fn u32_of(op: &Operand) -> Option<u32> {
    match op {
        Operand::LiteralBit32(v) => Some(*v),
        Operand::LiteralBit64(v) => Some(*v as u32),
        _ => None,
    }
}

fn u64_of(op: &Operand) -> Option<u64> {
    match op {
        Operand::LiteralBit32(v) => Some(*v as u64),
        Operand::LiteralBit64(v) => Some(*v),
        _ => None,
    }
}

fn str_of(op: &Operand) -> Option<&str> {
    match op {
        Operand::LiteralString(s) => Some(s),
        _ => None,
    }
}

/// Opaque resource types, as declared.
#[derive(Clone, Copy)]
enum ImageKind {
    Image {
        dim: Dim,
        arrayed: bool,
        multisampled: bool,
        /// `Sampled == 2`: a storage (read-write) image.
        storage: bool,
    },
    Sampler,
    /// Combined image + sampler; describes as its image type.
    SampledImage(u32),
}

/// HLSL spelling of an image type.
fn image_name(dim: Dim, arrayed: bool, multisampled: bool, storage: bool) -> String {
    let rw = if storage { "RW" } else { "" };
    let base = match dim {
        Dim::Dim1D => "Texture1D",
        Dim::Dim2D => "Texture2D",
        Dim::Dim3D => "Texture3D",
        Dim::DimCube => "TextureCube",
        Dim::DimRect => "TextureRect",
        Dim::DimBuffer => return format!("{rw}Buffer"),
        Dim::DimSubpassData => return "SubpassInput".to_owned(),
        other => return format!("{rw}Texture{other:?}"),
    };
    let ms = if multisampled { "MS" } else { "" };
    let array = if arrayed { "Array" } else { "" };
    format!("{rw}{base}{ms}{array}")
}

/// Everything needed to describe types by id.
#[derive(Default)]
struct Types {
    pointers: HashMap<u32, (StorageClass, u32)>,
    structs: HashMap<u32, Vec<u32>>,
    floats: HashMap<u32, u32>,
    ints: HashMap<u32, (u32, bool)>,
    vectors: HashMap<u32, (u32, u32)>,
    matrices: HashMap<u32, (u32, u32)>,
    arrays: HashMap<u32, (u32, u32)>,
    images: HashMap<u32, ImageKind>,
    constants: HashMap<u32, u64>,
    names: HashMap<u32, String>,
}

impl Types {
    fn describe(&self, id: u32, depth: u32) -> (String, u32) {
        if depth > 6 {
            return ("?".to_owned(), 0);
        }
        if let Some((w, signed)) = self.ints.get(&id) {
            let bytes = (w / 8).max(1);
            let name = match (*w, *signed) {
                (32, true) => "int".to_owned(),
                (32, false) => "uint".to_owned(),
                (64, true) => "int64".to_owned(),
                (64, false) => "uint64".to_owned(),
                (_, true) => format!("int{w}"),
                (_, false) => format!("uint{w}"),
            };
            return (name, bytes);
        }
        if let Some(w) = self.floats.get(&id) {
            let bytes = (w / 8).max(1);
            let name = match w {
                16 => "half".to_owned(),
                32 => "float".to_owned(),
                64 => "double".to_owned(),
                _ => format!("float{w}"),
            };
            return (name, bytes);
        }
        if let Some((comp, count)) = self.vectors.get(&id) {
            let (cn, cs) = self.describe(*comp, depth + 1);
            return (format!("{cn}{count}"), cs * count);
        }
        if let Some((col, rows)) = self.matrices.get(&id) {
            let (cn, cs) = self.describe(*col, depth + 1);
            return (format!("{cn}x{rows}"), cs * rows);
        }
        if let Some((elem, len)) = self.arrays.get(&id) {
            let (en, es) = self.describe(*elem, depth + 1);
            return (format!("{en}[{len}]"), es * len);
        }
        if self.structs.contains_key(&id) {
            let name = self.names.get(&id).cloned().unwrap_or_else(|| format!("struct{id}"));
            let size = self
                .structs
                .get(&id)
                .map(|m| m.iter().map(|m| self.describe(*m, depth + 1).1).sum())
                .unwrap_or(0);
            return (name, size);
        }
        if let Some(kind) = self.images.get(&id) {
            let name = match *kind {
                ImageKind::Image {
                    dim,
                    arrayed,
                    multisampled,
                    storage,
                } => image_name(dim, arrayed, multisampled, storage),
                ImageKind::Sampler => "SamplerState".to_owned(),
                ImageKind::SampledImage(image) => return self.describe(image, depth + 1),
            };
            return (name, 0);
        }
        ("?".to_owned(), 0)
    }
}

/// Parse a SPIR-V module and extract entry points, bindings and block layouts.
pub fn reflect(spirv: &[u8]) -> Result<Reflection, ShaderCacheError> {
    if !spirv.len().is_multiple_of(4) {
        return Err(ShaderCacheError::SpirvParse(
            "length not a multiple of 4".into(),
        ));
    }
    let words: Vec<u32> = spirv
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    let module = rspirv::dr::load_words(words.into_iter())
        .map_err(|e| ShaderCacheError::SpirvParse(format!("{e:?}")))?;

    let mut t = Types::default();
    let mut member_names: HashMap<(u32, u32), String> = HashMap::new();
    let mut member_offsets: HashMap<(u32, u32), u32> = HashMap::new();
    let mut sets: HashMap<u32, u32> = HashMap::new();
    let mut bindings: HashMap<u32, u32> = HashMap::new();
    let mut locations: HashMap<u32, u32> = HashMap::new();

    for inst in &module.debug_names {
        match inst.class.opcode {
            Op::Name => {
                if let (Some(id), Some(name)) =
                    (id_of(&inst.operands[0]), str_of(&inst.operands[1]))
                {
                    t.names.insert(id, name.to_owned());
                }
            }
            Op::MemberName => {
                if let (Some(ty), Some(idx), Some(name)) = (
                    id_of(&inst.operands[0]),
                    u32_of(&inst.operands[1]),
                    str_of(&inst.operands[2]),
                ) {
                    member_names.insert((ty, idx), name.to_owned());
                }
            }
            _ => {}
        }
    }

    for inst in &module.annotations {
        match inst.class.opcode {
            Op::Decorate => {
                let Some(target) = id_of(&inst.operands[0]) else {
                    continue;
                };
                let Some(Operand::Decoration(dec)) = inst.operands.get(1) else {
                    continue;
                };
                let Some(value) = inst.operands.get(2).and_then(u32_of) else {
                    continue;
                };
                match dec {
                    Decoration::DescriptorSet => {
                        sets.insert(target, value);
                    }
                    Decoration::Binding => {
                        bindings.insert(target, value);
                    }
                    Decoration::Location => {
                        locations.insert(target, value);
                    }
                    _ => {}
                }
            }
            Op::MemberDecorate => {
                if let (Some(ty), Some(idx), Some(Operand::Decoration(Decoration::Offset)), Some(v)) =
                    (
                        id_of(&inst.operands[0]),
                        u32_of(&inst.operands[1]),
                        inst.operands.get(2),
                        inst.operands.get(3).and_then(u32_of),
                    )
                {
                    member_offsets.insert((ty, idx), v);
                }
            }
            _ => {}
        }
    }

    let mut variables: Vec<(u32, u32)> = Vec::new();
    for inst in &module.types_global_values {
        match inst.class.opcode {
            Op::TypePointer => {
                if let (Some(id), Some(Operand::StorageClass(sc)), Some(pointee)) = (
                    inst.result_id,
                    inst.operands.first(),
                    inst.operands.get(1).and_then(id_of),
                ) {
                    t.pointers.insert(id, (*sc, pointee));
                }
            }
            Op::TypeStruct => {
                if let Some(id) = inst.result_id {
                    t.structs
                        .insert(id, inst.operands.iter().filter_map(id_of).collect());
                }
            }
            Op::TypeFloat => {
                if let (Some(id), Some(w)) = (inst.result_id, inst.operands.first().and_then(u32_of))
                {
                    t.floats.insert(id, w);
                }
            }
            Op::TypeInt => {
                if let (Some(id), Some(w), Some(signed)) = (
                    inst.result_id,
                    inst.operands.first().and_then(u32_of),
                    inst.operands.get(1).and_then(u32_of),
                ) {
                    t.ints.insert(id, (w, signed != 0));
                }
            }
            Op::TypeVector => {
                if let (Some(id), Some(c), Some(n)) = (
                    inst.result_id,
                    inst.operands.first().and_then(id_of),
                    inst.operands.get(1).and_then(u32_of),
                ) {
                    t.vectors.insert(id, (c, n));
                }
            }
            Op::TypeMatrix => {
                if let (Some(id), Some(c), Some(n)) = (
                    inst.result_id,
                    inst.operands.first().and_then(id_of),
                    inst.operands.get(1).and_then(u32_of),
                ) {
                    t.matrices.insert(id, (c, n));
                }
            }
            Op::TypeArray => {
                if let (Some(id), Some(elem), Some(len_id)) = (
                    inst.result_id,
                    inst.operands.first().and_then(id_of),
                    inst.operands.get(1).and_then(id_of),
                ) {
                    let len = t.constants.get(&len_id).copied().unwrap_or(0) as u32;
                    t.arrays.insert(id, (elem, len));
                }
            }
            Op::TypeImage => {
                // Operands: sampled type, Dim, Depth, Arrayed, MS, Sampled, Format.
                if let (Some(id), Some(Operand::Dim(dim))) = (inst.result_id, inst.operands.get(1)) {
                    let flag = |i: usize| inst.operands.get(i).and_then(u32_of);
                    t.images.insert(
                        id,
                        ImageKind::Image {
                            dim: *dim,
                            arrayed: flag(3) == Some(1),
                            multisampled: flag(4) == Some(1),
                            storage: flag(5) == Some(2),
                        },
                    );
                }
            }
            Op::TypeSampler => {
                if let Some(id) = inst.result_id {
                    t.images.insert(id, ImageKind::Sampler);
                }
            }
            Op::TypeSampledImage => {
                if let (Some(id), Some(image)) = (inst.result_id, inst.operands.first().and_then(id_of)) {
                    t.images.insert(id, ImageKind::SampledImage(image));
                }
            }
            Op::Constant => {
                if let Some(id) = inst.result_id
                    && let Some(v) = inst.operands.first().and_then(u64_of) {
                        t.constants.insert(id, v);
                    }
            }
            Op::Variable => {
                if let (Some(ty), Some(id)) = (inst.result_type, inst.result_id) {
                    variables.push((ty, id));
                }
            }
            _ => {}
        }
    }

    let mut out_bindings = Vec::new();
    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    for (ty, id) in variables {
        let Some(&(storage, pointee)) = t.pointers.get(&ty) else {
            continue;
        };
        if matches!(storage, StorageClass::Input | StorageClass::Output) {
            if let Some(&location) = locations.get(&id) {
                let var = StageVar {
                    location,
                    name: t.names.get(&id).cloned().unwrap_or_else(|| format!("<id{id}>")),
                    ty: t.describe(pointee, 0).0,
                };
                if storage == StorageClass::Input {
                    inputs.push(var);
                } else {
                    outputs.push(var);
                }
            }
            continue;
        }
        if matches!(storage, StorageClass::Function | StorageClass::Private) {
            continue;
        }

        let mut members = Vec::new();
        let mut block_size = 0u32;
        if let Some(fields) = t.structs.get(&pointee) {
            for (idx, field_ty) in fields.iter().enumerate() {
                let (ty_name, size) = t.describe(*field_ty, 0);
                members.push(Member {
                    name: member_names
                        .get(&(pointee, idx as u32))
                        .cloned()
                        .unwrap_or_else(|| format!("member{idx}")),
                    offset: member_offsets.get(&(pointee, idx as u32)).copied(),
                    ty: ty_name,
                    size,
                });
            }
            block_size = members
                .iter()
                .map(|m| m.offset.unwrap_or(0) + m.size)
                .max()
                .unwrap_or(0);
        }
        let name = t
            .names
            .get(&id)
            .cloned()
            .unwrap_or_else(|| format!("<id{id}>"));
        let set = sets.get(&id).copied();
        out_bindings.push(Binding {
            set,
            binding: bindings.get(&id).copied(),
            class: ResourceClass::from_set(set),
            storage_class: format!("{storage:?}"),
            ty: t.describe(pointee, 0).0,
            name,
            members,
            block_size,
        });
    }
    out_bindings.sort_by_key(|b| (b.set, b.binding));

    let entry_points = module
        .entry_points
        .iter()
        .filter(|e| e.class.opcode == Op::EntryPoint)
        .map(|e| EntryPoint {
            execution_model: match e.operands.first() {
                Some(Operand::ExecutionModel(m)) => format!("{m:?}"),
                _ => "Unknown".to_owned(),
            },
            name: e
                .operands
                .get(2)
                .and_then(str_of)
                .unwrap_or("<unnamed>")
                .to_owned(),
        })
        .collect();

    inputs.sort_by_key(|v| v.location);
    outputs.sort_by_key(|v| v.location);
    Ok(Reflection {
        entry_points,
        bindings: out_bindings,
        inputs,
        outputs,
    })
}





