# Architecture

`flutter_wgpu_texture` is split into three layers inside one package.

## Dart widget/controller

- `lib/src/flutter_wgpu_texture_widget.dart`
- `lib/src/flutter_wgpu_texture_controller.dart`

The widget mounts a Flutter `Texture` and keeps the surface sized to the
current layout.

The controller owns:

- the Rust renderer handle
- the platform surface id
- the animation ticker
- the command bridge into Rust
- the selected built-in scene type

## Rust renderer

- `rust/src/engine.rs`
- `rust/src/present.rs`
- `rust/src/api/mod.rs`

Rust owns the `wgpu` device, queue, pipelines, scene state, and present target.

The engine currently supports multiple scene types:

- `Cube`
- `Particles`
- `ShaderPlayground`

Each scene keeps its own renderer state and render path inside `engine.rs`.
Shared responsibilities handled by the top-level `Renderer` include:

- surface sizing
- present target lifetime
- frame timing / animation state
- generic runtime parameter routing
- dispatch to the active scene render function

## Native platform bridge

Each desktop platform only does texture registration and frame availability:

- macOS: Metal texture attached from `CVMetalTexture`
- Windows: DXGI shared handle texture
- Linux: Vulkan image exported as DMA-BUF and imported into `FlTextureGL`

The desktop bridge does not own scene logic. Scene behavior stays in Rust.
