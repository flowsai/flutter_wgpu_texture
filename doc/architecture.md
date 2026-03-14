# Architecture

`flutter_wgpu_texture` is split into three layers that each own a distinct
concern. They communicate through narrow, well-defined boundaries so you can
change one layer without touching the others.

## Layer overview

### Desktop / native (macOS, Windows, Linux)

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
                 │  method channel (texture registration only)
                 ▼
┌─────────────────────────────────────────┐
│           Rust renderer                 │
│                                         │
│   Renderer  ──►  Box<dyn Scene>         │
│   engine.rs      scenes/               │
│   scene.rs         ├ CubeScene         │
│   present.rs       ├ ParticlesScene    │
│   api/mod.rs       └ ShaderPlayground  │
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

### Web

On web there is no Rust layer. A pure-Dart backend renders directly to a
canvas, selecting the best available browser GPU API at runtime:

```
┌─────────────────────────────────────────┐
│              Flutter app                │
│                                         │
│   FlutterWgpuTexture (widget)           │
│   FlutterWgpuTextureController          │
└────────────────┬────────────────────────┘
                 │  delegates to web-only backend
                 ▼
┌─────────────────────────────────────────┐
│     FlutterWgpuTextureBackendWeb        │
│                                         │
│   HtmlElementView (platform view)       │
│   HTMLCanvasElement                     │
│   WebRenderer (abstract interface)      │
│     ├─ WebGpuRenderer  (WebGPU API)     │
│     └─ WebGlRenderer   (WebGL2 API)     │
└─────────────────────────────────────────┘
```

The backend tries `WebGpuRenderer` first; if it fails (adapter unavailable,
browser blocking) it falls back to `WebGlRenderer`. The widget API and
controller surface are identical across all platforms.

## Dart layer

`lib/src/flutter_wgpu_texture_widget.dart`
`lib/src/flutter_wgpu_texture_controller.dart`

The **widget** mounts a Flutter `Texture` widget and keeps the texture surface
sized to the current layout via `LayoutBuilder`. It does not own any renderer
state.

The **controller** is the coordinator between Rust and the native bridge. It
owns:

- the Rust `Renderer` handle (created via FRB FFI on `init`)
- the platform surface id
- the `Ticker` that drives the animation loop
- the command bridge: `setFloatParam`, `setBoolParam`, `setVec4Param`,
  `invokeRustCommand`
- the selected `sceneType` string forwarded to Rust on init

On each platform the controller follows this init sequence:

| Step | macOS | Windows | Linux |
|------|-------|---------|-------|
| 1 | `createRenderer` (FRB) | `createRenderer` (FRB) | `createRenderer` (FRB) |
| 2 | `createSurface` → method channel → Swift allocates Metal texture → returns `mtlTexturePtr` | `createDxgiSurface` (FRB) → DXGI handle | `ensureLinuxPresent` + `exportDmabuf` (FRB) → DMA-BUF params |
| 3 | `attachMetalTexture` (FRB) | `createSurface` → method channel → C++ registers GPU surface | `createSurface` → method channel → C++ imports DMA-BUF |

The native bridge never calls into Rust — all Rust operations go through the
FRB Dart API and the results are forwarded to native via the method channel.

The Dart→Rust boundary is deliberately generic (string keys + typed values)
so new renderer parameters do not require new Dart API.

## Rust layer

`rust/src/engine.rs` — `Renderer` struct
`rust/src/scene.rs` — `Scene` trait and `SceneRenderArgs`
`rust/src/scenes/` — built-in scene implementations
`rust/src/present.rs` — platform-specific present target
`rust/src/api/mod.rs` — FFI-exported functions consumed by flutter_rust_bridge

`Renderer` is the single owner of:

- the `wgpu` device and queue (shared, via `EngineDeviceContext`)
- the present target (platform-specific texture handle)
- the active scene as `Box<dyn Scene>`

On each tick the controller calls `requestFrame()` via FRB. `Renderer::render`
builds a `SceneRenderArgs` and passes it to `scene.render(&args, &mut encoder)`.

### Scene trait

```rust
pub trait Scene: Send {
    fn render(&mut self, args: &SceneRenderArgs, encoder: &mut wgpu::CommandEncoder)
        -> Result<(), String>;
    fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {}
    fn set_float_param(&mut self, _key: &str, _value: f32) {}
    fn set_bool_param(&mut self, _key: &str, _value: bool) {}
    fn set_vec4_param(&mut self, _key: &str, _value: [f32; 4]) {}
    fn invoke_command(&mut self, _command: &str, _payload: &str) -> Result<(), String> { Ok(()) }
}
```

The three built-in scenes (`CubeScene`, `ParticlesScene`, `ShaderPlaygroundScene`)
live in `rust/src/scenes/` and serve as reference implementations. Instantiation
is done by `scenes::scene_for_type(name, device, width, height)`.

## Native platform bridge

`macos/`  `windows/`  `linux/`

The bridge has one job: register a GPU texture with the Flutter engine and
signal when a new frame is ready. It does not call Rust and does not own
scene logic.

Each platform uses a different zero-copy interop path to avoid CPU readback:

| Platform | wgpu backend | Flutter texture type | Interop |
|----------|-------------|----------------------|---------|
| macOS    | Metal       | `FlutterTexture`     | `CVMetalTexture` backed by the same `CVPixelBuffer` that wgpu renders into |
| Windows  | DX12        | GPU surface texture  | DXGI shared handle created by Rust, consumed by Flutter |
| Linux    | Vulkan      | `FlTextureGL`        | Vulkan image → DMA-BUF → EGL import → GL texture |

## Build system

Rust is compiled by the build hook (`hook/build.dart`) using
`native_toolchain_rust`. The hook runs automatically as part of
`flutter build` / `flutter run` and produces a dynamic library that
`flutter_rust_bridge` loads at runtime via `DynamicLibrary.open()`.

The Swift / C++ method-channel plugin is compiled separately by Flutter's
normal plugin machinery (podspec on macOS, CMakeLists on Windows / Linux).

## Data flow per frame

1. Dart `Ticker` fires → controller calls `requestFrame()` over FRB
2. Rust builds a wgpu command buffer, renders the scene into the present target
3. Rust returns `true` (frame rendered) to Dart
4. Dart calls `markFrameAvailable` over the method channel
5. Native bridge calls `markTextureFrameAvailable` on the Flutter engine
6. Flutter composites the `Texture` widget on the next raster frame
