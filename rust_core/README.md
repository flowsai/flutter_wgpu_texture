# flutter_wgpu_texture_core

Core Rust abstractions for extending `flutter_wgpu_texture`.

This crate exposes the shared scene interfaces and registry used by the main
runtime crate:

- `Scene`
- `SceneRenderArgs`
- global scene registration helpers

It is intended for advanced users who want to register custom `wgpu` scenes in
Rust without forking the Flutter plugin.

Related crates and docs:

- Runtime crate: `flutter_wgpu_texture`
- Repository: <https://github.com/flowsai/flutter_wgpu_texture>
- Custom scene guide: <https://github.com/flowsai/flutter_wgpu_texture/blob/main/doc/custom_scene.md>
