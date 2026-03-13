# flutter_wgpu_texture

Desktop Flutter texture plugin powered by Rust and `wgpu`.

Supported targets:

- macOS
- Windows
- Linux

Disabled for now:

- Web
- Android
- iOS

The package provides:

- `FlutterWgpuTexture`: Flutter widget that displays a native GPU texture
- `FlutterWgpuTextureController`: controller that owns the native renderer and
  forwards runtime commands into Rust
- a desktop-native present bridge per platform

The rotating cube demo lives in [examples/spinning_cube](./examples/spinning_cube).

## Quick start

```dart
final controller = FlutterWgpuTextureController();

FlutterWgpuTexture(
  controller: controller,
)
```

Control the native Rust renderer at runtime:

```dart
await controller.stopAnimation();
await controller.setCubeColor(const Color(0xFFFFD400));
await controller.setBackgroundColor(const Color(0xFF1B5CFF));
await controller.setFloatParam('rotation_speed', 0.8);
await controller.invokeRustCommand('reset_scene');
```

## Architecture

- Dart:
  - `lib/src/flutter_wgpu_texture_widget.dart`
  - `lib/src/flutter_wgpu_texture_controller.dart`
- Rust:
  - `rust/src/engine.rs`
  - `rust/src/present.rs`
  - `rust/src/api/mod.rs`
- Native platform bridges:
  - `macos/`
  - `windows/`
  - `linux/`

More detail:

- [docs/architecture.md](./docs/architecture.md)
- [docs/extending_rust_logic.md](./docs/extending_rust_logic.md)
- [docs/platforms.md](./docs/platforms.md)

## Extending the Rust logic

The controller already exposes generic command forwarding:

- `setBoolParam`
- `setFloatParam`
- `setVec4Param`
- `invokeRustCommand`

If you need more Rust-side behavior:

1. add the state/command in `rust/src/engine.rs`
2. expose it through `rust/src/api/mod.rs` if needed
3. regenerate FRB bindings
4. optionally add a typed controller helper

The full workflow is documented in
[docs/extending_rust_logic.md](./docs/extending_rust_logic.md).

## Running the cube demo

```bash
cd examples/spinning_cube
flutter pub get
flutter run -d linux
```
