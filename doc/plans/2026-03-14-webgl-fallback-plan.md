# WebGL2 Fallback Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a WebGL2 renderer that automatically replaces WebGPU on browsers where `navigator.gpu` is unavailable, using a clean abstract `WebRenderer` interface.

**Architecture:** Extract an abstract `WebRenderer` interface; move existing WebGPU code into `WebGpuRenderer`; implement a new `WebGlRenderer` using WebGL2 JS interop; the backend file detects `navigator.gpu` at init and picks the renderer.

**Tech Stack:** Dart, `dart:js_util`, `package:web`, Flutter platform views, WebGL2, GLSL ES 3.0.

---

### Task 1: Create the abstract WebRenderer interface

**Files:**
- Create: `lib/src/renderer/web/web_renderer.dart`

**Step 1: Create the file**

```dart
// lib/src/renderer/web/web_renderer.dart
import 'dart:typed_data';
import 'dart:ui';

import 'package:web/web.dart' as web;

import '../../rust/api.dart' as rust_api;

/// Common interface for web rendering backends (WebGPU and WebGL2).
abstract class WebRenderer {
  /// Human-readable name of the active backend, e.g. 'WebGPU' or 'WebGL2'.
  String get backendName;

  /// Initialize the renderer with the given canvas, scene type, and size.
  /// Throws if initialization fails.
  Future<void> init(web.HTMLCanvasElement canvas, String sceneType, Size size);

  /// Render one frame.
  void drawFrame(
    double rotation,
    List<double> cubeColor,
    List<double> backgroundColor,
  );

  /// Update GPU resources after a canvas resize.
  void resize(web.HTMLCanvasElement canvas);

  /// Release all GPU resources.
  void dispose();
}
```

**Step 2: Verify file compiles (no errors yet)**

```bash
dart analyze lib/src/renderer/web/web_renderer.dart
```
Expected: no errors (only "nothing to analyze" is also fine at this stage).

**Step 3: Commit**

```bash
git add lib/src/renderer/web/web_renderer.dart
git commit -m "feat(web): add abstract WebRenderer interface"
```

---

### Task 2: Extract WebGpuRenderer from the backend file

**Files:**
- Create: `lib/src/renderer/web/web_gpu_renderer.dart`

**Step 1: Create the file by moving GPU-specific state and methods**

The file contains all WebGPU-specific fields and logic extracted from `flutter_wgpu_texture_backend_web.dart`. Copy the shader source and cube geometry constants too — they belong to the renderer, not the backend.

```dart
// lib/src/renderer/web/web_gpu_renderer.dart
import 'dart:js_util' as js_util;
import 'dart:typed_data';
import 'dart:ui';

import 'package:web/web.dart' as web;

import 'web_renderer.dart';

class WebGpuRenderer implements WebRenderer {
  static const String _shaderSource = '''
struct Uniforms {
  model_view_projection: mat4x4<f32>,
  cube_color: vec4<f32>,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;

struct VertexOutput {
  @builtin(position) position: vec4<f32>,
}

@vertex
fn vs_main(@location(0) position: vec3<f32>) -> VertexOutput {
  var output: VertexOutput;
  output.position = uniforms.model_view_projection * vec4<f32>(position, 1.0);
  return output;
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
  return uniforms.cube_color;
}
''';

  static final Float32List _cubeVertices = Float32List.fromList(<double>[
    -1, -1, 1,   1, -1, 1,   1, 1, 1,   -1, 1, 1,
    -1, -1, -1,  1, -1, -1,  1, 1, -1,  -1, 1, -1,
  ]);

  static final Uint16List _cubeIndices = Uint16List.fromList(<int>[
    0, 1, 2, 2, 3, 0,
    1, 5, 6, 6, 2, 1,
    5, 4, 7, 7, 6, 5,
    4, 0, 3, 3, 7, 4,
    3, 2, 6, 6, 7, 3,
    4, 5, 1, 1, 0, 4,
  ]);

  @override
  String get backendName => 'WebGPU';

  Object? _gpu;
  Object? _device;
  Object? _queue;
  Object? _context;
  Object? _pipeline;
  Object? _uniformBuffer;
  Object? _vertexBuffer;
  Object? _indexBuffer;
  Object? _bindGroup;
  Object? _depthTexture;
  Object? _depthTextureView;
  String? _presentationFormat;

  @override
  Future<void> init(
    web.HTMLCanvasElement canvas,
    String sceneType,
    Size size,
  ) async {
    final gpu = js_util.getProperty<Object?>(web.window.navigator, 'gpu');
    if (gpu == null) {
      throw UnsupportedError(
        'navigator.gpu is unavailable. WebGPU requires a supported browser and HTTPS.',
      );
    }
    _gpu = gpu;

    final adapter = await js_util.promiseToFuture<Object?>(
      js_util.callMethod<Object>(gpu, 'requestAdapter', const <Object?>[]),
    );
    if (adapter == null) throw UnsupportedError('No compatible WebGPU adapter.');

    _device = await js_util.promiseToFuture<Object?>(
      js_util.callMethod<Object>(adapter, 'requestDevice', const <Object?>[]),
    );
    if (_device == null) throw UnsupportedError('Unable to acquire WebGPU device.');

    _queue = js_util.getProperty<Object?>(_device!, 'queue');
    _context = js_util.callMethod<Object?>(canvas, 'getContext', <Object?>['webgpu']);
    if (_context == null) throw UnsupportedError('canvas.getContext("webgpu") returned null.');

    _presentationFormat = js_util.callMethod<String>(
      _gpu!,
      'getPreferredCanvasFormat',
      const <Object?>[],
    );
    _configureContext(canvas);
    _createPipelineResources();
    _recreateDepthTexture(canvas);
  }

  void _configureContext(web.HTMLCanvasElement canvas) {
    js_util.callMethod<void>(_context!, 'configure', <Object?>[
      js_util.jsify(<String, Object?>{
        'device': _device,
        'format': _presentationFormat,
        'alphaMode': 'premultiplied',
      }),
    ]);
  }

  void _createPipelineResources() {
    final shaderModule = js_util.callMethod<Object?>(_device!, 'createShaderModule', <Object?>[
      js_util.jsify(<String, Object?>{'code': _shaderSource}),
    ]);

    _vertexBuffer = _createBuffer(
      data: _cubeVertices,
      usage: _gpuBufferUsage('VERTEX') | _gpuBufferUsage('COPY_DST'),
      label: 'cube vertices',
    );
    _indexBuffer = _createBuffer(
      data: _cubeIndices,
      usage: _gpuBufferUsage('INDEX') | _gpuBufferUsage('COPY_DST'),
      label: 'cube indices',
    );
    _uniformBuffer = js_util.callMethod<Object?>(_device!, 'createBuffer', <Object?>[
      js_util.jsify(<String, Object?>{
        'label': 'cube uniforms',
        'size': 80,
        'usage': _gpuBufferUsage('UNIFORM') | _gpuBufferUsage('COPY_DST'),
      }),
    ]);

    _pipeline = js_util.callMethod<Object?>(_device!, 'createRenderPipeline', <Object?>[
      js_util.jsify(<String, Object?>{
        'layout': 'auto',
        'vertex': <String, Object?>{
          'module': shaderModule,
          'entryPoint': 'vs_main',
          'buffers': <Object?>[
            <String, Object?>{
              'arrayStride': 12,
              'attributes': <Object?>[
                <String, Object?>{'shaderLocation': 0, 'offset': 0, 'format': 'float32x3'},
              ],
            },
          ],
        },
        'fragment': <String, Object?>{
          'module': shaderModule,
          'entryPoint': 'fs_main',
          'targets': <Object?>[<String, Object?>{'format': _presentationFormat}],
        },
        'primitive': <String, Object?>{'topology': 'triangle-list', 'cullMode': 'back'},
        'depthStencil': <String, Object?>{
          'format': 'depth24plus',
          'depthWriteEnabled': true,
          'depthCompare': 'less',
        },
      }),
    ]);

    final bindGroupLayout = js_util.callMethod<Object?>(_pipeline!, 'getBindGroupLayout', <Object?>[0]);
    _bindGroup = js_util.callMethod<Object?>(_device!, 'createBindGroup', <Object?>[
      js_util.jsify(<String, Object?>{
        'layout': bindGroupLayout,
        'entries': <Object?>[
          <String, Object?>{'binding': 0, 'resource': <String, Object?>{'buffer': _uniformBuffer}},
        ],
      }),
    ]);
  }

  void _recreateDepthTexture(web.HTMLCanvasElement canvas) {
    if (_depthTexture != null) {
      js_util.callMethod<void>(_depthTexture!, 'destroy', const <Object?>[]);
    }
    _depthTexture = js_util.callMethod<Object?>(_device!, 'createTexture', <Object?>[
      js_util.jsify(<String, Object?>{
        'size': <String, Object?>{'width': canvas.width, 'height': canvas.height},
        'format': 'depth24plus',
        'usage': _gpuTextureUsage('RENDER_ATTACHMENT'),
      }),
    ]);
    _depthTextureView = js_util.callMethod<Object?>(_depthTexture!, 'createView', const <Object?>[]);
  }

  Object? _createBuffer({required TypedData data, required int usage, required String label}) {
    final buffer = js_util.callMethod<Object?>(_device!, 'createBuffer', <Object?>[
      js_util.jsify(<String, Object?>{
        'label': label,
        'size': data.lengthInBytes,
        'usage': usage,
        'mappedAtCreation': true,
      }),
    ]);
    final mappedRange = js_util.callMethod<Object?>(buffer!, 'getMappedRange', const <Object?>[]);
    if (data is Float32List) {
      final target = js_util.callConstructor<Object?>(
        js_util.getProperty<Object?>(js_util.globalThis, 'Float32Array')!,
        <Object?>[mappedRange],
      );
      js_util.callMethod<void>(target!, 'set', <Object?>[data]);
    } else if (data is Uint16List) {
      final target = js_util.callConstructor<Object?>(
        js_util.getProperty<Object?>(js_util.globalThis, 'Uint16Array')!,
        <Object?>[mappedRange],
      );
      js_util.callMethod<void>(target!, 'set', <Object?>[data]);
    }
    js_util.callMethod<void>(buffer, 'unmap', const <Object?>[]);
    return buffer;
  }

  @override
  void drawFrame(double rotation, List<double> cubeColor, List<double> backgroundColor) {
    final currentTexture = js_util.callMethod<Object?>(_context!, 'getCurrentTexture', const <Object?>[]);
    final currentView = js_util.callMethod<Object?>(currentTexture!, 'createView', const <Object?>[]);

    final uniformData = Float32List(20);
    uniformData.setAll(0, _buildMvpMatrix(rotation));
    uniformData.setAll(16, cubeColor);
    js_util.callMethod<void>(_queue!, 'writeBuffer', <Object?>[_uniformBuffer, 0, uniformData]);

    final commandEncoder = js_util.callMethod<Object?>(_device!, 'createCommandEncoder', const <Object?>[]);
    final renderPass = js_util.callMethod<Object?>(commandEncoder!, 'beginRenderPass', <Object?>[
      js_util.jsify(<String, Object?>{
        'colorAttachments': <Object?>[
          <String, Object?>{
            'view': currentView,
            'clearValue': <String, Object?>{
              'r': backgroundColor[0],
              'g': backgroundColor[1],
              'b': backgroundColor[2],
              'a': backgroundColor[3],
            },
            'loadOp': 'clear',
            'storeOp': 'store',
          },
        ],
        'depthStencilAttachment': <String, Object?>{
          'view': _depthTextureView,
          'depthClearValue': 1.0,
          'depthLoadOp': 'clear',
          'depthStoreOp': 'store',
        },
      }),
    ]);

    js_util.callMethod<void>(renderPass!, 'setPipeline', <Object?>[_pipeline]);
    js_util.callMethod<void>(renderPass, 'setBindGroup', <Object?>[0, _bindGroup]);
    js_util.callMethod<void>(renderPass, 'setVertexBuffer', <Object?>[0, _vertexBuffer]);
    js_util.callMethod<void>(renderPass, 'setIndexBuffer', <Object?>[_indexBuffer, 'uint16']);
    js_util.callMethod<void>(renderPass, 'drawIndexed', <Object?>[_cubeIndices.length, 1, 0, 0, 0]);
    js_util.callMethod<void>(renderPass, 'end', const <Object?>[]);

    final commandBuffer = js_util.callMethod<Object?>(commandEncoder, 'finish', const <Object?>[]);
    js_util.callMethod<void>(_queue!, 'submit', <Object?>[<Object?>[commandBuffer]]);
  }

  @override
  void resize(web.HTMLCanvasElement canvas) {
    _configureContext(canvas);
    _recreateDepthTexture(canvas);
  }

  @override
  void dispose() {
    if (_depthTexture != null) {
      js_util.callMethod<void>(_depthTexture!, 'destroy', const <Object?>[]);
      _depthTexture = null;
    }
    _depthTextureView = null;
    _device = null;
    _queue = null;
    _context = null;
    _pipeline = null;
    _uniformBuffer = null;
    _vertexBuffer = null;
    _indexBuffer = null;
    _bindGroup = null;
  }

  int _gpuBufferUsage(String key) {
    final g = js_util.getProperty<Object?>(js_util.globalThis, 'GPUBufferUsage')!;
    return js_util.getProperty<int>(g, key);
  }

  int _gpuTextureUsage(String key) {
    final g = js_util.getProperty<Object?>(js_util.globalThis, 'GPUTextureUsage')!;
    return js_util.getProperty<int>(g, key);
  }

  // Matrix math (identical to original backend file)
  Float32List _buildMvpMatrix(double angle) {
    import 'dart:math' as math;  // NOTE: add math import at top of file
    final aspect = 1.0; // placeholder — canvas ratio passed via init size
    // The actual implementation uses _canvas.width/_canvas.height.
    // Store canvas ref or width/height in init and use here.
    // See full note in Step 2 below.
    return Float32List(16); // replaced in Step 2
  }
}
```

> **Note:** The matrix math methods (`_buildModelViewProjectionMatrix`, `_perspectiveMatrix`, etc.) from the original file must be copied verbatim into this class. They are ~70 lines total and depend only on `dart:math` and `dart:typed_data`. Also store `canvas.width` and `canvas.height` as instance fields in `init()` so `drawFrame` can compute the aspect ratio — the canvas reference is not stored, only its dimensions at init/resize time.

**Step 2: Add the matrix math and fix the aspect ratio**

At the top of `web_gpu_renderer.dart` add `import 'dart:math' as math;`.

Add instance fields:
```dart
int _canvasWidth = 1;
int _canvasHeight = 1;
```

In `init()`, after canvas setup:
```dart
_canvasWidth = canvas.width;
_canvasHeight = canvas.height;
```

In `resize()`:
```dart
_canvasWidth = canvas.width;
_canvasHeight = canvas.height;
```

Replace `_buildMvpMatrix` with the full implementation from `flutter_wgpu_texture_backend_web.dart:595-603`, using `_canvasWidth` and `_canvasHeight`. Copy `_perspectiveMatrix`, `_translationMatrix`, `_rotationYMatrix`, `_rotationXMatrix`, `_multiplyMatrices` verbatim from lines 606–665 of the original.

**Step 3: Analyze**

```bash
dart analyze lib/src/renderer/web/
```
Expected: no errors.

**Step 4: Commit**

```bash
git add lib/src/renderer/web/web_gpu_renderer.dart
git commit -m "feat(web): extract WebGpuRenderer from backend"
```

---

### Task 3: Implement WebGlRenderer

**Files:**
- Create: `lib/src/renderer/web/web_gl_renderer.dart`

**Step 1: Create the file**

```dart
// lib/src/renderer/web/web_gl_renderer.dart
import 'dart:js_util' as js_util;
import 'dart:math' as math;
import 'dart:typed_data';
import 'dart:ui';

import 'package:web/web.dart' as web;

import 'web_renderer.dart';

class WebGlRenderer implements WebRenderer {
  static const String _vertexShaderSource = '''
#version 300 es
precision highp float;

layout(location = 0) in vec3 a_position;

uniform mat4 u_mvp;

void main() {
  gl_Position = u_mvp * vec4(a_position, 1.0);
}
''';

  static const String _fragmentShaderSource = '''
#version 300 es
precision highp float;

uniform vec4 u_color;

out vec4 fragColor;

void main() {
  fragColor = u_color;
}
''';

  static final Float32List _cubeVertices = Float32List.fromList(<double>[
    -1, -1, 1,   1, -1, 1,   1, 1, 1,   -1, 1, 1,
    -1, -1, -1,  1, -1, -1,  1, 1, -1,  -1, 1, -1,
  ]);

  static final Uint16List _cubeIndices = Uint16List.fromList(<int>[
    0, 1, 2, 2, 3, 0,
    1, 5, 6, 6, 2, 1,
    5, 4, 7, 7, 6, 5,
    4, 0, 3, 3, 7, 4,
    3, 2, 6, 6, 7, 3,
    4, 5, 1, 1, 0, 4,
  ]);

  @override
  String get backendName => 'WebGL2';

  Object? _gl;
  Object? _program;
  Object? _vao;
  Object? _vertexBuffer;
  Object? _indexBuffer;
  int _mvpLocation = -1;
  int _colorLocation = -1;
  int _canvasWidth = 1;
  int _canvasHeight = 1;

  @override
  Future<void> init(
    web.HTMLCanvasElement canvas,
    String sceneType,
    Size size,
  ) async {
    _canvasWidth = canvas.width;
    _canvasHeight = canvas.height;

    _gl = js_util.callMethod<Object?>(canvas, 'getContext', <Object?>['webgl2']);
    if (_gl == null) {
      throw UnsupportedError('canvas.getContext("webgl2") returned null.');
    }

    _program = _createProgram(_vertexShaderSource, _fragmentShaderSource);
    _setupGeometry();

    _mvpLocation = _getUniformLocation('u_mvp');
    _colorLocation = _getUniformLocation('u_color');

    _gl!
      .._enable(0x0B71)    // DEPTH_TEST
      .._depthFunc(0x0201); // LESS
  }

  Object _createProgram(String vertSrc, String fragSrc) {
    final gl = _gl!;
    final vert = _compileShader(0x8B31, vertSrc); // VERTEX_SHADER
    final frag = _compileShader(0x8B30, fragSrc); // FRAGMENT_SHADER

    final program = js_util.callMethod<Object>(gl, 'createProgram', const <Object?>[]);
    js_util.callMethod<void>(gl, 'attachShader', <Object?>[program, vert]);
    js_util.callMethod<void>(gl, 'attachShader', <Object?>[program, frag]);
    js_util.callMethod<void>(gl, 'linkProgram', <Object?>[program]);

    final linked = js_util.callMethod<bool>(
      gl,
      'getProgramParameter',
      <Object?>[program, 0x8B82], // LINK_STATUS
    );
    if (!linked) {
      final log = js_util.callMethod<String>(gl, 'getProgramInfoLog', <Object?>[program]);
      throw StateError('WebGL program link failed: $log');
    }

    js_util.callMethod<void>(gl, 'deleteShader', <Object?>[vert]);
    js_util.callMethod<void>(gl, 'deleteShader', <Object?>[frag]);
    return program;
  }

  Object _compileShader(int type, String source) {
    final gl = _gl!;
    final shader = js_util.callMethod<Object>(gl, 'createShader', <Object?>[type]);
    js_util.callMethod<void>(gl, 'shaderSource', <Object?>[shader, source]);
    js_util.callMethod<void>(gl, 'compileShader', <Object?>[shader]);

    final compiled = js_util.callMethod<bool>(
      gl,
      'getShaderParameter',
      <Object?>[shader, 0x8B81], // COMPILE_STATUS
    );
    if (!compiled) {
      final log = js_util.callMethod<String>(gl, 'getShaderInfoLog', <Object?>[shader]);
      throw StateError('WebGL shader compile failed: $log');
    }
    return shader;
  }

  void _setupGeometry() {
    final gl = _gl!;

    _vao = js_util.callMethod<Object>(gl, 'createVertexArray', const <Object?>[]);
    js_util.callMethod<void>(gl, 'bindVertexArray', <Object?>[_vao]);

    _vertexBuffer = js_util.callMethod<Object>(gl, 'createBuffer', const <Object?>[]);
    js_util.callMethod<void>(gl, 'bindBuffer', <Object?>[0x8892, _vertexBuffer]); // ARRAY_BUFFER
    js_util.callMethod<void>(gl, 'bufferData', <Object?>[0x8892, _cubeVertices, 0x88B8]); // STATIC_DRAW
    js_util.callMethod<void>(gl, 'enableVertexAttribArray', <Object?>[0]);
    js_util.callMethod<void>(gl, 'vertexAttribPointer', <Object?>[0, 3, 0x1406, false, 12, 0]); // FLOAT

    _indexBuffer = js_util.callMethod<Object>(gl, 'createBuffer', const <Object?>[]);
    js_util.callMethod<void>(gl, 'bindBuffer', <Object?>[0x8893, _indexBuffer]); // ELEMENT_ARRAY_BUFFER
    js_util.callMethod<void>(gl, 'bufferData', <Object?>[0x8893, _cubeIndices, 0x88B8]); // STATIC_DRAW

    js_util.callMethod<void>(gl, 'bindVertexArray', <Object?>[null]);
  }

  int _getUniformLocation(String name) {
    final loc = js_util.callMethod<Object?>(
      _gl!,
      'getUniformLocation',
      <Object?>[_program, name],
    );
    if (loc == null) throw StateError('WebGL uniform "$name" not found.');
    // Location objects are opaque JS objects, not integers. Do not store them as `int`.
    // We store the raw JS object and cast it when needed.
    return 0; // placeholder — see note below
  }

  @override
  void drawFrame(double rotation, List<double> cubeColor, List<double> backgroundColor) {
    final gl = _gl!;

    js_util.callMethod<void>(gl, 'viewport', <Object?>[0, 0, _canvasWidth, _canvasHeight]);
    js_util.callMethod<void>(gl, 'clearColor', <Object?>[
      backgroundColor[0],
      backgroundColor[1],
      backgroundColor[2],
      backgroundColor[3],
    ]);
    js_util.callMethod<void>(gl, 'clear', <Object?>[0x4100]); // COLOR_BUFFER_BIT | DEPTH_BUFFER_BIT

    js_util.callMethod<void>(gl, 'useProgram', <Object?>[_program]);
    js_util.callMethod<void>(gl, 'bindVertexArray', <Object?>[_vao]);

    final mvp = _buildMvpMatrix(rotation);
    final mvpLoc = js_util.callMethod<Object?>(
      gl, 'getUniformLocation', <Object?>[_program, 'u_mvp']);
    js_util.callMethod<void>(gl, 'uniformMatrix4fv', <Object?>[mvpLoc, false, mvp]);

    final colorLoc = js_util.callMethod<Object?>(
      gl, 'getUniformLocation', <Object?>[_program, 'u_color']);
    js_util.callMethod<void>(gl, 'uniform4fv', <Object?>[
      colorLoc,
      Float32List.fromList(cubeColor),
    ]);

    js_util.callMethod<void>(gl, 'drawElements', <Object?>[
      0x0004,                   // TRIANGLES
      _cubeIndices.length,
      0x1403,                   // UNSIGNED_SHORT
      0,
    ]);

    js_util.callMethod<void>(gl, 'bindVertexArray', <Object?>[null]);
  }

  @override
  void resize(web.HTMLCanvasElement canvas) {
    _canvasWidth = canvas.width;
    _canvasHeight = canvas.height;
    // viewport is set per-frame in drawFrame
  }

  @override
  void dispose() {
    final gl = _gl;
    if (gl == null) return;
    js_util.callMethod<void>(gl, 'deleteProgram', <Object?>[_program]);
    js_util.callMethod<void>(gl, 'deleteBuffer', <Object?>[_vertexBuffer]);
    js_util.callMethod<void>(gl, 'deleteBuffer', <Object?>[_indexBuffer]);
    js_util.callMethod<void>(gl, 'deleteVertexArray', <Object?>[_vao]);
    _gl = null;
    _program = null;
    _vao = null;
    _vertexBuffer = null;
    _indexBuffer = null;
  }

  // Matrix math — identical to WebGpuRenderer
  Float32List _buildMvpMatrix(double angle) {
    final aspect = _canvasHeight == 0 ? 1.0 : _canvasWidth / _canvasHeight;
    final projection = _perspectiveMatrix(math.pi / 4, aspect, 0.1, 100);
    final rotY = _rotationYMatrix(angle);
    final rotX = _rotationXMatrix(angle * 0.7);
    final translation = _translationMatrix(0, 0, -5.5);
    final model = _multiplyMatrices(rotY, rotX);
    final viewModel = _multiplyMatrices(translation, model);
    return _multiplyMatrices(projection, viewModel);
  }

  Float32List _perspectiveMatrix(double fovY, double aspect, double near, double far) {
    final f = 1.0 / math.tan(fovY / 2.0);
    final rangeInv = 1.0 / (near - far);
    return Float32List.fromList(<double>[
      f / aspect, 0, 0, 0,
      0, f, 0, 0,
      0, 0, (near + far) * rangeInv, -1,
      0, 0, near * far * rangeInv * 2.0, 0,
    ]);
  }

  Float32List _translationMatrix(double x, double y, double z) {
    return Float32List.fromList(<double>[
      1, 0, 0, 0,
      0, 1, 0, 0,
      0, 0, 1, 0,
      x, y, z, 1,
    ]);
  }

  Float32List _rotationYMatrix(double angle) {
    final c = math.cos(angle);
    final s = math.sin(angle);
    return Float32List.fromList(<double>[c, 0, -s, 0, 0, 1, 0, 0, s, 0, c, 0, 0, 0, 0, 1]);
  }

  Float32List _rotationXMatrix(double angle) {
    final c = math.cos(angle);
    final s = math.sin(angle);
    return Float32List.fromList(<double>[1, 0, 0, 0, 0, c, s, 0, 0, -s, c, 0, 0, 0, 0, 1]);
  }

  Float32List _multiplyMatrices(Float32List a, Float32List b) {
    final out = Float32List(16);
    for (var row = 0; row < 4; row++) {
      for (var col = 0; col < 4; col++) {
        double sum = 0;
        for (var i = 0; i < 4; i++) {
          sum += a[(i * 4) + row] * b[(col * 4) + i];
        }
        out[(col * 4) + row] = sum;
      }
    }
    return out;
  }
}
```

> **Note on uniform locations:** WebGL uniform location objects are opaque JS objects, not integers. Do not store them as `int`. Instead, call `gl.getUniformLocation` directly in `drawFrame` as shown (this is fine for 2 uniforms per frame — no measurable overhead). Remove the `_mvpLocation` and `_colorLocation` int fields from the class.

**Step 2: Analyze**

```bash
dart analyze lib/src/renderer/web/
```
Expected: no errors.

**Step 3: Commit**

```bash
git add lib/src/renderer/web/web_gl_renderer.dart
git commit -m "feat(web): implement WebGlRenderer with WebGL2"
```

---

### Task 4: Refactor backend_web.dart to use the renderer abstraction

**Files:**
- Modify: `lib/src/renderer/flutter_wgpu_texture_backend_web.dart`

**Step 1: Replace the file content**

The new version keeps only: canvas creation, platform view registration, animation loop, param state, and delegation to `_renderer`. All GPU code is gone.

```dart
// lib/src/renderer/flutter_wgpu_texture_backend_web.dart
import 'dart:async';
import 'dart:js_util' as js_util;
import 'dart:math' as math;
import 'dart:ui';
import 'dart:ui_web' as ui_web;

import 'package:flutter/scheduler.dart';
import 'package:web/web.dart' as web;

import '../rust/api.dart' as rust_api;
import 'flutter_wgpu_texture_backend.dart';
import 'web/web_gl_renderer.dart';
import 'web/web_gpu_renderer.dart';
import 'web/web_renderer.dart';

FlutterWgpuTextureBackend createFlutterWgpuTextureBackend({
  required bool autoStart,
  required String sceneType,
  required String surfaceId,
}) {
  return _WebFlutterWgpuTextureBackend(
    autoStart: autoStart,
    sceneType: sceneType,
    surfaceId: surfaceId,
  );
}

class _WebFlutterWgpuTextureBackend implements FlutterWgpuTextureBackend {
  _WebFlutterWgpuTextureBackend({
    required this.autoStart,
    required this.sceneType,
    required this.surfaceId,
  }) : _viewType = 'flutter_wgpu_texture_$surfaceId' {
    _canvas = web.HTMLCanvasElement()
      ..style.width = '100%'
      ..style.height = '100%'
      ..style.display = 'block';
    ui_web.platformViewRegistry.registerViewFactory(_viewType, (_) => _canvas);
  }

  final bool autoStart;
  final String sceneType;
  final String surfaceId;
  final String _viewType;

  late final web.HTMLCanvasElement _canvas;

  rust_api.BackendInfo? _backendInfo;
  Size? _size;
  bool _initialized = false;
  bool _animating = false;
  String? _unsupportedReason;
  double _rotation = 0;
  double _rotationSpeed = 1.0;
  bool _rotationEnabled = true;
  List<double> _cubeColor = const <double>[1.0, 0.831, 0.0, 1.0];
  List<double> _backgroundColor = const <double>[0.106, 0.361, 1.0, 1.0];
  Timer? _animationTimer;
  bool _frameInFlight = false;

  WebRenderer? _renderer;

  @override
  rust_api.BackendInfo? get backendInfo => _backendInfo;

  @override
  BigInt? get handle => null;

  @override
  bool get isAnimating => _animating;

  @override
  bool get isInitialized => _initialized;

  @override
  Size? get size => _size;

  @override
  int? get textureId => null;

  @override
  String? get unsupportedReason => _unsupportedReason;

  @override
  String? get viewType => _viewType;

  @override
  Future<void> ensureInitialized(Size size, TickerProvider vsync) async {
    final logicalWidth = math.max(1, size.width.round());
    final logicalHeight = math.max(1, size.height.round());
    _size = Size(logicalWidth.toDouble(), logicalHeight.toDouble());
    _resizeCanvas(_size!);

    if (_initialized) {
      _renderer?.resize(_canvas);
      await requestFrame();
      return;
    }

    if (sceneType != 'cube') {
      _unsupportedReason =
          'Web currently supports the "cube" scene only (requested: $sceneType).';
      return;
    }

    final hasWebGpu =
        js_util.getProperty<Object?>(web.window.navigator, 'gpu') != null;
    _renderer = hasWebGpu ? WebGpuRenderer() : WebGlRenderer();

    try {
      await _renderer!.init(_canvas, sceneType, size);
      _initialized = true;
      _backendInfo = rust_api.BackendInfo(
        backend: _renderer!.backendName,
        deviceName: 'Browser GPU Adapter',
        driver: 'web',
      );
      await requestFrame();
      if (autoStart) await startAnimation();
    } catch (error) {
      _unsupportedReason = '${_renderer!.backendName} initialization failed: $error';
      _renderer = null;
    }
  }

  @override
  Future<void> dispose() async {
    await stopAnimation();
    _renderer?.dispose();
    _renderer = null;
    _initialized = false;
    _backendInfo = null;
    _size = null;
  }

  @override
  Future<void> startAnimation() async {
    if (!_initialized || _animating) return;
    _animating = true;
    _animationTimer = Timer.periodic(const Duration(milliseconds: 16), (_) {
      if (_rotationEnabled) _rotation += _rotationSpeed / 60.0;
      unawaited(requestFrame());
    });
  }

  @override
  Future<void> stopAnimation() async {
    _animating = false;
    _animationTimer?.cancel();
    _animationTimer = null;
  }

  @override
  Future<void> requestFrame() async {
    if (!_initialized || _frameInFlight || _renderer == null) return;
    _frameInFlight = true;
    try {
      _renderer!.drawFrame(_rotation, _cubeColor, _backgroundColor);
    } finally {
      _frameInFlight = false;
    }
  }

  @override
  Future<void> setBoolParam(String key, bool value) async {
    if (key == 'rotation_enabled') {
      _rotationEnabled = value;
      await requestFrame();
    }
  }

  @override
  Future<void> setFloatParam(String key, double value) async {
    if (key == 'rotation_speed') {
      _rotationSpeed = value;
      await requestFrame();
    }
  }

  @override
  Future<void> setVec4Param(String key, List<double> value) async {
    if (value.length != 4) return;
    if (key == 'cube_color') {
      _cubeColor = List<double>.from(value);
    } else if (key == 'background_color') {
      _backgroundColor = List<double>.from(value);
    }
    await requestFrame();
  }

  @override
  Future<void> invokeCommand(String command, {String payload = '{}'}) async {
    if (command == 'reset_scene') {
      _rotation = 0;
      await requestFrame();
    }
  }

  void _resizeCanvas(Size size) {
    final cssWidth = math.max(1, size.width.round());
    final cssHeight = math.max(1, size.height.round());
    final devicePixelRatio = web.window.devicePixelRatio;
    final physicalWidth = math.max(1, (cssWidth * devicePixelRatio).round());
    final physicalHeight = math.max(1, (cssHeight * devicePixelRatio).round());

    _canvas
      ..width = physicalWidth
      ..height = physicalHeight
      ..style.width = '${cssWidth}px'
      ..style.height = '${cssHeight}px';
  }
}
```

**Step 2: Analyze the whole lib**

```bash
dart analyze lib/
```
Expected: no errors.

**Step 3: Commit**

```bash
git add lib/src/renderer/flutter_wgpu_texture_backend_web.dart
git commit -m "refactor(web): delegate rendering to WebRenderer abstraction"
```

---

### Task 5: Final verification

**Step 1: Full analyze**

```bash
dart analyze .
```
Expected: no errors or warnings.

**Step 2: Build the web example**

```bash
cd example && flutter build web --target lib/main.dart
```
Expected: build succeeds with no errors.

**Step 3: Manual browser test (Chrome — WebGPU path)**

```bash
cd example && flutter run -d chrome
```
Expected: spinning cube renders, `backendInfo.backend` shows `'WebGPU'`.

**Step 4: Manual browser test (Firefox — WebGL2 path)**

Open the built web app in Firefox. Expected: spinning cube renders, `backendInfo.backend` shows `'WebGL2'`.

**Step 5: Commit if any fixes were needed, then tag**

```bash
git commit -m "fix(web): address any issues found during verification" # only if needed
```

---

### Summary of files

| Action | File |
|--------|------|
| Create | `lib/src/renderer/web/web_renderer.dart` |
| Create | `lib/src/renderer/web/web_gpu_renderer.dart` |
| Create | `lib/src/renderer/web/web_gl_renderer.dart` |
| Modify | `lib/src/renderer/flutter_wgpu_texture_backend_web.dart` |
