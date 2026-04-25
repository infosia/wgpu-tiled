# Coding-Agent Instructions: MSAA Subpass Input Sample-Frequency Enforcement

| Field | Value |
|-------|-------|
| Closes | TILED.md gap #8 (MSAA subpass inputs — sample-frequency execution not enforced) |
| Repo | local checkout of this fork (run from the workspace root) |
| Branch base | `main` (private fork; `feature/subpass` work continues here) |
| Backward compatibility | Not required. The current behavior rejects `subpass_input_multisampled*` outright, so there is no working user code to preserve. |
| Companion docs | `TILED.md` (gap list), `subpass-input-typed-redesign.md` (typed surface RFC), `naga/src/ir/mod.rs` (subpass IR doc-comments) |

---

## 1. Goal

Lift the blanket rejection of `subpass_input_multisampled*` types and make MSAA subpass-input shaders work correctly across Vulkan and Metal — the two backends actively in scope for this fork. The portable semantic is **"`subpassLoad` on a multisampled input reads the current sample only"**, and the cross-backend trigger is **"the fragment entry point declares `@builtin(sample_index)` as one of its inputs"** (the value need not appear in the body — declaration is what triggers per-sample execution on both backends).

GLES is **not a current focus** for the fork (see project memory). Treat the GLSL backend as out-of-scope for MSAA subpass inputs: emit a hard error and do not invest in framebuffer-fetch or multipass per-sample lowering work. The non-MSAA GLSL paths must keep compiling but feature parity is not a goal here.

---

## 2. The contract this PR establishes

A WGSL fragment entry point that reaches a `subpass_input_multisampled<T>` / `subpass_input_depth_multisampled` / `subpass_input_stencil_multisampled` global must declare `@builtin(sample_index)` as one of its inputs. If absent, the validator rejects with a clear, actionable error. If present:

- **Vulkan**: pipeline creation sets `VkPipelineMultisampleStateCreateInfo::sampleShadingEnable = VK_TRUE` and `minSampleShading = 1.0`.
- **Metal**: the existing MSL emit already declares `[[sample_id]]` when `BuiltIn::SampleIndex` is referenced; no backend changes needed.
- **SPIR-V**: emit `OpExecutionMode <entry> SampleRateShading`.
- **GLSL**: emit a hard backend error in both `use_framebuffer_fetch: true` and `false` modes. GLES is out of scope for this fork; do not invest in a per-sample lowering.

Sample shape:

```wgsl
@group(0) @binding(0) var albedo_ms: subpass_input_multisampled<f32>;

@fragment
fn fs_main(@builtin(sample_index) _sid: u32) -> @location(0) vec4<f32> {
    return subpassLoad(albedo_ms);
}
```

---

## 3. Phase-by-phase plan

Implement in order. Each phase ends with `cargo build -p naga` and `cargo test -p naga` passing before moving on.

### Phase 1 — Validator rule (foundation)

Files: `naga/src/valid/interface.rs`, `naga/src/valid/mod.rs`

1. Add a new error variant on the appropriate validator error enum (`EntryPointError` or a sub-enum reachable from it):
   ```rust
   MsaaSubpassInputRequiresSampleIndex {
       entry_point: String,
       image_class: crate::ImageClass,
   }
   ```
   Display message: `"fragment entry point '{entry_point}' reaches MSAA subpass-input image class {image_class:?} but does not declare @builtin(sample_index) as an input. Add a @builtin(sample_index) parameter to the entry point — the value need not be used; declaring it is what triggers per-sample execution."`

2. In `validate_entry_point` (or whichever function performs the per-entry-point reachability walk in `interface.rs`), after collecting the set of globals reachable from this entry point: if any of those globals is an `Image` whose `class` is `SubpassInput { multi: true, .. }`, `SubpassInputDepth { multi: true }`, or `SubpassInputStencil { multi: true }`, scan `entry_point.function.arguments` for one whose binding is `Binding::BuiltIn(BuiltIn::SampleIndex)`. Reject with the new error if absent.

3. Locate and remove the existing placeholder that rejects `multi: true` subpass inputs outright. Search starting points: `naga/src/valid/interface.rs` and `naga/src/valid/expression.rs` for the strings `"MSAA"`, `"multi: true"`, `"sample-frequency"`. Replace the placeholder with the rule above.

### Phase 2 — SPIR-V emit

File: `naga/src/back/spv/writer.rs`

1. When emitting a fragment entry point, scan reachable globals for any MSAA subpass input. If at least one is found, ensure `OpExecutionMode <entry> SampleRateShading` is emitted. The existing path that emits `SampleRateShading` for explicit `SampleId` reads is the right place — generalize the trigger so that an MSAA subpass-input reference also implies it.

2. Verify the existing `OpImageRead` synthesized `(0, 0)` coordinate path for `SubpassData` works for multisampled images. Per SPIR-V spec, `OpImageRead` on a multisampled image at `Dim=SubpassData` requires no sample operand when `SampleRateShading` is on (the implicit sample is the current one).

### Phase 3 — Metal emit

File: `naga/src/back/msl/writer.rs`

No code changes expected. The existing emit already declares `[[sample_id]]` when the shader references `BuiltIn::SampleIndex`. Since the validator now requires that builtin, the MSL output gains the `[[sample_id]]` argument automatically.

Verify: the snapshot for the new MSAA test (Phase 7) should show both the `[[color(K)]]` subpass-input argument and the `[[sample_id]]` builtin argument on the fragment function.

### Phase 4 — GLSL hard error (GLES out of scope)

Files: `naga/src/back/glsl/writer.rs`, `naga/src/back/glsl/mod.rs`

GLES is not a current focus for the fork; do not invest in a per-sample lowering. Just refuse the input.

1. Add an error variant to the GLSL backend's error type (likely `naga::back::glsl::Error`):
   ```rust
   MsaaSubpassInputUnsupported,
   ```
   Display message: `"MSAA subpass inputs are not supported by the GLSL backend (GLES is out of scope for this fork)"`.

2. In the writer's subpass-input lowering paths, when emitting a `subpass_input_multisampled*` global return this error in **both** `use_framebuffer_fetch: true` and `use_framebuffer_fetch: false` modes.

3. Do **not** add new GLSL snapshot tests for MSAA. The non-MSAA GLSL paths must keep compiling and existing snapshots must not regress, but new GLSL coverage is not a goal.

### Phase 5 — Vulkan HAL

File: `wgpu-hal/src/vulkan/device.rs`

1. In `create_render_pipeline`, find the existing block that walks the fragment shader's naga module to populate `pipeline_input_attachments` (around line 2227 — look for `for (_, global) in naga_shader.module.global_variables.iter()`).

2. Track whether any reached subpass input is multisampled. If any are, set `VkPipelineMultisampleStateCreateInfo::sample_shading_enable = vk::TRUE` and `min_sample_shading = 1.0` on the multisample state used at `vk::GraphicsPipelineCreateInfo` build.

3. The existing multisample state is built somewhere later in `create_render_pipeline` — look for `vk::PipelineMultisampleStateCreateInfo`. Add a conditional that overrides the sample-shading fields when the multisampled-subpass-input flag is set.

### Phase 6 — Metal HAL

No code changes expected. Metal pipeline creation already wires per-sample execution through the `[[sample_id]]` argument that naga emits. Confirm with a manual run of the deferred_rendering example with MSAA enabled (see Phase 7 step 4).

### Phase 7 — Tests, snapshots, examples, docs

1. **New WGSL test input**: `naga/tests/in/wgsl/subpass-color-msaa.wgsl` and matching `subpass-color-msaa.toml`. Use `subpass_input_multisampled<f32>` plus `@builtin(sample_index)` and a fragment entry point that calls `subpassLoad`.

2. **Regenerate snapshots** under `naga/tests/out/{wgsl,msl,spv}/wgsl-subpass-color-msaa.*`. (No GLSL snapshot — GLSL backend rejects.)

3. **New error tests** in `naga/tests/naga/wgsl_errors.rs`:
   - `msaa_subpass_input_without_sample_index_rejected`: declares `subpass_input_multisampled<f32>` but the fragment entry point omits `@builtin(sample_index)`. Expect `MsaaSubpassInputRequiresSampleIndex`.
   - `msaa_subpass_input_with_sample_index_accepted`: same but with the builtin declared. Expect validation success.
   - `msaa_subpass_input_diagnostic_includes_actionable_hint`: assert via `emit_to_string` that the formatted error mentions "declare a `@builtin(sample_index)` parameter".

4. **deferred_rendering example MSAA flag** (optional but high-value):
   - Add a `--msaa` CLI flag (or a const in `examples/features/src/deferred_rendering/mod.rs`) that bumps `sample_count` from 1 to 4 on the G-buffer attachments and switches the lighting/composite shaders to `subpass_input_multisampled<f32>` plus `@builtin(sample_index)`.
   - Run `WGPU_BACKEND=metal cargo run --bin wgpu-examples -- deferred_rendering --msaa` and `WGPU_BACKEND=vulkan cargo run --bin wgpu-examples -- deferred_rendering --msaa` interactively to confirm both backends render correctly.
   - If wiring the flag is too invasive, keep the example single-sample and document MSAA as "verified via naga snapshots only".

5. **TILED.md update**:
   - Move gap #8 to the Resolved section. Cite the `@builtin(sample_index)` rule, the SPIR-V `SampleRateShading` execution mode, the Vulkan `sampleShadingEnable` plumbing, and the GLSL hard-error decision.
   - Note in the Resolved entry that GLSL/GLES is not in scope for this fork, so MSAA subpass inputs are deliberately rejected on that backend. Do not file a new "remaining" gap for it.

---

## 4. Verification

After each phase:
```sh
cargo build -p naga
cargo test -p naga
```

After full PR:
```sh
cargo test --workspace --exclude wgpu-test --exclude cts_runner
cargo clippy --workspace --all-targets -- -D warnings
```

Manual smoke test (interactive, foreground terminal — background launch from a non-foreground shell will not trigger window resume on macOS):
```sh
WGPU_BACKEND=metal  cargo run --bin wgpu-examples deferred_rendering
WGPU_BACKEND=vulkan cargo run --bin wgpu-examples deferred_rendering
# If --msaa flag added:
WGPU_BACKEND=metal  cargo run --bin wgpu-examples -- deferred_rendering --msaa
WGPU_BACKEND=vulkan cargo run --bin wgpu-examples -- deferred_rendering --msaa
```

Both should display the deferred-shading scene without validation errors.

Optional SPIR-V validity check (does not require GPU):
```sh
cargo run -p naga-cli -- naga/tests/in/wgsl/subpass-color-msaa.wgsl --output /tmp/claude/check.spv
spirv-val /tmp/claude/check.spv
# Expect: "execution mode SampleRateShading"-bearing SPIR-V that passes spirv-val.
```

---

## 5. Review-fix loop (per `CLAUDE.md` "Clean Review Then Fix")

1. After implementation, spawn a fresh reviewer agent with no context. It reads every changed/new file from scratch and produces severity-tagged findings (`[critical]`, `[major]`, `[minor]`, `[nit]`).
2. Address findings critical-first. Skip nits unless the user requests them.
3. Re-run `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings`.
4. Run the GitHub Copilot second-opinion pass per the recipe in `CLAUDE.md` (write the prompt to `/tmp/claude/copilot_review_prompt.md`, invoke read-only).

---

## 6. Private-information check (before committing)

Scan all changed and new files for filesystem paths (`/Users/`, `/home/`, `C:\Users\`), real person names, hardcoded secrets / API keys / tokens, personal email addresses, internal URLs / IPs. Remove or replace anything found.

---

## 7. Commits

Two commits, in order:

1. `feat(naga): enforce sample-frequency execution for MSAA subpass inputs`
   - All naga + wgpu-hal + tests + snapshots + example changes.
2. `docs(tiled): mark gap #8 resolved`
   - TILED.md only.

Use HEREDOC for commit messages. Do not amend; do not force-push; do not skip hooks.

---

## 8. Deliverable

- All workspace tests green.
- New error test cases lock in both the rejection (without `@builtin(sample_index)`) and the acceptance (with it).
- `cargo run -p naga-cli ... | spirv-val` confirms `OpExecutionMode SampleRateShading` for the new MSAA snapshot.
- `WGPU_BACKEND=metal` and `WGPU_BACKEND=vulkan` both render the deferred_rendering example without panics. (If the optional `--msaa` example flag is wired, both render with MSAA on.)
- TILED.md updated.
- PR description names: the new validator rule, the Vulkan pipeline-state plumbing, the GLSL deliberate rejection, and the SPIR-V execution-mode emission.

---

## 9. Non-goals (do not touch in this PR)

- Per-sample MSAA reads (TILED.md gap #9 — permanently out of scope).
- DX12 / HLSL subpass support (gaps #7 / #11).
- Public `wgpu::TransientAttachment` handle (gap #6 — Out of Scope).
- Layered / multiview subpass inputs (gap #10).
- GLSL MSAA subpass inputs — deliberately rejected (GLES is not a current focus for this fork).
- Any new GLES feature work — out of scope.

---

## 10. Risks / open questions

1. **Mali / Adreno subpass MSAA quirks.** Some mobile drivers have known issues with `sampleShadingEnable` combined with transient attachments. Manual smoke test on real hardware is recommended before claiming the gap fully closed. If issues surface, document them as a new TILED.md gap rather than backing out.
2. **Existing pipeline-multisample plumbing in Vulkan HAL.** The exact shape of `VkPipelineMultisampleStateCreateInfo` construction in `wgpu-hal/src/vulkan/device.rs::create_render_pipeline` may differ from what this document assumes. Read the current code before editing; the principle (set `sample_shading_enable=VK_TRUE` and `min_sample_shading=1.0` when an MSAA subpass input is reached) is what matters.
3. **SPIR-V backend already emitting `SampleRateShading` via existing logic.** It is possible the existing "scan for `SampleId` reads" path already covers MSAA-subpass-input shaders by virtue of the required `@builtin(sample_index)` declaration. Verify before adding new emit logic — Phase 2 may collapse to "no changes needed; rely on existing path".

If any step turns out to be ambiguous, prefer the simpler interpretation and document the choice in the PR description rather than inventing extra surface area.

---

## 11. Self-contained agent prompt

Pass this whole document to the agent. The block below summarizes the role and rules so the agent can run end-to-end without further context.

````
ROLE: You are a senior Rust engineer working on a private wgpu fork
(`feature/subpass`, base v29.0.1). You are landing a single PR that
enforces sample-frequency execution for MSAA subpass inputs. Backward
compatibility is not required — the previous behavior was a blanket
rejection of multisampled subpass inputs.

REPO: <local checkout of this fork; cwd should be the workspace root>
DOCS TO READ FIRST (in order):
  1. subpass-input-msaa.md           — this document (the plan).
  2. TILED.md                        — gap #8 in the Remaining list.
  3. subpass-input-typed-redesign.md — prior RFC for the typed surface.
  4. naga/src/ir/mod.rs               — subpass IR doc-comments.

GOAL (one sentence):
  Make `subpass_input_multisampled*` work end-to-end on Vulkan and
  Metal by enforcing that fragment entry points reaching them declare
  `@builtin(sample_index)`, then plumb the resulting per-sample
  execution through the SPIR-V execution mode and the Vulkan pipeline
  multisample state. Reject MSAA subpass inputs from the GLSL backend.

CONSTRAINTS:
  - One PR. No staged migration.
  - Every public API change has unit tests in the same PR
    (per CLAUDE.md "Testing Policy").
  - No comments beyond non-obvious WHY (per CLAUDE.md style rules).
  - Use Edit / Write / Read tools for files; do not use cat / sed / echo.
  - Run sandboxed Bash for builds and tests.
  - Before committing, run the "Private Information Check" from CLAUDE.md.

EXECUTION ORDER (from §3 of this document; follow phase by phase):

  Phase 1 — Validator rule
    - Add MsaaSubpassInputRequiresSampleIndex error variant.
    - In valid/interface.rs entry-point analysis, require
      @builtin(sample_index) when an MSAA subpass-input global is
      reachable from a fragment entry point.
    - Remove the existing blanket rejection of `multi: true` subpass
      inputs.

  Phase 2 — SPIR-V emit
    - Ensure OpExecutionMode <entry> SampleRateShading is emitted when
      the fragment entry point reaches an MSAA subpass input. Reuse
      existing SampleId-driven path if it already covers this.

  Phase 3 — Metal emit
    - No code changes expected. Verify the snapshot output shows both
      [[color(K)]] and [[sample_id]] arguments on the fragment function.

  Phase 4 — GLSL hard error (GLES out of scope)
    - Add naga::back::glsl::Error::MsaaSubpassInputUnsupported.
    - Emit it from both fbfetch and subpassInput lowering paths when
      the source class is `SubpassInput*` with `multi: true`.
    - Do not add new GLSL snapshot tests; do not implement per-sample
      lowering for GLES.

  Phase 5 — Vulkan HAL
    - In wgpu-hal/src/vulkan/device.rs::create_render_pipeline, set
      sample_shading_enable=VK_TRUE and min_sample_shading=1.0 on the
      VkPipelineMultisampleStateCreateInfo whenever the fragment shader
      reaches an MSAA subpass input.

  Phase 6 — Metal HAL
    - No code changes expected.

  Phase 7 — Tests, snapshots, example, docs
    - Add naga/tests/in/wgsl/subpass-color-msaa.wgsl + .toml.
    - Regenerate naga/tests/out/{wgsl,msl,spv}/wgsl-subpass-color-msaa.*.
    - Add naga/tests/naga/wgsl_errors.rs cases:
        msaa_subpass_input_without_sample_index_rejected
        msaa_subpass_input_with_sample_index_accepted
        msaa_subpass_input_diagnostic_includes_actionable_hint
    - Optional: add --msaa flag to deferred_rendering example.
    - Move TILED.md gap #8 to Resolved.

VERIFICATION (must all pass at end):
    cargo build -p naga
    cargo test -p naga
    cargo test --workspace --exclude wgpu-test --exclude cts_runner
    cargo clippy --workspace --all-targets -- -D warnings
  Optional:
    cargo run -p naga-cli -- naga/tests/in/wgsl/subpass-color-msaa.wgsl
      --output /tmp/claude/check.spv
    spirv-val /tmp/claude/check.spv

REVIEW-FIX LOOP (per CLAUDE.md "Clean Review Then Fix"):
  1. Spawn a fresh reviewer agent with no context. Severity-tag findings.
  2. Address critical → major → minor.
  3. Re-run tests + clippy.
  4. Run GitHub Copilot second opinion per CLAUDE.md recipe.

PRIVATE INFORMATION CHECK (per CLAUDE.md, before committing):
  Scan all changed and new files for:
    - filesystem paths (/Users/, /home/, C:\Users\)
    - real person names
    - hardcoded secrets / passwords / API keys
    - personal email addresses
    - internal URLs / IPs
  Remove or replace anything found.

COMMITS:
  Two commits, in order:
    1. feat(naga): enforce sample-frequency execution for MSAA subpass inputs
    2. docs(tiled): mark gap #8 resolved

DELIVERABLE:
  - All tests green.
  - SPIR-V output for the new MSAA snapshot includes OpExecutionMode
    SampleRateShading.
  - Vulkan pipeline state sets sample_shading_enable when an MSAA subpass
    input is reached.
  - GLSL backend rejects MSAA subpass inputs with a clear error.
  - WGSL frontend rejects entry points missing @builtin(sample_index)
    when they reach MSAA subpass-input globals, with an actionable hint.
  - TILED.md updated; gap #8 moved to Resolved.
  - PR description names: validator rule, SPIR-V execution mode, Vulkan
    sample_shading plumbing, GLSL deliberate rejection.

NON-GOALS (do not touch in this PR):
  - Per-sample MSAA reads (gap #9 — permanently out of scope).
  - DX12 / HLSL subpass support (gaps #7 / #11).
  - Public wgpu::TransientAttachment handle (gap #6).
  - Layered / multiview subpass inputs (gap #10).
  - GLSL MSAA subpass inputs — deliberately rejected here.

If any step is ambiguous, prefer the simpler interpretation and document
the choice in the PR description rather than inventing extra surface area.
````
