# Platform Notes

## macOS

**wgpu backend:** Metal
**Flutter presentation:** `FlutterTexture` (pixel buffer path)
**Interop:** Dart calls FRB to create the Rust renderer, then calls the Swift
bridge via method channel to allocate a `CVPixelBuffer` + `CVMetalTexture`.
Swift returns the raw `id<MTLTexture>` pointer back to Dart, which passes it
to Rust via FRB (`attachMetalTexture`). Rust renders directly into the Metal
texture; the Swift bridge signals frame availability to Flutter.

**Setup:**

- Xcode or Xcode Command Line Tools required
- No extra entitlements needed for GPU access on macOS

## Windows

**wgpu backend:** D3D12
**Flutter presentation:** external GPU surface texture
**Interop:** Dart calls FRB (`createDxgiSurface`) to have Rust create a D3D12
texture and export it as a DXGI shared handle. Dart forwards the handle to the
C++ bridge via method channel. The bridge imports the handle and registers it
with the Flutter engine as a GPU surface texture.

**Setup:**

- Visual Studio 2022 with the "Desktop development with C++" workload
- Windows SDK (typically bundled with Visual Studio)

## Linux

**wgpu backend:** Vulkan
**Flutter presentation:** `FlTextureGL`
**Interop:** Dart calls FRB (`ensureLinuxPresent`, `exportDmabuf`) to have
Rust create a Vulkan image and export it as a DMA-BUF file descriptor. Dart
forwards the fd and metadata to the C++ bridge via method channel. The bridge
imports it into EGL as an external image and exposes the resulting OpenGL
texture to the Flutter engine via `FlTextureGL`.

This path deliberately avoids CPU readback — pixel data stays on the GPU
throughout.

**Setup:**

- Vulkan-capable GPU with up-to-date drivers
- `libvulkan-dev`, `libegl-dev` (or Mesa equivalents)
- Flutter Linux desktop toolchain (`clang`, `cmake`, `ninja`, `pkg-config`,
  `libgtk-3-dev`)

On Ubuntu/Debian:

```bash
sudo apt install libvulkan-dev libegl-dev libgtk-3-dev \
  clang cmake ninja-build pkg-config
```

## Not supported

| Platform | Reason |
|----------|--------|
| Android  | `wgpu` Android support and Flutter GPU texture interop not implemented |
| iOS      | Metal interop path not implemented |
| Web      | `wgpu` targets WebGPU but Flutter web texture interop not implemented |

PRs welcome for any of the above.
