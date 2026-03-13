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

If the Rust renderer already handles the key or command, no new Dart code is
needed.

## Add a parameter to an existing scene

1. Add the field to the scene state struct in `rust/src/engine.rs`.
2. Route the incoming value in the appropriate `Renderer` setter:
   - `Renderer::set_float_param`
   - `Renderer::set_bool_param`
   - `Renderer::set_vec4_param`
3. Use the field in the scene's render function.

Call it from Dart immediately — no binding regeneration required:

```dart
await controller.setFloatParam('your_new_param', 0.5);
```

## Add a new command

1. Add a branch in `Renderer::invoke_command` in `rust/src/engine.rs`.
2. Update the appropriate scene state.
3. Call it from Dart:

```dart
await controller.invokeRustCommand('your_command', payload: '{"value":1}');
```

## Add a new built-in scene

1. Add a variant to `SceneType` in `rust/src/engine.rs`.
2. Add the scene state struct and a constructor.
3. Instantiate it in `Renderer::new` based on the incoming scene type string.
4. Add the scene render function and dispatch to it from `Renderer::render`.
5. Route scene-specific params in `set_float_param` / `set_vec4_param` etc.
6. Map the public `sceneType` string to the new enum variant in
   `rust/src/api/mod.rs`.

Then use it from Dart:

```dart
final controller = FlutterWgpuTextureController(sceneType: 'your_scene');
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
