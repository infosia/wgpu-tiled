# wgpu-tiled

> **A mobile-optimized fork of [wgpu](https://github.com/gfx-rs/wgpu) with tile-based deferred rendering (TBDR) support.**

<img align="right" width="20%" src="logo.png">

## What is wgpu-tiled?

wgpu-tiled extends wgpu with **multi-subpass render passes**, **transient (tile-memory-only) attachments**, and **input attachments** -- the critical GPU features that mobile TBDR architectures (Apple GPU, Qualcomm Adreno, ARM Mali) need to keep intermediate rendering data in fast on-chip tile memory instead of round-tripping through slow main memory (DRAM).

Standard wgpu follows the WebGPU spec, which only supports flat single-pass rendering. This forces mobile GPUs to write intermediate data (G-buffers, depth, normals) to DRAM between passes, then read it back -- wasting bandwidth and battery. **wgpu-tiled eliminates this bottleneck.**

### Key Features

| Feature | What it does | Mobile benefit |
|---------|-------------|----------------|
| **Transient Attachments** | Tile-memory-only textures with no DRAM backing (`MTLStorageModeMemoryless`, `VK_IMAGE_USAGE_TRANSIENT_ATTACHMENT_BIT`) | Zero main memory allocation for intermediate buffers |
| **Multi-Subpass Render Passes** | Multiple rendering phases within a single hardware pass | G-buffer data stays in tile memory across subpasses |
| **Input Attachments** | Read previous subpass output at the current fragment position | Direct tile memory read, no texture sampling overhead |
| **Dynamic Subpass Culling** | Per-frame subpass activation/deactivation without pipeline recompilation | Skip debug/optional passes with zero GPU cost |
| **RenderGraphBuilder** | Declarative API that auto-infers load/store ops and generates subpass dependencies | Correct TBDR optimization with minimal boilerplate |
| **SubpassData in naga** | `ImageDimension::SubpassData` IR + `InputAttachmentIndex` SPIR-V decoration + GLSL `use_framebuffer_fetch` option | Shader compiler support for tile memory reads |

### Supported Backends

| Backend | Transient Attachments | Multi-Subpass | Tile Dispatch |
|---------|:--------------------:|:-------------:|:-------------:|
| **Metal** | `MTLStorageModeMemoryless` | Single encoder, `[[color(N)]]` tile shading | Deferred |
| **Vulkan** | `LAZILY_ALLOCATED` memory | Native `vkCmdNextSubpass` with `BY_REGION_BIT` | N/A |
| **GLES** | Regular texture (fallback) | `EXT_shader_framebuffer_fetch` or multi-pass | N/A |
| **DX12** | Stub (regular texture) | Stub (separate passes) | N/A |

### Architecture

```
wgpu (public API)          -- RenderPass::next_subpass(), RenderGraphBuilder
  |
wgpu-core (validation)     -- TransientAttachment resources, subpass validation errors
  |
wgpu-hal (HAL traits)      -- Device::create_transient_attachment(), CommandEncoder::next_subpass()
  |
  +-- Metal backend         -- Memoryless textures, single-encoder subpass state machine
  +-- Vulkan backend        -- LAZILY_ALLOCATED, N-subpass VkRenderPass, SubpassDependency
  +-- GLES backend          -- Renderbuffer fallback, EXT_shader_framebuffer_fetch detection
  +-- DX12 backend          -- Stub implementations
  |
naga (shader compiler)     -- SubpassData IR, InputAttachmentIndex, use_framebuffer_fetch
```

### WGSL Shader Extension: `@input_attachment_index`

wgpu-tiled extends WGSL with `@input_attachment_index(N)` for reading previous subpass output directly from tile memory:

```wgsl
// Lighting pass shader -- reads G-Buffer via input attachments
@group(0) @binding(0) @input_attachment_index(0) var t_albedo: texture_2d<f32>;
@group(0) @binding(1) @input_attachment_index(1) var t_normal: texture_2d<f32>;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let albedo = textureLoad(t_albedo, vec2<i32>(0, 0), 0).rgb;
    let normal = textureLoad(t_normal, vec2<i32>(0, 0), 0).rgb;
    // ... lighting calculation ...
}
```

The `@input_attachment_index(N)` attribute marks a `texture_2d` variable as a subpass input attachment. The coordinates passed to `textureLoad` are ignored -- the backend reads from the current fragment position automatically. naga compiles this to:
- **SPIR-V**: `OpTypeImage` with `Dim=SubpassData`, `InputAttachmentIndex` decoration
- **MSL**: `[[color(N)]]` fragment function parameter (tile memory read)
- **GLSL**: `inout` color attachment via `EXT_shader_framebuffer_fetch`, or `texelFetch(sampler, ivec2(gl_FragCoord.xy), 0)` fallback

### Quick Example

```rust
use wgpu::{RenderGraphBuilder, TextureFormat, SubpassIndex};

// Declare a deferred rendering graph
let mut builder = RenderGraphBuilder::new();
let albedo = builder.add_transient_color("albedo", TextureFormat::Rgba8Unorm);
let output = builder.add_persistent_color("output", TextureFormat::Bgra8Unorm);

builder.add_subpass("gbuffer").writes_color(albedo);
builder.add_subpass("composite").reads(albedo).writes_color(output);

let graph = builder.build()?;

// Dynamic culling -- skip optional passes without pipeline recompilation
let mask = graph.resolve_active(&[SubpassIndex(0), SubpassIndex(1)])?;
```

See `examples/features/src/deferred_rendering/` for a complete working example with G-Buffer + lighting subpasses running on Vulkan.

### Performance

Benchmarked with a 3-subpass deferred rendering pipeline (G-Buffer → Lighting → Composite), 288K triangles, 225 draw calls per frame. Comparison is wgpu-tiled subpass mode vs upstream wgpu multi-pass (3 separate render passes).

#### Metal — Apple A15 GPU (iOS, 960×1440)

| Metric | wgpu-tiled (subpass) | Upstream wgpu (multi-pass) | Improvement |
|--------|---------------------:|---------------------------:|------------:|
| GPU render time (avg) | **4.16 ms** | 4.46 ms | **7% faster** |
| DRAM bandwidth/frame | **5.3 MB** | 63.3 MB | **12x less** |

#### Vulkan — ARM Mali-G78 (Android, 1080×2209)

| Metric | wgpu-tiled (subpass) | Upstream wgpu (multi-pass) | Improvement |
|--------|---------------------:|---------------------------:|------------:|
| Avg frame time | **2.19 ms** (456 FPS) | 4.07 ms (245 FPS) | **1.86x faster** |
| DRAM bandwidth/frame | **9.1 MB** | 109.2 MB | **12x less** |

Both backends achieve a **12x DRAM bandwidth reduction** — G-buffer intermediates stay in on-chip tile memory, and only the final swapchain write hits DRAM. On Mali-G78, the bandwidth savings translate to a 1.86x frame time improvement despite slightly higher GPU shader execution time, demonstrating that DRAM avoidance dominates performance on bandwidth-constrained mobile SoCs.

### This is a Fork

wgpu-tiled is a **permanent fork** of wgpu v29.0.1. It deliberately breaks WebGPU spec compliance to expose native TBDR capabilities. There are no plans to upstream these changes or track upstream wgpu releases.

---

# wgpu (upstream)

`wgpu` is a cross-platform, safe, pure-Rust graphics API. It runs natively on Vulkan, Metal, D3D12, and OpenGL; and on top of WebGL2 and WebGPU on wasm.

The API is based on the [WebGPU standard][webgpu], but is a fully native Rust library. It serves as the core of the WebGPU integration in Firefox, Servo, and Deno.

## Getting Started

See our examples online at <https://wgpu.rs/examples/>. You can see the Rust sources at [examples](examples) and run them directly with `cargo run --bin wgpu-examples <example>`.

### Learning `wgpu`

If you are new to `wgpu` and graphics programming, we recommend starting with [Learn Wgpu].
<!-- Note, "Learn Wgpu" is using the capitalization style in their header, NOT our styling -->

Additionally, [WebGPU Fundamentals] is a tutorial for WebGPU which is very similar to our API, minus differences between Rust and Javascript.

[Learn Wgpu]: https://sotrh.github.io/learn-wgpu/
[WebGPU Fundamentals]: https://webgpufundamentals.org/

### Wiki

We have a [wiki](https://github.com/gfx-rs/wgpu/wiki) which has information on useful architecture patterns, debugging tips, and more getting started information. 

### Need Help? Want to Contribute? 

The wgpu community uses Matrix and Discord to discuss.

- [![`#wgpu:matrix.org`](https://img.shields.io/static/v1?label=wgpu-devs&message=%23wgpu&color=blueviolet&logo=matrix)](https://matrix.to/#/#wgpu:matrix.org) - discussion of wgpu's development.
- [![`#wgpu-users:matrix.org`](https://img.shields.io/static/v1?label=wgpu-users&message=%23wgpu-users&color=blueviolet&logo=matrix)](https://matrix.to/#/#wgpu-users:matrix.org) - discussion of using the library and the surrounding ecosystem.
- [![#wgpu on the Rust Gamedev Discord](https://img.shields.io/discord/676678179678715904?logo=discord&logoColor=E0E3FF&label=%23wgpu&color=5865F2)
](https://discord.gg/X3MYBNXUMJ) - Dedicated support channel on the Rust Gamedev Discord.


### Other Languages

To use wgpu in C or dozens of other languages, look at [wgpu-native](https://github.com/gfx-rs/wgpu-native). These are C bindings to wgpu and has an up-to-date list of libraries bringing support to other languages. 

[Learn WebGPU (for C++)] is a good resource for learning how to use wgpu-native from C++.

[Learn WebGPU (for C++)]: https://eliemichel.github.io/LearnWebGPU/
[webgpu]: https://gpuweb.github.io/gpuweb/

## Quick Links

| Docs                  | Examples                  | Changelog               |
|:---------------------:|:-------------------------:|:-----------------------:|
| [v29][rel-docs]       | [v29][rel-examples]       | [v29][rel-change]       |
| [`trunk`][trunk-docs] | [`trunk`][trunk-examples] | [`trunk`][trunk-change] |

Contributors are welcome! See [CONTRIBUTING.md][contrib] for more information.

[rel-docs]: https://docs.rs/wgpu/
[rel-examples]: https://github.com/gfx-rs/wgpu/tree/v29/examples#readme
[rel-change]: https://github.com/gfx-rs/wgpu/releases
[trunk-docs]: https://wgpu.rs/doc/wgpu/
[trunk-examples]: https://github.com/gfx-rs/wgpu/tree/trunk/examples#readme
[trunk-change]: https://github.com/gfx-rs/wgpu/blob/trunk/CHANGELOG.md#unreleased
[contrib]: CONTRIBUTING.md

## Supported Platforms

| API    | Windows            | Linux/Android      | macOS/iOS          | Web (wasm)         |
| ------ | ------------------ | ------------------ | ------------------ | ------------------ |
| Vulkan |         ✅         |         ✅         |         🌋         |                    |
| Metal  |                    |                    |         ✅         |                    |
| DX12   |         ✅         |                    |                    |                    |
| OpenGL |    🆗 (GL 3.3+)    |  🆗 (GL ES 3.0+)   |         📐         |    🆗 (WebGL2)     |
| WebGPU |                    |                    |                    |         ✅         |

✅ = First Class Support  
🆗 = Downlevel/Best Effort Support  
📐 = Requires the [ANGLE](https://github.com/gfx-rs/wgpu/wiki/Running-on-ANGLE) translation layer (GL ES 3.0 only)  
🌋 = Requires the [MoltenVK](https://vulkan.lunarg.com/sdk/home#mac) translation layer  
🛠️ = Unsupported, though open to contributions

## Environment Variables

Testing, examples, and `::from_env()` methods use a standardized set of environment variables to control wgpu's behavior.

- `WGPU_BACKEND` with a comma-separated list of the backends you want to use (`vulkan`, `metal`, `dx12`, or `gl`).
- `WGPU_ADAPTER_NAME` with a case-insensitive substring of the name of the adapter you want to use (ex. `1080` will match `NVIDIA GeForce 1080ti`).
- `WGPU_DX12_COMPILER` with the DX12 shader compiler you wish to use (`dxc`, `static-dxc`, or `fxc`). Note that `dxc` requires `dxcompiler.dll` (min v1.8.2502) to be in the working directory, and `static-dxc` requires the `static-dxc` crate feature to be enabled. Otherwise, it will fall back to `fxc`.

See the [documentation](https://docs.rs/wgpu/latest/wgpu/index.html?search=env) for more environment variables.

When running the CTS, use the variables `DENO_WEBGPU_ADAPTER_NAME`, `DENO_WEBGPU_BACKEND`, `DENO_WEBGPU_POWER_PREFERENCE`, and `DENO_WEBGPU_DX12_COMPILER`.

## Repo Overview

For an overview of all the components in the gfx-rs ecosystem, see [the big picture](./docs/big-picture.png).

## MSRV policy

TL;DR: If you're using `wgpu`, our MSRV is **1.87**. If you're running our tests or examples, our MSRV is **1.93**.

We will avoid bumping the MSRV of `wgpu` without good reason, and such a change is considered breaking.

<details>
<summary> Specific Details </summary>

Due to complex dependants, we have three MSRV policies:

- `wgpu`'s MSRV is **1.87**
- `wgpu-core` (and hence `wgpu-hal`, `naga`, and `wgpu-types`)'s MSRV is **1.87**.
- The rest of the workspace has an MSRV of **1.93**.

It is enforced on CI (in "/.github/workflows/ci.yml") with the `WGPU_MSRV`, `CORE_MSRV`, and `REPO_MSRV` variables, respectively.
This version can only be upgraded in breaking releases, though we release a breaking version every three months.

The following rules apply:
- The `wgpu-core` crate should never require an MSRV ahead of Firefox's MSRV for nightly builds, as
determined by the value of `MINIMUM_RUST_VERSION` in [`python/mozboot/mozboot/util.py`][moz-msrv].
- The `wgpu` crate should never require an MSRV ahead of Servo's MSRV, as determined by the value of
their rust-version declaration in [`Cargo.toml`][servo-msrv]
- The repository MSRV should never require an MSRV higher than `stable - 3`. For example, if stable is
at 1.97, the repository MSRV should be no higher than 1.94. This is to allow people who are using a decently-updated
OS-provided rust to be able to build our repository. Consider cross checking with [NixOS][nixos-msrv], though this
is not required.

[moz-msrv]: https://searchfox.org/mozilla-central/source/python/mozboot/mozboot/util.py
[servo-msrv]: https://github.com/servo/servo/blob/main/Cargo.toml#L23
[nixos-msrv]: https://search.nixos.org/packages?show=rustc

</details>

## Testing and Environment Variables

[Information about testing](./docs/testing.md), including where tests of various kinds live, and how to run the tests.

## Tracking the WebGPU and WGSL draft specifications

The `wgpu` crate is meant to be an idiomatic Rust translation of the [WebGPU API][webgpu spec].
That specification, along with its shading language, [WGSL][wgsl spec],
are both still in the "Working Draft" phase,
and while the general outlines are stable,
details change frequently.
Until the specification is stabilized, the `wgpu` crate and the version of WGSL it implements
will likely differ from what is specified,
as the implementation catches up.

Exactly which WGSL features `wgpu` supports depends on how you are using it:

- When running as native code, `wgpu` uses [Naga][naga]
  to translate WGSL code into the shading language of your platform's native GPU API.
  Naga is working on catching up to the WGSL specification,
  with [bugs][naga bugs] tracking various issues,
  but there is no concise summary of differences from the specification.

- When running in a web browser (by compilation to WebAssembly)
  without the `"webgl"` feature enabled,
  `wgpu` relies on the browser's own WebGPU implementation.
  WGSL shaders are simply passed through to the browser,
  so that determines which WGSL features you can use.

- When running in a web browser with `wgpu`'s `"webgl"` feature enabled,
  `wgpu` uses Naga to translate WGSL programs into GLSL.
  This uses the same version of Naga as if you were running `wgpu` as native code.

[webgpu spec]: https://www.w3.org/TR/webgpu/
[wgsl spec]: https://gpuweb.github.io/gpuweb/wgsl/
[naga]: https://github.com/gfx-rs/wgpu/tree/trunk/naga/
[naga bugs]: https://github.com/gfx-rs/wgpu/issues?q=is%3Aissue%20state%3Aopen%20label%3A%22naga%22

