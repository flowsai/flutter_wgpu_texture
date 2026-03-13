# Extending Rust Logic

The package is designed so users can add their own Rust-side rendering logic
without rewriting the Flutter texture plumbing.

## Use the generic controller methods first

The controller already exposes:

- `setBoolParam(key, value)`
- `setFloatParam(key, value)`
- `setVec4Param(key, value)`
- `invokeRustCommand(command, payload: '{}')`

If the Rust renderer already understands a key or command, you do not need any
new Dart API.

## Add a new parameter

1. Add a field in `rust/src/engine.rs`.
2. Update:
   - `Renderer::set_bool_param`
   - `Renderer::set_float_param`
   - `Renderer::set_vec4_param`
3. Use the field inside `Renderer::render`.

Then call it from Dart through the generic controller methods.

## Add a new command

1. Add a branch in `Renderer::invoke_command`.
2. Parse the payload string if you need structured data.
3. Call it from Dart:

```dart
await controller.invokeRustCommand('your_command', payload: '{"value":1}');
```

## Add a typed Dart helper

Once a command becomes stable, add a convenience method on
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
