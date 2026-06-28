//! Compiles the GLSL shaders in `shaders/` to SPIR-V using `glslangValidator`
//! (from the `glslang-tools` package). The resulting `.spv` files are written
//! into `OUT_DIR` and embedded into the binary with `include_bytes!`.

use std::path::Path;
use std::process::Command;

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let out_dir = std::env::var("OUT_DIR").unwrap();
    // shaders live at the workspace root, one level above the crate.
    let shader_dir = Path::new(&manifest_dir).join("../../shaders");

    let shaders = ["blob.vert", "blob.frag"];
    let validator = std::env::var("GLSLANG_VALIDATOR").unwrap_or_else(|_| "glslangValidator".to_string());

    for shader in shaders {
        let src = shader_dir.join(shader);
        let dst = Path::new(&out_dir).join(format!("{shader}.spv"));
        println!("cargo:rerun-if-changed={}", src.display());

        let status = Command::new(&validator)
            .args(["-V", src.to_str().unwrap(), "-o", dst.to_str().unwrap()])
            .status()
            .unwrap_or_else(|e| panic!("failed to run {validator}: {e}. Install `glslang-tools`."));

        if !status.success() {
            panic!("shader compilation failed for {}", src.display());
        }
    }

    println!("cargo:rerun-if-changed=build.rs");
}
