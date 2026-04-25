# subpass_msaa

This example demonstrates MSAA subpass inputs with `subpass_input_multisampled<f32>` and `@builtin(sample_index)` on Metal and Vulkan.

It exercises the resolved path from `TILED.md` gap #8 ("MSAA subpass inputs"): a multisampled G-buffer input attachment is read in a lighting subpass at sample frequency, then a fullscreen composite pass explicitly resolves and tonemaps the multisampled lit buffer for presentation.

## To Run

```
cargo run --bin wgpu-examples subpass_msaa
```
