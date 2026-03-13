# Platform Notes

## macOS

**wgpu backend:** Metal
**Flutter presentation:** `FlutterTexture` (pixel buffer path)
**Interop:** Rust renders into a `CVMetalTexture`; the macOS plugin registers
this texture with the Flutter engine as a `FlutterTexture` and calls
`markTextureFrameAvailable` after each render.

**Setup:**

- Xcode or Xcode Command Line Tools required
- No extra entitlements needed for GPU access on macOS

## Windows

**wgpu backend:** D3D12
**Flutter presentation:** external GPU surface texture
**Interop:** Rust creates a D3D12 texture and exports it as a DXGI shared
handle. The Windows plugin imports the handle and registers it with the
Flutter engine as a GPU surface texture.

**Setup:**

- Visual Studio 2022 with the "Desktop development with C++" workload
- Windows SDK (typically bundled with Visual Studio)

## Linux

**wgpu backend:** Vulkan
**Flutter presentation:** `FlTextureGL`
**Interop:** Rust renders into a Vulkan image, exports it as a DMA-BUF file
descriptor, imports that into EGL as an external image, and exposes the
resulting OpenGL texture id to the Flutter engine via `FlTextureGL`.

This path deliberately avoids CPU readback — the pixel data stays on the GPU
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
