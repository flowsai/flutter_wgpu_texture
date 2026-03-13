# Architecture

`flutter_wgpu_texture` is split into three layers inside one package.

## Dart widget/controller

- `lib/src/flutter_wgpu_texture_widget.dart`
- `lib/src/flutter_wgpu_texture_controller.dart`

The widget only mounts a Flutter `Texture` and keeps the surface sized to the
current layout.

The controller owns:

- the Rust renderer handle
- the platform surface id
- the animation ticker
- the command bridge into Rust

## Rust renderer

- `rust/src/engine.rs`
- `rust/src/present.rs`
- `rust/src/api/mod.rs`

Rust owns the `wgpu` device, queue, pipelines, scene state, and present target.
The built-in demo scene is a rotating cube, but the architecture is command
driven so the scene logic can be extended without changing the platform bridges.

## Native platform bridge

Each desktop platform only does texture registration and frame availability:

- macOS: Metal texture attached from `CVMetalTexture`
- Windows: DXGI shared handle texture
- Linux: Vulkan image exported as DMA-BUF and imported into `FlTextureGL`
