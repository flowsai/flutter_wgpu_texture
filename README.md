# flutter_wgpu_texture

![screenshot-wgpu-texture](https://i.imgur.com/LvecF8R.png)

Desktop Flutter texture plugin powered by Rust and [`wgpu`](https://wgpu.rs/).
Renders GPU content into a native Flutter texture on macOS, Windows, and Linux.

| Platform | Supported |
|----------|-----------|
| macOS    | ✓         |
| Windows  | ✓         |
| Linux    | ✓         |
| Android  | ✗         |
| iOS      | ✗         |
| Web      | ✗         |

## Installation

```yaml
dependencies:
  flutter_wgpu_texture:
    path: ../  # or your pub.dev / git reference
```

**Requirements:**

- Flutter desktop toolchain
- Rust toolchain (`rustup`)
- macOS: Xcode command line tools
- Windows: Visual Studio C++ build tools
- Linux: Vulkan or EGL drivers

## Usage

```dart
final controller = FlutterWgpuTextureController(sceneType: 'cube');

@override
Widget build(BuildContext context) {
  return FlutterWgpuTexture(controller: controller);
}
```

Control the renderer at runtime:

```dart
await controller.setCubeColor(const Color(0xFFFFD400));
await controller.setBackgroundColor(const Color(0xFF1B5CFF));
await controller.setFloatParam('rotation_speed', 0.8);
await controller.stopAnimation();
await controller.invokeRustCommand('reset_scene');
```

See the [API reference](https://pub.dev/documentation/flutter_wgpu_texture) for the full controller API.

## Examples

| Example | Description |
|---------|-------------|
| [spinning_cube](./example/spinning_cube) | Rotating 3D cube with color controls |
| [particles](./example/particles) | Particle scene with size and motion controls |
| [shader_playground](./example/shader_playground) | Live WGSL shader editor with uniform sliders |
| [custom_scene](./example/custom_scene) | Animated gradient — custom scene outside the plugin |

```bash
cd example/spinning_cube
flutter pub get
flutter run -d macos  # or windows / linux
```

## Custom scenes

You can implement your own GPU scene in Rust **without forking the plugin**.
The plugin's renderer is split into two crates:

- **`flutter_wgpu_texture_core`** — the `Scene` trait, `SceneRenderArgs`, and a
  global scene registry (published to crates.io).
- **`flutter_wgpu_texture`** — the FRB API, C exports, and built-in scenes.

Add a Rust workspace to your app that links both the engine and your scene crate
into a single replacement dylib.  Your scene self-registers via `#[ctor::ctor]`
at load time, and Dart selects it with `sceneType: 'my_scene'`.

See **[doc/custom_scene.md](./doc/custom_scene.md)** for the step-by-step
guide and **[example/custom_scene/](./example/custom_scene)** for a complete
reference implementation (animated gradient with runtime colour controls).

## Architecture

Built on `flutter_rust_bridge` and `native_toolchain_rust`. The Dart controller
coordinates a Rust/wgpu renderer via FRB FFI; the renderer writes directly into
a shared Metal / D3D12 / DMA-BUF surface that Flutter composites as a texture.

- [doc/architecture.md](./doc/architecture.md)
- [doc/extending_rust_logic.md](./doc/extending_rust_logic.md)
- [doc/custom_scene.md](./doc/custom_scene.md)
- [doc/platforms.md](./doc/platforms.md)
