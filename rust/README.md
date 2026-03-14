# flutter_wgpu_texture

Rust runtime crate for the `flutter_wgpu_texture` Flutter plugin.

This crate provides the native renderer, built-in scenes, and the
`flutter_rust_bridge` bindings used by the Dart package. It targets desktop
platforms with `wgpu` backends:

- macOS: Metal
- Windows: D3D12
- Linux: Vulkan

Most users should depend on the Flutter package instead:

- Pub package: `flutter_wgpu_texture`
- Repository: <https://github.com/flowsai/flutter_wgpu_texture>

If you want to implement custom scenes in Rust, look at the companion crate:

- `flutter_wgpu_texture_core`

For end-to-end usage and examples, see the repository README and examples.
