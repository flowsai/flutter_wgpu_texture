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
| [spinning_cube](./examples/spinning_cube) | Rotating 3D cube with color controls |
| [particles](./examples/particles) | Particle scene with size and motion controls |
| [shader_playground](./examples/shader_playground) | Live WGSL shader editor with uniform sliders |

```bash
cd examples/spinning_cube
flutter pub get
flutter run -d macos  # or windows / linux
```

## Architecture

Built on `flutter_rust_bridge` and `native_toolchain_rust`. The Dart controller
coordinates a Rust/wgpu renderer via FRB FFI; the renderer writes directly into
a shared Metal / D3D12 / DMA-BUF surface that Flutter composites as a texture.

- [docs/architecture.md](./docs/architecture.md)
- [docs/extending_rust_logic.md](./docs/extending_rust_logic.md)
- [docs/platforms.md](./docs/platforms.md)
