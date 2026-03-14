# Extending Rust Logic

The package is designed so you can add Rust-side rendering logic without
touching the Flutter texture plumbing or the native platform bridge.

## Start with the generic controller API

`FlutterWgpuTextureController` already exposes a generic command bridge:

```dart
await controller.setFloatParam('key', 1.0);
await controller.setBoolParam('key', true);
await controller.setVec4Param('key', [r, g, b, a]);
await controller.invokeRustCommand('command', payload: '{"value":1}');
```

If the Rust scene already handles the key or command, no new Dart code is
needed.

## Add a parameter to an existing scene

1. Add the field to the scene struct in the relevant file under
   `rust/src/scenes/` (e.g. `rust/src/scenes/cube.rs`).
2. Route the incoming key in the scene's `Scene` trait implementation:
   - `fn set_float_param(&mut self, key: &str, value: f32)`
   - `fn set_bool_param(&mut self, key: &str, value: bool)`
   - `fn set_vec4_param(&mut self, key: &str, value: [f32; 4])`
3. Use the field in the scene's `render` function.

Call it from Dart immediately — no binding regeneration required:

```dart
await controller.setFloatParam('your_new_param', 0.5);
```

## Add a new command

1. Add a branch in the scene's `invoke_command` implementation in
   `rust/src/scenes/<your_scene>.rs`.
2. Update the scene state.
3. Call it from Dart:

```dart
await controller.invokeRustCommand('your_command', payload: '{"value":1}');
```

## Add a new built-in scene

1. Create `rust/src/scenes/my_scene.rs` implementing the `Scene` trait:

```rust
use crate::scene::{Scene, SceneRenderArgs};

pub struct MyScene { /* state */ }

impl MyScene {
    pub fn new(device: &wgpu::Device) -> Result<Self, String> { /* ... */ }
}

impl Scene for MyScene {
    fn render(&mut self, args: &SceneRenderArgs, encoder: &mut wgpu::CommandEncoder)
        -> Result<(), String> { /* ... */ }

    fn set_float_param(&mut self, key: &str, value: f32) { /* ... */ }
}
```

2. Expose it from `rust/src/scenes/mod.rs`:

```rust
mod my_scene;
pub use my_scene::MyScene;
```

3. Add a branch in `scene_for_type`:

```rust
"my_scene" => Ok(Box::new(MyScene::new(device)?)),
```

4. Use it from Dart:

```dart
final controller = FlutterWgpuTextureController(sceneType: 'my_scene');
```

## Add a typed Dart convenience method

Once a param stabilises, wrap it on the controller for better autocompletion
and type safety:

```dart
Future<void> setExposure(double value) => setFloatParam('exposure', value);
```

## Regenerate FFI bindings

Only needed when the public surface of `rust/src/api/mod.rs` changes
(new exported functions, changed signatures). Run from the package root:

```bash
flutter_rust_bridge_codegen generate
```

Parameter keys and command strings do not require regeneration.
