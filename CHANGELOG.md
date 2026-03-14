## 0.1.0

* Initial release.
* Renders GPU content into a native Flutter texture using Rust and wgpu.
* Supports macOS (Metal), Windows (D3D12), and Linux (Vulkan/DMA-BUF).
* Built-in scenes: `cube`, `particles`, `shader_playground`.
* Runtime parameter bridge: `setFloatParam`, `setVec4Param`, `invokeRustCommand`.
* Extensible scene registry via `flutter_wgpu_texture_core` — implement custom
  GPU scenes in Rust without forking the plugin.
