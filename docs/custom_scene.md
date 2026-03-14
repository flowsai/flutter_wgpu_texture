# Custom Scene — Workspace Pattern

This guide explains how to implement a custom GPU scene in Rust and use it with
`flutter_wgpu_texture` **without forking the plugin**.

---

## How it works

The plugin is split into two crates:

| Crate | Role |
|-------|------|
| `flutter_wgpu_texture_core` | `Scene` trait, `SceneRenderArgs`, global scene registry |
| `flutter_wgpu_texture` (plugin) | FRB API, C exports, built-in scenes |

Your app provides a **combined Rust workspace** that links the plugin engine and
your scene into a single shared library.  That library is bundled alongside the
plugin's own, and because it is discovered first by the loader, it replaces the
plugin's dylib for the lifetime of the process.

### Loading order (macOS)

The plugin's Dart loader (`rust_dylib.dart`) checks frameworks in this order:

1. `custom_scene_bridge.framework/custom_scene_bridge` — your combined workspace
2. `flutter_wgpu_texture.framework/flutter_wgpu_texture` — the plugin's own dylib

> **Convention:** your bridge crate **must** be named `custom_scene_bridge` so
> the loader finds it automatically.  This is the only fixed naming requirement.

---

## Step-by-step

### 1. Project layout

```
my_app/
├── lib/
│   └── main.dart
├── hook/
│   └── build.dart          ← builds the combined Rust workspace
├── pubspec.yaml
└── rust/
    ├── Cargo.toml           ← workspace root + combined cdylib
    ├── build.rs             ← linker flags for symbol export
    ├── rust-toolchain.toml  ← pin Rust version
    ├── src/
    │   └── lib.rs           ← re-exports engine + links scene
    └── my_scene/
        ├── Cargo.toml
        └── src/
            └── lib.rs       ← your Scene implementation
```

### 2. `rust/Cargo.toml`

```toml
[workspace]
members = ["my_scene"]

[package]
# MUST be "custom_scene_bridge" — the loader looks for this exact framework name.
name = "custom_scene_bridge"
version = "0.1.0"
edition = "2021"

[lib]
# native_toolchain_rust requires both crate types.
crate-type = ["staticlib", "cdylib"]

[dependencies]
# Alias the engine dep to avoid a naming conflict with this package.
flutter_wgpu_texture_engine = {
  git = "https://github.com/flowsai/flutter_wgpu_texture",
  package = "flutter_wgpu_texture",
}
# (use `path = "path/to/plugin/rust"` for local development)

# Your scene crate — its #[ctor] fn registers the scene at dylib load time.
my_scene = { path = "my_scene" }
```

### 3. `rust/src/lib.rs`

```rust
// Re-export every public symbol from the plugin engine into this cdylib.
// This prevents Rust's dead-code eliminator from stripping the engine's
// #[no_mangle] C exports and FRB boilerplate before the link step.
//
// The my_scene dep is listed in Cargo.toml; its #[ctor::ctor] fn runs at
// dylib load time and registers the scene — no explicit reference needed here.
pub use flutter_wgpu_texture_engine::*;
```

> **Why `pub use *` and not `extern crate`?**  In Rust 2021 `extern crate` is
> not required for linking.  More importantly, re-exporting the engine symbols
> at the crate root prevents dead-code elimination: Rust only includes dep
> symbols in a cdylib when they are reachable from the crate root.

### 4. `rust/build.rs`

Forces the linker to include all object files from static archives — required
for the engine symbols to survive into the final dylib on macOS/Linux:

```rust
fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    match target_os.as_str() {
        "macos" | "ios" => {
            println!("cargo:rustc-link-arg-cdylib=-Wl,-all_load");
        }
        "linux" | "android" => {
            println!("cargo:rustc-link-arg-cdylib=-Wl,--whole-archive");
            println!("cargo:rustc-link-arg-cdylib=-Wl,--no-whole-archive");
        }
        _ => {}
    }
}
```

> Use `CARGO_CFG_TARGET_OS` (target platform) not `#[cfg(target_os = ...)]`
> (host platform) — they differ when cross-compiling.

### 5. `rust/rust-toolchain.toml`

Pin to the same Rust version the plugin was built with (`native_toolchain_rust`
requires this file):

```toml
[toolchain]
channel = "1.92.0"
targets = [
  "aarch64-apple-darwin",
  "x86_64-apple-darwin",
  "aarch64-pc-windows-msvc",
  "x86_64-pc-windows-msvc",
  "x86_64-unknown-linux-gnu",
]
```

### 6. `rust/my_scene/Cargo.toml`

```toml
[package]
name = "my_scene"
version = "0.1.0"
edition = "2021"

[dependencies]
flutter_wgpu_texture_core = "0.1"   # or path = "../../../../rust_core" locally
wgpu = "0.19"
ctor = "0.2"
```

### 7. `rust/my_scene/src/lib.rs`

```rust
use flutter_wgpu_texture_core::{register_scene, Scene, SceneRenderArgs};

pub struct MyScene { /* your GPU resources */ }

impl MyScene {
    pub fn new(device: &wgpu::Device) -> Result<Self, String> {
        // create pipelines, buffers, textures, …
        todo!()
    }
}

impl Scene for MyScene {
    fn render(
        &mut self,
        args: &SceneRenderArgs,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<(), String> {
        // record render commands into args.view
        todo!()
    }
}

/// Runs when the dylib is loaded — before any Dart call arrives.
#[ctor::ctor]
fn _register() {
    register_scene("my_scene", |device, _w, _h| {
        MyScene::new(device)
            .map(|s| Box::new(s) as Box<dyn flutter_wgpu_texture_core::Scene>)
    });
}
```

### 8. `hook/build.dart`

```dart
import 'package:hooks/hooks.dart';
import 'package:native_toolchain_rust/native_toolchain_rust.dart';

void main(List<String> args) async {
  await build(args, (input, output) async {
    await RustBuilder(
      assetName: 'flutter_wgpu_texture',
      cratePath: 'rust',
    ).run(input: input, output: output);
  });
}
```

### 9. `pubspec.yaml`

```yaml
dependencies:
  flutter_wgpu_texture: ^0.1.0
  hooks: ^1.0.2
  native_toolchain_rust: ^1.0.3
```

### 10. `lib/main.dart`

```dart
final controller = FlutterWgpuTextureController(sceneType: 'my_scene');

Widget build(BuildContext context) =>
    FlutterWgpuTexture(controller: controller);
```

---

## Scene trait reference

```rust
pub trait Scene: Send {
    /// Record render commands for one frame.
    fn render(
        &mut self,
        args: &SceneRenderArgs,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<(), String>;

    /// Called when the texture is resized.
    fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {}

    // Optional parameter bridges — call from Dart via the controller:
    fn set_float_param(&mut self, _key: &str, _value: f32) {}
    fn set_bool_param(&mut self, _key: &str, _value: bool) {}
    fn set_vec4_param(&mut self, _key: &str, _value: [f32; 4]) {}
    fn invoke_command(&mut self, _cmd: &str, _payload: &str) -> Result<(), String> { Ok(()) }
}
```

`SceneRenderArgs` fields:

| Field | Type | Description |
|-------|------|-------------|
| `device` | `&wgpu::Device` | wgpu device |
| `queue` | `&wgpu::Queue` | submission queue |
| `view` | `&wgpu::TextureView` | render target |
| `width` / `height` | `u32` | current texture size in pixels |
| `dt` | `f32` | seconds since last frame |
| `animation_running` | `bool` | `false` when paused |
| `clear_color` | `wgpu::Color` | background color from the controller |

---

## How the registry works

`register_scene` stores a factory closure in a global
`OnceLock<Mutex<HashMap<String, SceneFactory>>>` inside
`flutter_wgpu_texture_core`.

The `#[ctor::ctor]` attribute generates a platform-specific dylib initializer
(`__mod_init_func` on macOS, `DT_INIT` on Linux, `DllMain` on Windows) that
runs **before the first exported function is called**.  Because your scene crate
and the plugin engine are linked into the **same dylib**, the registry is shared
and the factory is available when Dart calls `create_renderer`.

---

## Reference implementation

[`examples/custom_scene/`](../examples/custom_scene) is a complete working
example: an animated gradient with color and pause controls, built exactly as
described above.

---

## Using built-in parameter bridges

If you only need to tweak behaviour at runtime without writing Rust, use the
parameter bridge on any scene — no custom workspace needed:

```dart
await controller.setFloatParam('rotation_speed', 0.4);
await controller.setVec4Param('cube_color', [1.0, 0.5, 0.0, 1.0]);
await controller.invokeRustCommand('reset_scene');
```

See [`extending_rust_logic.md`](./extending_rust_logic.md) for details.
