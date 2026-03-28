use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: convert <input.skin|.cgf> [output.glb]");
        std::process::exit(1);
    }

    let input = &args[1];
    let output = if args.len() >= 3 {
        args[2].clone()
    } else {
        input.replace(".skin", ".glb").replace(".cgf", ".glb")
    };

    let data = std::fs::read(input).expect("failed to read input file");
    println!("input: {} ({} bytes)", input, data.len());

    match starbreaker_gltf::skin_to_glb(&data) {
        Ok(glb) => {
            std::fs::write(&output, &glb).expect("failed to write output");
            println!("output: {} ({} bytes)", output, glb.len());
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
