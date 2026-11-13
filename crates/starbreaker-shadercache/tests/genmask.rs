//! Generation-mask bits are per shader family: the same flag name can sit on
//! different bits in different `.ext` files.

use starbreaker_shadercache::GenMasks;

const ALPHA: &str = "Property\n{\n  Name = %GLOW\n  Mask = 0x20\n}\nProperty\n{\n  Name = %WEAR\n  Mask = 0x100\n}\n";
const BETA: &str = "Property\n{\n  Name = %GLOW\n  Mask = 0x1000\n}\n";

fn masks() -> GenMasks {
    GenMasks::from_ext_files([
        ("Shaders/Alpha.ext", ALPHA),
        (r"Shaders\Beta.ext", BETA),
    ])
}

#[test]
fn same_flag_maps_to_each_familys_own_bit() {
    let m = masks();
    assert_eq!(m.family("Alpha").and_then(|f| f.bit("%GLOW")), Some(0x20));
    assert_eq!(m.family("Beta").and_then(|f| f.bit("GLOW")), Some(0x1000));
}

#[test]
fn flags_of_other_families_do_not_leak() {
    let m = masks();
    let beta = m.family("Beta").expect("family");
    let (mask, unknown) = beta.mask_for(&["GLOW".to_owned(), "WEAR".to_owned()]);
    assert_eq!(mask, 0x1000);
    assert_eq!(unknown, vec!["WEAR".to_owned()]);
}

#[test]
fn family_lookup_ignores_case_and_rejects_unknown_families() {
    let m = masks();
    assert!(m.family("alpha").is_some());
    assert!(m.family("Gamma").is_none());
    assert_eq!(m.families(), vec!["Alpha", "Beta"]);
}
