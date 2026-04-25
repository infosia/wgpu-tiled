/*!
Tests for the opt-in `Options::allow_unresolved_overrides` flag on the
SPIR-V and MSL backends. The flag preserves WGSL `override` declarations as
native specialization constants instead of returning `Error::Override`.

Coverage:
- SPIR-V emits `OpDecorate ... SpecId N` plus `OpSpecConstant*` per override.
- MSL emits `[[function_constant(N)]]` per override.
- Each scalar kind (`bool`, `i32`, `u32`, `f32`) maps to the right opcode.
- Implicit `@id` allocation is deterministic across repeated parses, and the
  SpecId number in SPIR-V matches the `function_constant` slot in MSL.
- Default-off behaviour still returns `Error::Override`.
- Restrictions (`@workgroup_size`, array length) error out with the new
  variants even when the flag is on.
*/

#![cfg(all(feature = "wgsl-in", spv_out, msl_out))]

use rspirv::binary::Disassemble;

fn parse_module(source: &str) -> (naga::Module, naga::valid::ModuleInfo) {
    let module = naga::front::wgsl::parse_str(source).unwrap_or_else(|err| {
        panic!(
            "expected WGSL to parse successfully:\n{}",
            err.emit_to_string(source)
        );
    });
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .expect("validation failed");
    (module, info)
}

fn write_spv(module: &naga::Module, info: &naga::valid::ModuleInfo) -> String {
    use naga::back::spv;
    let options = spv::Options {
        allow_unresolved_overrides: true,
        ..Default::default()
    };
    let words = spv::write_vec(module, info, &options, None).expect("SPIR-V emission failed");
    rspirv::dr::load_words(words)
        .expect("Produced invalid SPIR-V")
        .disassemble()
}

fn write_msl(module: &naga::Module, info: &naga::valid::ModuleInfo) -> String {
    use naga::back::msl;
    let options = msl::Options {
        allow_unresolved_overrides: true,
        lang_version: (2, 0),
        ..Default::default()
    };
    let pipeline_options = msl::PipelineOptions::default();
    let (output, _) = msl::write_string(module, info, &options, &pipeline_options)
        .expect("MSL emission failed");
    output
}

#[test]
fn override_spirv_emits_spec_constant() {
    let source = "@id(0) override scale: u32 = 5u; \
                  @compute @workgroup_size(1) fn main() { _ = scale; }";
    let (module, info) = parse_module(source);
    let spv = write_spv(&module, &info);
    assert!(
        spv.contains("OpDecorate") && spv.contains("SpecId 0"),
        "expected `OpDecorate ... SpecId 0` in SPIR-V output:\n{spv}",
    );
    assert!(
        spv.contains("OpSpecConstant"),
        "expected `OpSpecConstant` in SPIR-V output:\n{spv}",
    );
}

#[test]
fn override_msl_emits_function_constant() {
    let source = "@id(0) override scale: u32 = 5u; \
                  @compute @workgroup_size(1) fn main() { _ = scale; }";
    let (module, info) = parse_module(source);
    let msl = write_msl(&module, &info);
    assert!(
        msl.contains("[[function_constant(0)]]"),
        "expected `[[function_constant(0)]]` in MSL output:\n{msl}",
    );
}

#[test]
fn override_scalar_coverage() {
    let source = "@id(0) override b: bool = true; \
                  @id(1) override i: i32 = -3; \
                  @id(2) override u: u32 = 7u; \
                  @id(3) override f: f32 = 1.5; \
                  @compute @workgroup_size(1) fn main() { _ = b; _ = i; _ = u; _ = f; }";
    let (module, info) = parse_module(source);

    let spv = write_spv(&module, &info);
    assert!(spv.contains("OpSpecConstantTrue"), "{spv}");
    for spec_id in 0..=3 {
        assert!(
            spv.contains(&format!("SpecId {spec_id}")),
            "missing SpecId {spec_id} in SPIR-V:\n{spv}",
        );
    }

    let msl = write_msl(&module, &info);
    for slot in 0..=3 {
        assert!(
            msl.contains(&format!("[[function_constant({slot})]]")),
            "missing function_constant({slot}) in MSL:\n{msl}",
        );
    }
}

#[test]
fn override_implicit_id_is_deterministic() {
    // Implicit IDs should be assigned at parse time and remain stable across
    // repeated parses so SPIR-V and MSL outputs agree on the spec slot.
    let source = "override a: u32 = 1u; \
                  @id(7) override b: u32 = 2u; \
                  override c: u32 = 3u; \
                  @compute @workgroup_size(1) fn main() { _ = a; _ = b; _ = c; }";

    let (module1, info1) = parse_module(source);
    let (module2, info2) = parse_module(source);

    let ids1: Vec<_> = module1.overrides.iter().map(|(_, ov)| ov.id).collect();
    let ids2: Vec<_> = module2.overrides.iter().map(|(_, ov)| ov.id).collect();
    assert_eq!(ids1, ids2, "implicit @id assignment must be deterministic");

    // Sanity: smallest unused integers are assigned in declaration order.
    assert_eq!(ids1, vec![Some(0), Some(7), Some(1)]);

    // The SpecId number in SPIR-V must equal the `function_constant` slot
    // in MSL for every override; otherwise runtime specialization can't bridge
    // the two backends.
    let spv = write_spv(&module1, &info1);
    let msl = write_msl(&module1, &info1);
    for id in [0u16, 7, 1] {
        assert!(
            spv.contains(&format!("SpecId {id}")),
            "SPIR-V missing SpecId {id}:\n{spv}",
        );
        assert!(
            msl.contains(&format!("[[function_constant({id})]]")),
            "MSL missing function_constant({id}):\n{msl}",
        );
    }
    let _ = info2;
}

#[test]
fn override_default_off_unchanged() {
    use naga::back::spv;
    let source = "@id(0) override scale: u32 = 5u; \
                  @compute @workgroup_size(1) fn main() { _ = scale; }";
    let (module, info) = parse_module(source);

    let options = spv::Options::default();
    let result = spv::write_vec(&module, &info, &options, None);
    assert!(
        matches!(result, Err(spv::Error::Override)),
        "expected `Err(Error::Override)` with `allow_unresolved_overrides: false`, got {result:?}",
    );
}

#[test]
fn override_in_workgroup_size_rejected() {
    use naga::back::spv;
    let source = "override wg_x: u32 = 4u; \
                  @compute @workgroup_size(wg_x) fn main() {}";
    let (module, info) = parse_module(source);
    let options = spv::Options {
        allow_unresolved_overrides: true,
        ..Default::default()
    };
    let result = spv::write_vec(&module, &info, &options, None);
    assert!(
        matches!(result, Err(spv::Error::OverrideInWorkgroupSizeUnsupported(_))),
        "expected `OverrideInWorkgroupSizeUnsupported`, got {result:?}",
    );
}

#[test]
fn override_in_array_length_rejected() {
    use naga::back::spv;
    let source = "override arr_len: u32 = 4u; \
                  var<workgroup> data: array<u32, arr_len>; \
                  @compute @workgroup_size(1) fn main() { data[0] = 1u; }";
    let (module, info) = parse_module(source);
    let options = spv::Options {
        allow_unresolved_overrides: true,
        ..Default::default()
    };
    let result = spv::write_vec(&module, &info, &options, None);
    assert!(
        matches!(result, Err(spv::Error::OverrideAsArrayLengthUnsupported(_))),
        "expected `OverrideAsArrayLengthUnsupported`, got {result:?}",
    );
}
