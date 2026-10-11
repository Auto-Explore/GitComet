# GitComet renderer patch

This is `crates/gpui_render` from
`https://github.com/Havunen/gpui-ce`, revision
`f7474ab18033dee1e58770494c3b56afb3938b24`.
The Apache license and third-party notices are retained.

The only shader change intersects shadow vertex rectangles with their content
masks before rasterization. Shadow bounds, Gaussian coverage, gradients,
antialiasing and paint coordinates remain unchanged. This avoids running the
fragment shader and blending transparent pixels outside each mask for floating
surfaces. Application window frames no longer paint a shadow.

The package manifest expands the upstream workspace dependencies to the same
pinned versions so this small crate can build independently. The application
continues to use the original GPUI dependency pins. Shader generation and
validation for WGSL, GLSL, HLSL and MSL use the unchanged upstream build pipeline.

When updating GPUI, synchronize this crate with the new pin or remove the Cargo
patch after upstream includes the fix. Run the renderer shader tests and the
native connector pixel comparisons documented in `scripts/profiling/README.md`.
