#![allow(clippy::disallowed_methods, reason = "build scripts are exempt")]

fn main() {
    #[cfg(target_os = "macos")]
    macos_build::run();
}

#[cfg(target_os = "macos")]
mod macos_build {
    use std::{
        env,
        path::{Path, PathBuf},
    };

    use cbindgen::Config;

    pub fn run() {
        let header_path = generate_shader_bindings();

        #[cfg(feature = "runtime_shaders")]
        emit_stitched_shaders(&header_path);
        #[cfg(not(feature = "runtime_shaders"))]
        compile_metal_shaders(&header_path);
    }

    fn generate_shader_bindings() -> PathBuf {
        let output_path = PathBuf::from(env::var("OUT_DIR").unwrap()).join("scene.h");

        let gpui_dir = find_gpui_crate_dir();

        let mut config = Config {
            include_guard: Some("SCENE_H".into()),
            language: cbindgen::Language::C,
            no_includes: true,
            ..Default::default()
        };
        config.export.include.extend([
            "Bounds".into(),
            "Corners".into(),
            "Edges".into(),
            "Size".into(),
            "Pixels".into(),
            "PointF".into(),
            "Hsla".into(),
            "ContentMask".into(),
            "Uniforms".into(),
            "AtlasTile".into(),
            "PathRasterizationInputIndex".into(),
            "PathVertex_ScaledPixels".into(),
            "PathRasterizationVertex".into(),
            "ShadowInputIndex".into(),
            "Shadow".into(),
            "QuadInputIndex".into(),
            "Underline".into(),
            "UnderlineInputIndex".into(),
            "Quad".into(),
            "BorderStyle".into(),
            "SpriteInputIndex".into(),
            "MonochromeSprite".into(),
            "PolychromeSprite".into(),
            "PathSprite".into(),
            "SurfaceInputIndex".into(),
            "SurfaceBounds".into(),
            "TransformationMatrix".into(),
        ]);
        config.no_includes = true;
        config.enumeration.prefix_with_name = true;

        let mut builder = cbindgen::Builder::new();

        let crate_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());

        // Source files from gpui that define types used in shaders
        let gpui_src_paths = [
            gpui_dir.join("src/scene.rs"),
            gpui_dir.join("src/geometry.rs"),
            gpui_dir.join("src/color.rs"),
            gpui_dir.join("src/window.rs"),
            gpui_dir.join("src/platform.rs"),
        ];

        // Source files from this crate
        let local_src_paths = [crate_dir.join("src/metal_renderer.rs")];

        for src_path in gpui_src_paths.iter().chain(local_src_paths.iter()) {
            println!("cargo:rerun-if-changed={}", src_path.display());
            builder = builder.with_src(src_path);
        }

        builder
            .with_config(config)
            .generate()
            .expect("Unable to generate bindings")
            .write_to_file(&output_path);

        output_path
    }

    /// Locate the gpui crate directory relative to this crate. Resolved at
    /// build-script runtime against this crate's manifest dir, so no checkout
    /// path is baked into a compiled artifact (which corgi rejects).
    fn find_gpui_crate_dir() -> PathBuf {
        if let Some(path) = std::env::var_os("GPUI_PRE_SOURCE_DIR") {
            return PathBuf::from(path);
        }

        let cargo_home = std::env::var_os("CARGO_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cargo")))
            .expect("CARGO_HOME or HOME must be set to locate gpui-pre source");
        let registry_src = cargo_home.join("registry").join("src");

        for index in std::fs::read_dir(&registry_src)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", registry_src.display()))
        {
            let index = index.expect("failed to read Cargo registry entry").path();
            let candidate = index.join("gpui-pre-0.3.6");
            if candidate.join("src/scene.rs").is_file() {
                return candidate;
            }
        }

        panic!(
            "gpui-pre 0.3.6 source not found under {}; set GPUI_PRE_SOURCE_DIR to its crate directory",
            registry_src.display()
        );
    }

    /// To enable runtime compilation, we need to "stitch" the shaders file with the generated header
    /// so that it is self-contained.
    #[cfg(feature = "runtime_shaders")]
    fn emit_stitched_shaders(header_path: &Path) {
        fn stitch_header(header: &Path, shader_path: &Path) -> std::io::Result<PathBuf> {
            let header_contents = std::fs::read_to_string(header)?;
            let shader_contents = std::fs::read_to_string(shader_path)?;
            let stitched_contents = format!("{header_contents}\n{shader_contents}");
            let out_path =
                PathBuf::from(env::var("OUT_DIR").unwrap()).join("stitched_shaders.metal");
            std::fs::write(&out_path, stitched_contents)?;
            Ok(out_path)
        }
        let shader_source_path = "./src/shaders.metal";
        let shader_path = PathBuf::from(shader_source_path);
        stitch_header(header_path, &shader_path).unwrap();
        println!("cargo:rerun-if-changed={shader_source_path}");
    }

    #[cfg(not(feature = "runtime_shaders"))]
    fn compile_metal_shaders(header_path: &Path) {
        use std::process::{self, Command};
        let shader_path = "./src/shaders.metal";
        let air_output_path = PathBuf::from(env::var("OUT_DIR").unwrap()).join("shaders.air");
        let metallib_output_path =
            PathBuf::from(env::var("OUT_DIR").unwrap()).join("shaders.metallib");
        println!("cargo:rerun-if-changed={}", shader_path);

        // The metal compiler records the resolved absolute path of its input
        // unconditionally. Compile a copy staged in OUT_DIR so the recorded
        // location is the build's canonical output directory, never the
        // checkout (corgi rejects artifacts that embed the build path).
        let staged_shader_path = PathBuf::from(env::var("OUT_DIR").unwrap()).join("shaders.metal");
        std::fs::copy(shader_path, &staged_shader_path).unwrap();

        let output = Command::new("xcrun")
            .args([
                "-sdk",
                "macosx",
                "metal",
                "-gline-tables-only",
                "-mmacosx-version-min=10.15.7",
                "-MO",
                "-c",
            ])
            .arg(&staged_shader_path)
            .args(["-include", header_path.to_str().unwrap(), "-o"])
            .arg(&air_output_path)
            .output()
            .unwrap();

        if !output.status.success() {
            println!(
                "cargo::error=metal shader compilation failed:\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
            process::exit(1);
        }

        let output = Command::new("xcrun")
            .args(["-sdk", "macosx", "metallib"])
            .arg(&air_output_path)
            .arg("-o")
            .arg(metallib_output_path)
            .output()
            .unwrap();

        if !output.status.success() {
            println!(
                "cargo::error=metallib compilation failed:\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
            process::exit(1);
        }

        // The .air intermediate records the compiler's working directory in
        // its debug info; the metallib built from it does not. Nothing reads
        // the .air after this point, so drop it rather than leave a
        // checkout-path-bearing file in OUT_DIR.
        std::fs::remove_file(&air_output_path).unwrap();
    }
}
