//! Shader parameter defaults from decoded shader text: annotated declarations,
//! the preprocessor (`%FLAG`, `#define`d symbols, `#if`/`#elif`/`#else`) and
//! the `#include` order. All text here is synthetic.

use std::collections::{BTreeMap, BTreeSet};

use starbreaker_shadercache::defaults::param_defaults;

fn sources(files: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
    let map: BTreeMap<String, String> = files.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    move |stem: &str| map.get(stem).cloned()
}

fn flags(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn parses_annotated_scalar_and_vector_defaults() {
    let fam = "float WearBlendBase <string UIWidget = \" slider \"; string UIName = \" a > b \"; float UIMin = 0.0;>= 0.25; \
               float MacroTiling ; <string UIName = \" Tiling \";>= 2; \
               float4 TintA <string UIName = \" T \";> = {1, 0.5, 0, 1}; \
               float3 TintB <string UIName = \" U \";>= float3 (0.1, 0.2, 0.3); \
               bool UseX <string UIName = \" X \";>= true; \
               float NotAParam = 3.0; float Expr <string UIName = \" E \";>= SOME_MACRO;";
    let d = param_defaults("Fam", &sources(&[("Fam", fam)]), &flags(&[])).unwrap();
    assert_eq!(d["WearBlendBase"].values, vec![0.25]);
    assert_eq!(d["WearBlendBase"].ty, "float");
    assert_eq!(d["MacroTiling"].values, vec![2.0], "stray `;` before `<` is tolerated");
    assert_eq!(d["TintA"].values, vec![1.0, 0.5, 0.0, 1.0]);
    assert_eq!(d["TintB"].values, vec![0.1, 0.2, 0.3]);
    assert_eq!(d["UseX"].values, vec![1.0]);
    assert!(!d.contains_key("NotAParam"), "no annotation: not a public parameter");
    assert!(!d.contains_key("Expr"), "unparsable value: left out, never guessed");
    assert_eq!(d["TintA"].file, "Fam");
}

#[test]
fn defaults_follow_preprocessor_and_include_order() {
    let fam = "#if %WEAR_LAYER\n\
               #define HAS_WEAR 1\n\
               #endif\n\
               #include Lib\n\
               #if %WEAR_LAYER && !%_RT_QUALITY\n\
               float A <string UIName = \" a \";>= 1.0;\n\
               #elif HAS_OTHER\n\
               float A <string UIName = \" a \";>= 2.0;\n\
               #else\n\
               float A <string UIName = \" a \";>= 3.0;\n\
               #endif\n\
               #include Late float trailing = 0;\n";
    let lib = "#ifdef HAS_WEAR\nfloat B <string UIName = \" b \";>= 10.0;\n#else\nfloat B <string UIName = \" b \";>= 20.0;\n#endif\n\
               #include Lib\n";
    let late = "float B <string UIName = \" b \";>= 30.0;\n#define %SECRET 1\n#if %SECRET\nfloat C <string UIName = \" c \";>= 5.0;\n#endif\n";
    let src = sources(&[("Fam", fam), ("Lib", lib), ("Late", late)]);

    let d = param_defaults("Fam", &src, &flags(&["WEAR_LAYER"])).unwrap();
    assert_eq!(d["A"].values, vec![1.0]);
    assert_eq!(d["B"].values, vec![30.0], "the later include's redefinition wins");
    assert_eq!(d["B"].file, "Late");
    assert_eq!(d["C"].values, vec![5.0], "a #define'd %FLAG is true");

    let d = param_defaults("Fam", &src, &flags(&[])).unwrap();
    assert_eq!(d["A"].values, vec![3.0]);
}

#[test]
fn unparsable_condition_is_an_error() {
    let fam = "#if (1 ==\nfloat A <string UIName = \" a \";>= 1.0;\n#endif\n";
    let err = param_defaults("Fam", &sources(&[("Fam", fam)]), &flags(&[])).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("Fam") && msg.contains("line 1"), "{msg}");
}

/// Integer preprocessor semantics: macro values, comparisons, arithmetic;
/// an undefined name is 0.
#[test]
fn integer_conditions_follow_the_c_preprocessor() {
    let fam = "#define LAYERS 3\n\
               #if (LAYERS> 2)\nfloat A <string UIName = \" a \";>= 1.0;\n#endif\n\
               #if LAYERS * 2 == 6 && UNDEFINED_THING == 0\nfloat B <string UIName = \" b \";>= 2.0;\n#endif\n\
               #if LAYERS - 3\nfloat C <string UIName = \" c \";>= 3.0;\n#endif\n";
    let d = param_defaults("Fam", &sources(&[("Fam", fam)]), &flags(&[])).unwrap();
    assert_eq!(d["A"].values, vec![1.0]);
    assert_eq!(d["B"].values, vec![2.0]);
    assert!(!d.contains_key("C"));
}

#[test]
fn missing_family_is_an_error() {
    assert!(param_defaults("Nope", &sources(&[]), &flags(&[])).is_err());
}

/// Real data (ignored; needs Data.p4k): with the Gladius `primary_hardsurface`
/// flags, every reflected PER_MATERIAL member of its G-buffer permutation
/// has a default, except the engine-filled and layer-definition members.
/// Prints counts only.
#[test]
#[ignore = "needs Data.p4k"]
fn hardsurface_defaults_on_real_data() {
    use starbreaker_shadercache::{EXT_PAK, GenMasks, ResourceClass, ShaderPak, Stage, cfxb, hlsl, read_pak_entries};
    let Ok((p4k_path, _)) = starbreaker_p4k::find_p4k() else {
        eprintln!("no Data.p4k; skipping");
        return;
    };
    let p4k_path = p4k_path.to_string_lossy().into_owned();
    let p4k = starbreaker_p4k::MappedP4k::open(&p4k_path).unwrap();
    let mtl = starbreaker_3d::mtl::parse_mtl(&p4k.read_file(r"Data\Objects\Spaceships\Ships\AEGS\Gladius\AEGS_Gladius.mtl").unwrap()).unwrap();
    let sub = mtl.materials.iter().find(|m| m.name == "primary_hardsurface").unwrap();
    let tokens = sub.decoded_string_gen_mask().tokens;
    let pak = ShaderPak::open_in_p4k(&p4k_path).unwrap();
    let decode = |stem: &str| pak.read_shader_bin(stem).ok().and_then(|b| cfxb::parse(&b).ok()).and_then(|b| hlsl::decode(&b).ok());
    let flags: BTreeSet<String> = tokens.iter().cloned().collect();
    let d = param_defaults("HardSurface", &decode, &flags).unwrap();

    let ext = read_pak_entries(&p4k_path, EXT_PAK).unwrap();
    let files: Vec<(String, String)> = ext.iter().map(|(n, b)| (n.clone(), String::from_utf8_lossy(b).into_owned())).collect();
    let masks = GenMasks::from_ext_files(files.iter().map(|(n, t)| (n.as_str(), t.as_str())));
    let (want, _) = masks.family("HardSurface").unwrap().mask_for(&tokens);
    let perm = pak
        .family("HardSurface")
        .filter(|p| p.stage == Stage::Pixel && p.entry == "HardSurfaceZPassPS")
        .min_by_key(|p| {
            let gl = p.masks.gl.unwrap_or(0);
            ((want & !gl).count_ones(), (gl & !want).count_ones())
        })
        .unwrap();
    let r = pak.reflect(perm).unwrap();
    let engine_filled = ["MatDifColor", "MatEmissiveColor", "_TintOverWrite_Opacity_fAlphaRef_GlowFactor"];
    let members: Vec<String> = r
        .uniform_blocks(ResourceClass::PerMaterial)
        .flat_map(|b| b.members.iter())
        .filter(|m| !engine_filled.contains(&m.name.as_str()) && m.ty != "LayerDefinition")
        .map(|m| m.name.clone())
        .collect();
    let missing: Vec<&String> = members.iter().filter(|m| !d.contains_key(*m)).collect();
    println!("defaults: {} parsed, {} PER_MATERIAL members checked, missing {:?}", d.len(), members.len(), missing);
    let all: BTreeSet<String> = ["WEAR_LAYER", "STENCIL_MAP", "PARALLAX_OCCLUSION_MAPPING", "IRIDESCENCE", "USE_DAMAGE_MAP", "MACRO_EMISSIVE_MAP", "ANIM_GLOW", "VERTCOLORS", "USE_DETAIL_MAPS"].iter().map(|s| s.to_string()).collect();
    let wide = param_defaults("HardSurface", &decode, &all).unwrap();
    println!("defaults with many flags: {} {:?}", wide.len(), wide.keys().collect::<Vec<_>>());
    assert!(missing.is_empty(), "members without a default: {missing:?}");
}

/// The decoder can join code onto a directive line: the condition is the
/// longest valid expression; what follows is code on the active path.
#[test]
fn code_joined_onto_if_lines_is_code() {
    let fam = "#if %A || B { x = 1; } float P <string UIName = \" p \";>= 4.0;\n#endif\n\
               #if %NOPE float Q <string UIName = \" q \";>= 5.0;\n#endif\n";
    let d = param_defaults("Fam", &sources(&[("Fam", fam)]), &flags(&["A"])).unwrap();
    assert_eq!(d["P"].values, vec![4.0]);
    assert!(!d.contains_key("Q"));
}
