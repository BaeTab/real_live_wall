//! Compile-check every built-in scene without a GPU: the WGSL default scene and
//! each `shaders/*.glsl` Shadertoy scene are parsed and validated through naga,
//! exactly like wgpu does at pipeline-creation time. `cargo test` therefore
//! catches shader syntax/type errors before the app is ever launched.

use wgpu::naga;

fn validate(module: &naga::Module, label: &str) {
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    if let Err(e) = validator.validate(module) {
        panic!("{label}: validation error: {e:?}");
    }
}

#[test]
fn default_wgsl_scene_compiles() {
    for (label, src) in [
        ("FULLSCREEN_VS", real_live_wall::shader::FULLSCREEN_VS),
        ("DEFAULT_WGSL_FS", real_live_wall::shader::DEFAULT_WGSL_FS),
    ] {
        let module = naga::front::wgsl::parse_str(src)
            .unwrap_or_else(|e| panic!("{label}: WGSL parse error: {e}"));
        validate(&module, label);
    }
}

#[test]
fn all_glsl_scenes_compile() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders");
    let mut checked = 0;
    for entry in std::fs::read_dir(&dir).expect("shaders/ directory") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("glsl") {
            continue;
        }
        let label = path.file_name().unwrap().to_string_lossy().into_owned();
        let user_src = std::fs::read_to_string(&path).expect("read shader");
        let wrapped = real_live_wall::shader::wrap_shadertoy_glsl(&user_src);

        let mut frontend = naga::front::glsl::Frontend::default();
        let options = naga::front::glsl::Options::from(naga::ShaderStage::Fragment);
        let module = frontend
            .parse(&options, &wrapped)
            .unwrap_or_else(|e| panic!("{label}: GLSL parse error: {e:?}"));
        validate(&module, &label);
        checked += 1;
    }
    assert!(checked >= 5, "expected the built-in scenes, found {checked}");
}
