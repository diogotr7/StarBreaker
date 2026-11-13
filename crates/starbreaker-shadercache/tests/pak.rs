//! `ShaderPak` over a synthetic, stored (uncompressed) ZIP built by the test.
//!
//! Regression: permutations that share a shader and entry point but differ in
//! their masks must each read back their *own* module.

use starbreaker_shadercache::{ShaderPak, Stage};

/// Minimal stored-ZIP writer: local headers, central directory, end record.
fn stored_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in files {
        let mut crc = flate2::Crc::new();
        crc.update(data);
        let crc = crc.sum();
        let offset = out.len() as u32;
        let size = data.len() as u32;

        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0u16.to_le_bytes()); // flags
        out.extend_from_slice(&0u16.to_le_bytes()); // stored
        out.extend_from_slice(&0u32.to_le_bytes()); // time + date
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);

        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes()); // made by
        central.extend_from_slice(&20u16.to_le_bytes()); // needed
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u32.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // extra
        central.extend_from_slice(&0u16.to_le_bytes()); // comment
        central.extend_from_slice(&0u16.to_le_bytes()); // disk
        central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
        central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let cd_offset = out.len() as u32;
    let cd_size = central.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

/// An `SHCB` block whose payload is a recognisable marker.
fn shcb(marker: u8) -> Vec<u8> {
    let mut b = b"SHCB".to_vec();
    b.extend_from_slice(&[0u8; 24]);
    b.extend_from_slice(&[marker; 8]);
    b
}

#[test]
fn each_mask_variant_reads_its_own_module() {
    let a = shcb(0xA1);
    let b = shcb(0xB2);
    let c = shcb(0xC3);
    let zip = stored_zip(&[
        ("Shaders/Vulkan/PShaders/Fam@FamPS(GL1)(RT0).shcb", &a),
        ("Shaders/Vulkan/PShaders/Fam@FamPS(GL2)(RT0).shcb", &b),
        ("Shaders/Vulkan/PShaders/Fam@FamPS(GL2)(RT4).shcb", &c),
    ]);
    let path = std::env::temp_dir().join(format!("sbc-pak-{}.pak", std::process::id()));
    std::fs::write(&path, &zip).unwrap();

    let pak = ShaderPak::open(&path).expect("opens");
    assert_eq!(pak.len(), 3);
    for perm in pak.permutations() {
        let want = match (perm.masks.gl, perm.masks.rt) {
            (Some(1), _) => &a,
            (Some(2), Some(0)) => &b,
            (Some(2), Some(4)) => &c,
            other => panic!("unexpected masks {other:?}"),
        };
        assert_eq!(&pak.read_block(perm).expect("reads"), want, "{}", perm.path);
        assert_eq!(perm.stage, Stage::Pixel);
    }

    std::fs::remove_file(&path).ok();
}

/// A fragment module with `uniforms` members in `PER_MATERIAL` (set 1) and
/// `textures` 2D textures in set 1, wrapped in an `SHCB` block.
fn shcb_with_material_interface(uniforms: u32, textures: u32) -> Vec<u8> {
    use rspirv::binary::Assemble;
    use rspirv::dr::{Builder, Operand};
    use rspirv::spirv::{
        AddressingModel, Capability, Decoration, Dim, ExecutionModel, FunctionControl,
        ImageFormat, MemoryModel, StorageClass,
    };
    let mut b = Builder::new();
    b.set_version(1, 0);
    b.capability(Capability::Shader);
    b.memory_model(AddressingModel::Logical, MemoryModel::GLSL450);
    let void = b.type_void();
    let f32_ty = b.type_float(32, None);
    let vec4 = b.type_vector(f32_ty, 4);
    let mut interface = Vec::new();
    if uniforms > 0 {
        let block = b.type_struct((0..uniforms).map(|_| vec4).collect::<Vec<_>>());
        b.decorate(block, Decoration::Block, []);
        for i in 0..uniforms {
            b.member_name(block, i, format!("u{i}"));
            b.member_decorate(block, i, Decoration::Offset, [Operand::LiteralBit32(i * 16)]);
        }
        let ptr = b.type_pointer(None, StorageClass::Uniform, block);
        let var = b.variable(ptr, None, StorageClass::Uniform, None);
        b.name(var, "PER_MATERIAL");
        b.decorate(var, Decoration::DescriptorSet, [Operand::LiteralBit32(1)]);
        b.decorate(var, Decoration::Binding, [Operand::LiteralBit32(35)]);
    }
    let image = b.type_image(f32_ty, Dim::Dim2D, 0, 0, 0, 1, ImageFormat::Unknown, None);
    let ptr = b.type_pointer(None, StorageClass::UniformConstant, image);
    for i in 0..textures {
        let var = b.variable(ptr, None, StorageClass::UniformConstant, None);
        b.name(var, format!("tex{i}"));
        b.decorate(var, Decoration::DescriptorSet, [Operand::LiteralBit32(1)]);
        b.decorate(var, Decoration::Binding, [Operand::LiteralBit32(40 + i)]);
        interface.push(var);
    }
    let fn_ty = b.type_function(void, []);
    let main = b.begin_function(void, None, FunctionControl::NONE, fn_ty).unwrap();
    b.begin_block(None).unwrap();
    b.ret().unwrap();
    b.end_function().unwrap();
    b.entry_point(ExecutionModel::Fragment, main, "main", &interface);
    let mut out = b"SHCB".to_vec();
    out.extend_from_slice(&[0u8; 24]);
    out.extend(b.module().assemble().into_iter().flat_map(|w| w.to_le_bytes()));
    out
}

fn open_pak(tag: &str, files: &[(&str, &[u8])]) -> (ShaderPak, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!("sbc-pak-{tag}-{}.pak", std::process::id()));
    std::fs::write(&path, stored_zip(files)).unwrap();
    (ShaderPak::open(&path).expect("opens"), path)
}

#[test]
fn equal_mask_ties_go_to_the_pass_reading_the_most_material_state() {
    let main = shcb_with_material_interface(4, 6);
    let depth = shcb_with_material_interface(4, 2);
    let helper = shcb_with_material_interface(0, 0);
    // Two orders, so pak order cannot be what decides.
    for (tag, files) in [
        ("tie-a", [
            ("Shaders/Vulkan/PShaders/Fam@DepthPassPS(GL4).shcb", depth.as_slice()),
            ("Shaders/Vulkan/PShaders/Fam@CommonHelperPS(GL4).shcb", helper.as_slice()),
            ("Shaders/Vulkan/PShaders/Fam@MainPS(GL4).shcb", main.as_slice()),
        ]),
        ("tie-b", [
            ("Shaders/Vulkan/PShaders/Fam@MainPS(GL4).shcb", main.as_slice()),
            ("Shaders/Vulkan/PShaders/Fam@CommonHelperPS(GL4).shcb", helper.as_slice()),
            ("Shaders/Vulkan/PShaders/Fam@DepthPassPS(GL4).shcb", depth.as_slice()),
        ]),
    ] {
        let (pak, path) = open_pak(tag, &files);
        let (perm, missing, extra) = pak
            .select_stage("Fam", Stage::Pixel, 0x4)
            .expect("reflects")
            .expect("a candidate");
        assert_eq!((perm.entry.as_str(), missing, extra), ("MainPS", 0, 0), "{tag}");
        std::fs::remove_file(&path).ok();
    }
}

#[test]
fn mask_fit_still_outranks_interface_size() {
    let rich = shcb_with_material_interface(8, 8);
    let lean = shcb_with_material_interface(1, 1);
    let (pak, path) = open_pak("fit", &[
        // richer, but carries an extra bit the material did not ask for
        ("Shaders/Vulkan/PShaders/Fam@RichPS(GL6).shcb", rich.as_slice()),
        ("Shaders/Vulkan/PShaders/Fam@LeanPS(GL4).shcb", lean.as_slice()),
    ]);
    let (perm, _, extra) = pak.select_stage("Fam", Stage::Pixel, 0x4).unwrap().unwrap();
    assert_eq!((perm.entry.as_str(), extra), ("LeanPS", 0));
    std::fs::remove_file(&path).ok();
}

/// A module of `model` with `Input`/`Output` variables at the given locations
/// and `bindings` extra set-0 uniform blocks, wrapped in an `SHCB` block.
fn shcb_with_io(
    model: rspirv::spirv::ExecutionModel,
    inputs: &[u32],
    outputs: &[u32],
    bindings: u32,
) -> Vec<u8> {
    use rspirv::binary::Assemble;
    use rspirv::dr::{Builder, Operand};
    use rspirv::spirv::{AddressingModel, Capability, Decoration, FunctionControl, MemoryModel, StorageClass};
    let mut b = Builder::new();
    b.set_version(1, 0);
    b.capability(Capability::Shader);
    b.memory_model(AddressingModel::Logical, MemoryModel::GLSL450);
    let void = b.type_void();
    let f32_ty = b.type_float(32, None);
    let vec4 = b.type_vector(f32_ty, 4);
    let mut interface = Vec::new();
    for (class, locs) in [(StorageClass::Input, inputs), (StorageClass::Output, outputs)] {
        let ptr = b.type_pointer(None, class, vec4);
        for &loc in locs {
            let var = b.variable(ptr, None, class, None);
            b.name(var, format!("{class:?}{loc}"));
            b.decorate(var, Decoration::Location, [Operand::LiteralBit32(loc)]);
            interface.push(var);
        }
    }
    let block = b.type_struct([vec4]);
    b.decorate(block, Decoration::Block, []);
    b.member_decorate(block, 0, Decoration::Offset, [Operand::LiteralBit32(0)]);
    let ptr = b.type_pointer(None, StorageClass::Uniform, block);
    for i in 0..bindings {
        let var = b.variable(ptr, None, StorageClass::Uniform, None);
        b.decorate(var, Decoration::DescriptorSet, [Operand::LiteralBit32(0)]);
        b.decorate(var, Decoration::Binding, [Operand::LiteralBit32(i)]);
    }
    let fn_ty = b.type_function(void, []);
    let main = b.begin_function(void, None, FunctionControl::NONE, fn_ty).unwrap();
    b.begin_block(None).unwrap();
    b.ret().unwrap();
    b.end_function().unwrap();
    b.entry_point(model, main, "main", &interface);
    let mut out = b"SHCB".to_vec();
    out.extend_from_slice(&[0u8; 24]);
    out.extend(b.module().assemble().into_iter().flat_map(|w| w.to_le_bytes()));
    out
}

#[test]
fn linked_selection_requires_outputs_that_feed_the_consumer() {
    use rspirv::spirv::ExecutionModel::{Fragment, Vertex};
    let ps = shcb_with_io(Fragment, &[0, 1, 2, 5], &[0], 0);
    let main_vs = shcb_with_io(Vertex, &[], &[0, 1, 2, 3, 4, 5], 1);
    // Busier, but its outputs cannot feed the pixel shader.
    let helper_vs = shcb_with_io(Vertex, &[], &[0, 1, 2], 6);
    let (pak, path) = open_pak("link", &[
        ("Shaders/Vulkan/PShaders/Fam@MainPS(GL0).shcb", ps.as_slice()),
        ("Shaders/Vulkan/VShaders/Fam@HelperVS(GL0).shcb", helper_vs.as_slice()),
        ("Shaders/Vulkan/VShaders/Fam@MainVS(GL0).shcb", main_vs.as_slice()),
    ]);
    let (pixel, ..) = pak.select_stage("Fam", Stage::Pixel, 0).unwrap().unwrap();
    let (vertex, ..) = pak
        .select_linked("Fam", Stage::Vertex, 0, pixel)
        .unwrap()
        .expect("a linkable vertex shader");
    assert_eq!(vertex.entry, "MainVS");

    let reflection = pak.reflect(pixel).unwrap();
    let inputs: Vec<u32> = reflection.inputs.iter().map(|v| v.location).collect();
    assert_eq!(inputs, vec![0, 1, 2, 5]);
    let outputs: Vec<u32> = pak.reflect(vertex).unwrap().outputs.iter().map(|v| v.location).collect();
    assert_eq!(outputs, vec![0, 1, 2, 3, 4, 5]);
    std::fs::remove_file(&path).ok();
}

#[test]
fn linked_selection_returns_none_when_nothing_links() {
    use rspirv::spirv::ExecutionModel::{Fragment, Vertex};
    let ps = shcb_with_io(Fragment, &[0, 7], &[0], 0);
    let vs = shcb_with_io(Vertex, &[], &[0, 1], 0);
    let (pak, path) = open_pak("nolink", &[
        ("Shaders/Vulkan/PShaders/Fam@MainPS(GL0).shcb", ps.as_slice()),
        ("Shaders/Vulkan/VShaders/Fam@MainVS(GL0).shcb", vs.as_slice()),
    ]);
    let (pixel, ..) = pak.select_stage("Fam", Stage::Pixel, 0).unwrap().unwrap();
    assert!(pak.select_linked("Fam", Stage::Vertex, 0, pixel).unwrap().is_none());
    std::fs::remove_file(&path).ok();
}
