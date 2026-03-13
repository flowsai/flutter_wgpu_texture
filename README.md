# flutter_wgpu_texture

Desktop Flutter texture plugin powered by Rust and `wgpu`.

Supported platforms:

- macOS
- Windows
- Linux

Not supported:

- Android
- iOS
- Web

The root repo may still contain standard Flutter plugin scaffolding for
unsupported platforms, but the current plugin implementation is desktop-only.

## What the package provides

- `FlutterWgpuTexture`: Flutter widget that displays a native GPU texture
- `FlutterWgpuTextureController`: controller that owns the renderer lifecycle,
  surface sizing, animation ticker, and command bridge into Rust
- desktop-native presentation bridges for macOS, Windows, and Linux

## Examples

- [examples/spinning_cube](./examples/spinning_cube): rotating 3D cube demo
- [examples/particles](./examples/particles): particle scene with size and
  motion controls
- [examples/shader_playground](./examples/shader_playground): fullscreen WGSL
  shader demo with live uniform controls

## Requirements

- Flutter desktop toolchain for your target platform
- Rust toolchain
- Platform-native build prerequisites:
  - macOS: Xcode command line tools / Xcode
  - Windows: Visual Studio C++ build tools
  - Linux: desktop Flutter toolchain plus Vulkan/EGL-related dependencies used
    by the plugin path

This package uses `flutter_rust_bridge` and `cargokit` to build and load the
Rust side.

## Quick start

```dart
final controller = FlutterWgpuTextureController();

FlutterWgpuTexture(
  controller: controller,
)
```

Control the Rust renderer at runtime:

```dart
await controller.stopAnimation();
await controller.setCubeColor(const Color(0xFFFFD400));
await controller.setBackgroundColor(const Color(0xFF1B5CFF));
await controller.setFloatParam('rotation_speed', 0.8);
await controller.invokeRustCommand('reset_scene');
```

To select a built-in scene:

```dart
final controller = FlutterWgpuTextureController(
  sceneType: 'particles',
);
```

Known built-in scene types:

- `cube`
- `particles`
- `shader_playground`

Known built-in runtime params:

- `cube`
  - `rotation_speed`
  - `angle`
  - `cube_color`
  - `background_color`
- `particles`
  - `point_size`
  - `motion_scale`
  - `time`
  - `color1`
  - `color2`
  - `background_color`
- `shader_playground`
  - `speed`
  - `noise_scale`
  - `distortion`
  - `time`
  - `primary_color`
  - `secondary_color`
  - `pointer`
  - `background_color`

## Running the examples

Spinning cube:

```bash
cd examples/spinning_cube
flutter pub get
flutter run -d macos
```

Particles:

```bash
cd examples/particles
flutter pub get
flutter run -d macos
```

Shader playground:

```bash
cd examples/shader_playground
flutter pub get
flutter run -d macos
```

Replace `macos` with `windows` or `linux` as needed.

## Architecture

- Dart:
  - `lib/src/flutter_wgpu_texture_widget.dart`
  - `lib/src/flutter_wgpu_texture_controller.dart`
- Rust:
  - `rust/src/engine.rs`
  - `rust/src/present.rs`
  - `rust/src/api/mod.rs`
- Native desktop bridges:
  - `macos/`
  - `windows/`
  - `linux/`

More detail:

- [docs/architecture.md](./docs/architecture.md)
- [docs/extending_rust_logic.md](./docs/extending_rust_logic.md)
- [docs/platforms.md](./docs/platforms.md)
