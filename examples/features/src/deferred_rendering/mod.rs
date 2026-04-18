//! Placeholder deferred-rendering entry point.
//!
//! This currently delegates to the headless subpass render-graph sample while
//! backend-specific multi-subpass input-attachment paths stabilize.
pub fn main() {
    log::warn!(
        "deferred_rendering example is currently a placeholder; running subpass_render_graph"
    );
    crate::subpass_render_graph::main();
}
