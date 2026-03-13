# Architecture

`flutter_wgpu_texture` is split into three layers that each own a distinct
concern. They communicate through narrow, well-defined boundaries so you can
change one layer without touching the others.

## Layer overview

```
┌─────────────────────────────────────────┐
│              Flutter app                │
│                                         │
│   FlutterWgpuTexture (widget)           │
│   FlutterWgpuTextureController          │
│         lib/src/                        │
└────────────────┬────────────────────────┘
                 │  FFI (flutter_rust_bridge)
                 │  params / commands (string keys + typed values)
                 ▼
┌─────────────────────────────────────────┐
│           Rust renderer                 │
│                                         │
│   Renderer  ──►  SceneType              │
│   engine.rs      ├ Cube                 │
│   present.rs     ├ Particles            │
│   api/mod.rs     └ ShaderPlayground     │
└────────────────┬────────────────────────┘
                 │  writes pixels into a shared GPU surface
                 ▼
┌─────────────────────────────────────────┐
│        Native platform bridge           │
│                                         │
│   macOS:   Metal  (CVMetalTexture)      │
│   Windows: DX12   (DXGI shared handle)  │
│   Linux:   Vulkan (DMA-BUF → EGL → GL) │
└─────────────────────────────────────────┘
```

## Dart layer

`lib/src/flutter_wgpu_texture_widget.dart`
`lib/src/flutter_wgpu_texture_controller.dart`

The **widget** mounts a Flutter `Texture` widget and keeps the texture surface
sized to the current layout via `LayoutBuilder`. It does not own any renderer
state.

The **controller** owns:

- the Rust `Renderer` handle (created via FFI on `init`)
- the platform surface id returned by the native bridge
- the `Ticker` that drives the animation loop
- the command bridge: `setFloatParam`, `setBoolParam`, `setVec4Param`,
  `invokeRustCommand`
- the selected `sceneType` string forwarded to Rust on init

The Dart→Rust boundary is deliberately generic (string keys + typed values)
so new renderer parameters do not require new Dart API.

## Rust layer

`rust/src/engine.rs` — `Renderer` struct and `SceneType` enum
`rust/src/present.rs` — platform-specific present target
`rust/src/api/mod.rs` — FFI-exported functions consumed by flutter_rust_bridge

`Renderer` is the single owner of:

- the `wgpu` device, queue, and swapchain-equivalent surface
- the present target (platform-specific texture handle)
- all scene state

On each tick the controller calls `render()` via FFI. `Renderer::render`
dispatches to the active scene's render function, which records a command
buffer and submits it to the wgpu queue.

### Scene model

Each scene is a variant of `SceneType` that holds its own state struct.
The `Renderer` owns one active scene at a time. Shared responsibilities
(`resize`, `set_float_param`, `set_vec4_param`, `invoke_command`) are handled
at the `Renderer` level and delegated to the active scene where needed.

## Native platform bridge

`macos/`  `windows/`  `linux/`

The bridge has one job: register a GPU texture with the Flutter engine and
signal when a new frame is ready. It does not own scene logic.

Each platform uses a different zero-copy interop path to avoid CPU readback:

| Platform | wgpu backend | Flutter texture type | Interop |
|----------|-------------|----------------------|---------|
| macOS    | Metal       | `FlutterTexture`     | `CVMetalTexture` shared between wgpu and Flutter |
| Windows  | DX12        | GPU surface texture  | DXGI shared handle |
| Linux    | Vulkan      | `FlTextureGL`        | Vulkan image → DMA-BUF → EGL import → GL texture |

## Data flow per frame

1. Dart `Ticker` fires → controller calls `render()` over FFI
2. Rust records and submits a GPU command buffer for the active scene
3. Rust signals the native bridge that a frame is ready
4. Native bridge calls `markTextureFrameAvailable` on the Flutter engine
5. Flutter composites the `Texture` widget on the next raster frame
