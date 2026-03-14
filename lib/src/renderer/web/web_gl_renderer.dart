import 'dart:js_util' as js_util;
import 'dart:math' as math;
import 'dart:typed_data';
import 'dart:ui';

import 'package:web/web.dart' as web;

import 'web_renderer.dart';

/// WebGL2-based implementation of [WebRenderer].
class WebGlRenderer implements WebRenderer {
  @override
  String get backendName => 'WebGL2';

  // ---------------------------------------------------------------------------
  // GLSL ES 3.0 shaders
  // ---------------------------------------------------------------------------

  static const String _vertShaderSource = '''
#version 300 es
precision highp float;
layout(location = 0) in vec3 a_position;
uniform mat4 u_mvp;
void main() {
  gl_Position = u_mvp * vec4(a_position, 1.0);
}
''';

  static const String _fragShaderSource = '''
#version 300 es
precision highp float;
uniform vec4 u_color;
out vec4 fragColor;
void main() {
  fragColor = u_color;
}
''';

  // ---------------------------------------------------------------------------
  // Cube geometry (identical to WebGpuRenderer)
  // ---------------------------------------------------------------------------

  static final Float32List _cubeVertices = Float32List.fromList(<double>[
    -1, -1, 1,
    1, -1, 1,
    1, 1, 1,
    -1, 1, 1,
    -1, -1, -1,
    1, -1, -1,
    1, 1, -1,
    -1, 1, -1,
  ]);

  static final Uint16List _cubeIndices = Uint16List.fromList(<int>[
    0, 1, 2, 2, 3, 0,
    1, 5, 6, 6, 2, 1,
    5, 4, 7, 7, 6, 5,
    4, 0, 3, 3, 7, 4,
    3, 2, 6, 6, 7, 3,
    4, 5, 1, 1, 0, 4,
  ]);

  // ---------------------------------------------------------------------------
  // Instance fields
  // ---------------------------------------------------------------------------

  Object? _gl;
  Object? _program;
  Object? _vao;
  Object? _vertexBuffer;
  Object? _indexBuffer;

  Object? _mvpUniformLocation;
  Object? _colorUniformLocation;

  int _canvasWidth = 1;
  int _canvasHeight = 1;

  // ---------------------------------------------------------------------------
  // WebRenderer interface
  // ---------------------------------------------------------------------------

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
      throw UnsupportedError(
        'canvas.getContext("webgl2") returned null. '
        'WebGL2 is not supported in this browser.',
      );
    }

    _program = _createProgram(_vertShaderSource, _fragShaderSource);
    _setupGeometry();

    _mvpUniformLocation = js_util.callMethod<Object?>(
      _gl!, 'getUniformLocation', <Object?>[_program, 'u_mvp']);
    _colorUniformLocation = js_util.callMethod<Object?>(
      _gl!, 'getUniformLocation', <Object?>[_program, 'u_color']);
    if (_mvpUniformLocation == null || _colorUniformLocation == null) {
      throw StateError('WebGL2: could not find expected uniforms u_mvp or u_color.');
    }

    // Enable depth testing.
    js_util.callMethod<void>(_gl!, 'enable', <Object?>[0x0B71]); // DEPTH_TEST
    js_util.callMethod<void>(_gl!, 'depthFunc', <Object?>[0x0201]); // LESS
  }

  @override
  void drawFrame(
    double rotation,
    List<double> cubeColor,
    List<double> backgroundColor,
  ) {
    final gl = _gl!;

    // 1. Viewport
    js_util.callMethod<void>(
      gl,
      'viewport',
      <Object?>[0, 0, _canvasWidth, _canvasHeight],
    );

    // 2. Clear color
    js_util.callMethod<void>(gl, 'clearColor', <Object?>[
      backgroundColor[0],
      backgroundColor[1],
      backgroundColor[2],
      backgroundColor[3],
    ]);

    // 3. Clear — COLOR_BUFFER_BIT | DEPTH_BUFFER_BIT = 0x4100
    js_util.callMethod<void>(gl, 'clear', <Object?>[0x4100]);

    // 4. Use program
    js_util.callMethod<void>(gl, 'useProgram', <Object?>[_program]);

    // 5. Bind VAO
    js_util.callMethod<void>(gl, 'bindVertexArray', <Object?>[_vao]);

    // 6. MVP uniform
    final mvpMatrix = _buildMvpMatrix(rotation);
    js_util.callMethod<void>(
      gl,
      'uniformMatrix4fv',
      <Object?>[_mvpUniformLocation, false, mvpMatrix],
    );

    // 7. Color uniform
    js_util.callMethod<void>(
      gl,
      'uniform4fv',
      <Object?>[_colorUniformLocation, Float32List.fromList(cubeColor)],
    );

    // 8. Draw — TRIANGLES=0x0004, UNSIGNED_SHORT=0x1403
    js_util.callMethod<void>(
      gl,
      'drawElements',
      <Object?>[0x0004, _cubeIndices.length, 0x1403, 0],
    );

    // 9. Unbind VAO
    js_util.callMethod<void>(gl, 'bindVertexArray', <Object?>[null]);
  }

  @override
  void resize(web.HTMLCanvasElement canvas) {
    _canvasWidth = canvas.width;
    _canvasHeight = canvas.height;
    // viewport is set per-frame in drawFrame.
  }

  @override
  void dispose() {
    if (_gl == null) return;
    final gl = _gl!;
    if (_program != null) {
      js_util.callMethod<void>(gl, 'deleteProgram', <Object?>[_program]);
      _program = null;
    }
    if (_vertexBuffer != null) {
      js_util.callMethod<void>(gl, 'deleteBuffer', <Object?>[_vertexBuffer]);
      _vertexBuffer = null;
    }
    if (_indexBuffer != null) {
      js_util.callMethod<void>(gl, 'deleteBuffer', <Object?>[_indexBuffer]);
      _indexBuffer = null;
    }
    if (_vao != null) {
      js_util.callMethod<void>(gl, 'deleteVertexArray', <Object?>[_vao]);
      _vao = null;
    }
    _mvpUniformLocation = null;
    _colorUniformLocation = null;
    _gl = null;
  }

  // ---------------------------------------------------------------------------
  // Private helpers
  // ---------------------------------------------------------------------------

  Object _createProgram(String vertSrc, String fragSrc) {
    final gl = _gl!;
    final vert = _compileShader(0x8B31, vertSrc); // VERTEX_SHADER
    final frag = _compileShader(0x8B30, fragSrc); // FRAGMENT_SHADER

    final program = js_util.callMethod<Object?>(gl, 'createProgram', const <Object?>[])!;
    js_util.callMethod<void>(gl, 'attachShader', <Object?>[program, vert]);
    js_util.callMethod<void>(gl, 'attachShader', <Object?>[program, frag]);
    js_util.callMethod<void>(gl, 'linkProgram', <Object?>[program]);

    // Check LINK_STATUS (0x8B82)
    final linked = js_util.callMethod<bool>(
      gl,
      'getProgramParameter',
      <Object?>[program, 0x8B82],
    );
    if (!linked) {
      final log = js_util.callMethod<String?>(gl, 'getProgramInfoLog', <Object?>[program]) ?? '';
      js_util.callMethod<void>(gl, 'deleteShader', <Object?>[vert]);
      js_util.callMethod<void>(gl, 'deleteShader', <Object?>[frag]);
      js_util.callMethod<void>(gl, 'deleteProgram', <Object?>[program]);
      throw StateError('WebGL2 program link failed: $log');
    }

    // Delete shaders after linking — they are no longer needed.
    js_util.callMethod<void>(gl, 'deleteShader', <Object?>[vert]);
    js_util.callMethod<void>(gl, 'deleteShader', <Object?>[frag]);

    return program;
  }

  Object _compileShader(int type, String source) {
    final gl = _gl!;
    final shader = js_util.callMethod<Object?>(gl, 'createShader', <Object?>[type])!;
    js_util.callMethod<void>(gl, 'shaderSource', <Object?>[shader, source]);
    js_util.callMethod<void>(gl, 'compileShader', <Object?>[shader]);

    // Check COMPILE_STATUS (0x8B81)
    final compiled = js_util.callMethod<bool>(
      gl,
      'getShaderParameter',
      <Object?>[shader, 0x8B81],
    );
    if (!compiled) {
      final log = js_util.callMethod<String?>(gl, 'getShaderInfoLog', <Object?>[shader]) ?? '';
      js_util.callMethod<void>(gl, 'deleteShader', <Object?>[shader]);
      throw StateError(
        'WebGL2 shader compile failed: $log',
      );
    }

    return shader;
  }

  void _setupGeometry() {
    final gl = _gl!;

    // Create and bind VAO.
    _vao = js_util.callMethod<Object?>(gl, 'createVertexArray', const <Object?>[]);
    js_util.callMethod<void>(gl, 'bindVertexArray', <Object?>[_vao]);

    // Vertex buffer — ARRAY_BUFFER = 0x8892, STATIC_DRAW = 0x88B8
    _vertexBuffer = js_util.callMethod<Object?>(gl, 'createBuffer', const <Object?>[]);
    js_util.callMethod<void>(
      gl,
      'bindBuffer',
      <Object?>[0x8892, _vertexBuffer],
    );
    js_util.callMethod<void>(
      gl,
      'bufferData',
      <Object?>[0x8892, _cubeVertices, 0x88B8],
    );

    // Attribute 0: vec3 position, stride 12 bytes, FLOAT = 0x1406
    js_util.callMethod<void>(gl, 'enableVertexAttribArray', <Object?>[0]);
    js_util.callMethod<void>(
      gl,
      'vertexAttribPointer',
      <Object?>[0, 3, 0x1406, false, 12, 0],
    );

    // Index buffer — ELEMENT_ARRAY_BUFFER = 0x8893
    _indexBuffer =
        js_util.callMethod<Object?>(gl, 'createBuffer', const <Object?>[]);
    js_util.callMethod<void>(
      gl,
      'bindBuffer',
      <Object?>[0x8893, _indexBuffer],
    );
    js_util.callMethod<void>(
      gl,
      'bufferData',
      <Object?>[0x8893, _cubeIndices, 0x88B8],
    );

    // Unbind VAO.
    js_util.callMethod<void>(gl, 'bindVertexArray', <Object?>[null]);
  }

  // ---------------------------------------------------------------------------
  // Matrix math (verbatim copy from WebGpuRenderer)
  // ---------------------------------------------------------------------------

  Float32List _buildMvpMatrix(double angle) {
    final aspect = _canvasHeight == 0 ? 1.0 : _canvasWidth / _canvasHeight;
    final projection = _perspectiveMatrix(math.pi / 4, aspect, 0.1, 100);
    final rotationY = _rotationYMatrix(angle);
    final rotationX = _rotationXMatrix(angle * 0.7);
    final translation = _translationMatrix(0, 0, -5.5);
    final model = _multiplyMatrices(rotationY, rotationX);
    final viewModel = _multiplyMatrices(translation, model);
    return _multiplyMatrices(projection, viewModel);
  }

  Float32List _perspectiveMatrix(
    double fovY,
    double aspect,
    double near,
    double far,
  ) {
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
    return Float32List.fromList(<double>[
      c, 0, -s, 0,
      0, 1, 0, 0,
      s, 0, c, 0,
      0, 0, 0, 1,
    ]);
  }

  Float32List _rotationXMatrix(double angle) {
    final c = math.cos(angle);
    final s = math.sin(angle);
    return Float32List.fromList(<double>[
      1, 0, 0, 0,
      0, c, s, 0,
      0, -s, c, 0,
      0, 0, 0, 1,
    ]);
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
