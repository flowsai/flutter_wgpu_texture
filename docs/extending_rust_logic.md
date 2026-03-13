# Extending Rust Logic

The package is designed so you can add Rust-side rendering logic without
rewriting the Flutter texture plumbing.

## Use the generic controller methods first

`FlutterWgpuTextureController` already exposes:

- `setBoolParam(key, value)`
- `setFloatParam(key, value)`
- `setVec4Param(key, value)`
- `invokeRustCommand(command, payload: '{}')`

If the Rust renderer already understands a key or command, you do not need any
new Dart API.

## Add a new parameter to an existing scene

1. Add the new field to the relevant scene renderer state in `rust/src/engine.rs`.
2. Route the value in one of:
   - `Renderer::set_bool_param`
   - `Renderer::set_float_param`
   - `Renderer::set_vec4_param`
3. Consume the field in the relevant scene render function.

Examples already in the engine:

- `rotation_speed` for `Cube`
- `point_size` / `motion_scale` for `Particles`
- `speed` / `noise_scale` / `distortion` for `ShaderPlayground`

## Add a new command

1. Add a branch in `Renderer::invoke_command`.
2. Update the appropriate scene state.
3. Call it from Dart:

```dart
await controller.invokeRustCommand('your_command', payload: '{"value":1}');
```

## Add a new built-in scene

1. Add a new variant to `SceneType` in `rust/src/engine.rs`.
2. Add scene-specific state and a constructor.
3. Instantiate it from `Renderer::new`.
4. Add a render function and dispatch to it from `Renderer::render`.
5. Route scene-specific params in the generic setter methods.
6. Map the public Dart `sceneType` string to the Rust enum in
   `rust/src/api/mod.rs`.

## Add a typed Dart helper

Once a param or command becomes stable, add a convenience method on
`FlutterWgpuTextureController`.

Example:

```dart
Future<void> setExposure(double value) {
  return setFloatParam('exposure', value);
}
```

## Regenerate FRB bindings

When `rust/src/api/mod.rs` changes, regenerate bindings from the package root:

```bash
flutter_rust_bridge_codegen generate
```
