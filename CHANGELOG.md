## 0.2.1

* Refined pub.dev package metadata and topics.

## 0.2.0

* **Web support**: the plugin now runs in any modern browser.
  * Chrome 120+ / Edge 120+: uses WebGPU for GPU-accelerated rendering.
  * Firefox, Safari, Brave, and any other WebGL2-capable browser: automatically
    falls back to a WebGL2 renderer. No configuration required.
  * The active backend is reported in `controller.backendInfo.backend` as
    `"WebGPU"` or `"WebGL2"`.
  * Web renders via an `HtmlElementView` platform view (not a Flutter texture).
    `controller.textureId` is `null` on web.
  * Only the `cube` scene is available on web.
* README and package metadata updated for the 0.2.0 release.

## 0.1.0

* Initial release.
* Renders GPU content into a native Flutter texture using Rust and wgpu.
* Supports macOS (Metal), Windows (D3D12), and Linux (Vulkan/DMA-BUF).
* Built-in scenes: `cube`, `particles`, `shader_playground`.
* Runtime parameter bridge: `setFloatParam`, `setVec4Param`, `invokeRustCommand`.
* Extensible scene registry via `flutter_wgpu_texture_core` — implement custom
  GPU scenes in Rust without forking the plugin.
