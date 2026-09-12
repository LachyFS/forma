//! Validate every portable kernel and generate all three native shader languages
//! without requiring a GPU or a macOS/Windows host. Hardware behavior is covered
//! by the separate GPU and material-preview integration suites.
use naga::{
    AddressSpace, ShaderStage, StorageAccess, TypeInner,
    back::{hlsl, msl, spv},
    valid::{Capabilities, ValidationFlags, Validator},
};

const SHADER: &str = include_str!("../src/shader.wgsl");
const PREVIEW: &str = include_str!("../src/preview.wgsl");
const IBL: &str = include_str!("../src/ibl.wgsl");

fn validate_and_translate(source: &str, expected_entries: &[&str]) {
    let module = naga::front::wgsl::parse_str(source)
        .unwrap_or_else(|error| panic!("{}", error.emit_to_string(source)));
    let info = Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .unwrap_or_else(|error| panic!("{}", error.emit_to_string(source)));
    let mut msl_options = msl::Options {
        lang_version: (2, 1),
        fake_missing_bindings: false,
        ..Default::default()
    };
    let mut hlsl_options = hlsl::Options {
        fake_missing_bindings: false,
        ..Default::default()
    };
    let mut resources = msl::EntryPointResources {
        sizes_buffer: Some(30),
        ..Default::default()
    };
    for (_, global) in module.global_variables.iter() {
        let Some(binding) = global.binding else {
            continue;
        };
        let slot = (binding.group * 16 + binding.binding) as u8;
        let is_image = matches!(module.types[global.ty].inner, TypeInner::Image { .. });
        resources.resources.insert(
            binding,
            msl::BindTarget {
                buffer: (!is_image).then_some(slot),
                texture: is_image.then_some(slot),
                mutable: matches!(global.space, AddressSpace::Storage { access } if access.contains(StorageAccess::STORE)),
                ..Default::default()
            },
        );
        hlsl_options.binding_map.insert(
            binding,
            hlsl::BindTarget {
                space: binding.group as u8,
                register: binding.binding,
                binding_array_size: None,
                dynamic_storage_buffer_offsets_index: None,
                restrict_indexing: true,
            },
        );
    }
    for entry in &module.entry_points {
        msl_options
            .per_entry_point_map
            .insert(entry.name.clone(), resources.clone());
    }
    let actual: Vec<_> = module
        .entry_points
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    assert_eq!(actual, expected_entries);
    for entry in &module.entry_points {
        assert_eq!(entry.stage, ShaderStage::Compute);
        assert_eq!(entry.workgroup_size, [8, 8, 1]);
        let spv_words = spv::write_vec(
            &module,
            &info,
            &spv::Options::default(),
            Some(&spv::PipelineOptions {
                shader_stage: ShaderStage::Compute,
                entry_point: entry.name.clone(),
            }),
        )
        .unwrap_or_else(|error| panic!("SPIR-V {}: {error}", entry.name));
        assert_eq!(spv_words[0], 0x0723_0203);
        let (metal_source, metal_info) = msl::write_string(
            &module,
            &info,
            &msl_options,
            &msl::PipelineOptions {
                entry_point: Some((ShaderStage::Compute, entry.name.clone())),
                ..Default::default()
            },
        )
        .unwrap_or_else(|error| panic!("MSL {}: {error}", entry.name));
        assert!(!metal_source.is_empty());
        assert!(metal_info.entry_point_names.iter().all(Result::is_ok));
        let mut hlsl_source = String::new();
        let hlsl_pipeline = hlsl::PipelineOptions {
            entry_point: Some((ShaderStage::Compute, entry.name.clone())),
        };
        let hlsl_info = hlsl::Writer::new(&mut hlsl_source, &hlsl_options, &hlsl_pipeline)
            .write(&module, &info, None)
            .unwrap_or_else(|error| panic!("HLSL {}: {error}", entry.name));
        assert!(!hlsl_source.is_empty());
        assert!(hlsl_info.entry_point_names.iter().all(Result::is_ok));
    }
}

#[test]
fn render_and_preview_compile_for_vulkan_metal_and_dx12() {
    validate_and_translate(
        &format!("{SHADER}\n{PREVIEW}"),
        &["render_main", "preview_main"],
    );
}

#[test]
fn environment_bake_compiles_for_vulkan_metal_and_dx12() {
    validate_and_translate(
        &format!("{SHADER}\n{IBL}"),
        &["render_main", "bake_diffuse", "bake_specular", "bake_brdf"],
    );
}

#[test]
fn shader_structs_preserve_native_scene_buffer_layouts() {
    let module = naga::front::wgsl::parse_str(SHADER).unwrap();
    for (name, expected_span, expected_offsets) in [
        ("Triangle", 192, (0..12).map(|i| i * 16).collect::<Vec<_>>()),
        ("GpuMaterial", 112, vec![0, 16, 32, 48, 64, 80]),
        ("BvhNode", 48, vec![0, 16, 32]),
        ("Uniforms", 176, (0..11).map(|i| i * 16).collect()),
    ] {
        let ty = module
            .types
            .iter()
            .find_map(|(_, ty)| (ty.name.as_deref() == Some(name)).then_some(ty))
            .unwrap();
        let TypeInner::Struct { members, span } = &ty.inner else {
            panic!("{name} must be a struct");
        };
        assert_eq!(*span, expected_span, "{name}");
        assert_eq!(
            members
                .iter()
                .map(|member| member.offset)
                .collect::<Vec<_>>(),
            expected_offsets,
            "{name}"
        );
    }
}
