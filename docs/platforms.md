# Platform Notes

## macOS

- `wgpu` backend: Metal
- Flutter presentation: `FlutterTexture`
- Rust renders into a Metal texture created from `CVMetalTexture`

## Windows

- `wgpu` backend: DX12
- Flutter presentation: external GPU surface texture
- Rust exports a DXGI shared handle and the plugin registers it with Flutter

## Linux

- `wgpu` backend: Vulkan
- Flutter presentation: `FlTextureGL`
- Interop path: Vulkan image -> DMA-BUF -> EGL import -> OpenGL texture

The Linux path intentionally avoids CPU readback.

## Not supported yet

- Web
- Android
- iOS
