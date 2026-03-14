# WebGL2 Fallback for Web Platform

**Date:** 2026-03-14
**Status:** Approved

## Problem

The current web implementation only supports WebGPU (`canvas.getContext('webgpu')`), which requires Chrome 120+ and is unavailable in Firefox stable and Safari without flags. This limits the plugin's web reach significantly.

## Goal

Add a WebGL2 fallback so the plugin works in all modern browsers. When WebGPU is available, use it. When it is not, silently fall back to WebGL2. The user-facing API and widget are unchanged.

## Approach

**Two concrete renderer classes selected at runtime (Approach B).**

Extract an abstract `WebRenderer` interface. Move existing WebGPU logic into `WebGpuRenderer`. Implement a new `WebGlRenderer` using WebGL2 JS interop. The backend file detects `navigator.gpu` at init time and picks the appropriate renderer.

## File Structure

```
lib/src/renderer/
  web/
    web_renderer.dart          # abstract interface
    web_gpu_renderer.dart      # existing WebGPU logic, moved here
    web_gl_renderer.dart       # new WebGL2 implementation
  flutter_wgpu_texture_backend_web.dart   # thin orchestrator, unchanged public API
```

## Abstract Interface

```dart
abstract class WebRenderer {
  Future<void> init(HTMLCanvasElement canvas, String sceneType, Size size);
  Future<void> requestFrame();
  Future<void> resize(Size size);
  Future<void> dispose();
  Future<void> setBoolParam(String key, bool value);
  Future<void> setFloatParam(String key, double value);
  Future<void> setVec4Param(String key, List<double> value);
}
```

## Backend Orchestrator (init excerpt)

```dart
Future<void> init(...) async {
  // Canvas creation and platform view registration stay here
  _canvas = HTMLCanvasElement()...;
  ui_web.platformViewRegistry.registerViewFactory(_viewType, (_) => _canvas);

  final hasWebGpu = js_util.getProperty(window.navigator, 'gpu') != null;
  _renderer = hasWebGpu ? WebGpuRenderer() : WebGlRenderer();
  await _renderer.init(_canvas, sceneType, size);
}
```

All subsequent calls (`requestFrame`, `resize`, params) delegate to `_renderer`.

## Data Flow

### Init
```
backend_web.dart
  → create HTMLCanvasElement, register platform view
  → detect navigator.gpu
  → instantiate WebGpuRenderer or WebGlRenderer
  → renderer.init(canvas, sceneType, size)
      WebGpuRenderer: canvas.getContext('webgpu') → adapter → device → pipeline
      WebGlRenderer:  canvas.getContext('webgl2') → compile shaders → link program → buffers
```

### Per-frame
```
Timer.periodic (16ms)
  → renderer.requestFrame()
      WebGpuRenderer: queue.submit([commandEncoder.finish()])
      WebGlRenderer:  gl.drawElements(TRIANGLES, 36, UNSIGNED_SHORT, 0)
```

### Resize
```
backend_web.dart.resize(size)
  → update canvas CSS + physical dimensions
  → renderer.resize(size)
      WebGpuRenderer: reconfigure context + recreate depth texture
      WebGlRenderer:  gl.viewport(0, 0, w, h) + recreate depth renderbuffer
```

## WebGlRenderer Implementation Notes

- **Shaders:** GLSL ES 3.0, functionally equivalent to current WGSL (MVP matrix + solid cube color)
- **Geometry:** Same vertex/index data as WebGPU renderer
- **Uniforms:** `gl.uniformMatrix4fv` + `gl.uniform4fv`
- **Depth:** `gl.enable(DEPTH_TEST)` + depth renderbuffer (no separate texture needed)
- **Canvas context:** `canvas.getContext('webgl2')`

## Error Handling

- `getContext('webgl2')` returns null → set `_unsupportedReason`, widget shows blank (no crash)
- Shader compile failure → check `COMPILE_STATUS`, log to console, set `_unsupportedReason`
- No retry logic

## Testing

- Unit tests not feasible (require real browser GPU context)
- Manual test matrix:
  - Chrome 120+ → WebGPU path
  - Firefox stable → WebGL2 path
  - Safari → WebGL2 path
- Existing example app used for validation, no new test infrastructure needed

## Out of Scope

- WebGL1 fallback (WebGL2 has sufficient browser support)
- Exposing which renderer is active to the app developer
- Adding new scene types (remains cube-only on web)
