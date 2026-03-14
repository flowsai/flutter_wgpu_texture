import 'dart:js_interop';
import 'dart:math' as math;
import 'dart:typed_data';
import 'dart:ui';

import 'package:web/web.dart' as web;

import 'web_renderer.dart';

class WebGlRenderer implements WebRenderer {
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

  web.WebGL2RenderingContext? _gl;
  web.WebGLProgram? _program;
  web.WebGLVertexArrayObject? _vao;
  web.WebGLBuffer? _vertexBuffer;
  web.WebGLBuffer? _indexBuffer;
  web.WebGLUniformLocation? _mvpUniformLocation;
  web.WebGLUniformLocation? _colorUniformLocation;
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

    final ctx = canvas.getContext('webgl2');
    if (ctx == null) {
      throw UnsupportedError('canvas.getContext("webgl2") returned null.');
    }
    _gl = ctx as web.WebGL2RenderingContext;

    _program = _createProgram(_vertShaderSource, _fragShaderSource);
    _setupGeometry();

    _mvpUniformLocation = _gl!.getUniformLocation(_program!, 'u_mvp');
    _colorUniformLocation = _gl!.getUniformLocation(_program!, 'u_color');
    if (_mvpUniformLocation == null || _colorUniformLocation == null) {
      throw StateError('WebGL2: could not find uniforms u_mvp or u_color.');
    }

    _gl!.enable(web.WebGLRenderingContext.DEPTH_TEST);
    _gl!.depthFunc(web.WebGLRenderingContext.LESS);
  }

  web.WebGLProgram _createProgram(String vertSrc, String fragSrc) {
    final gl = _gl!;
    final vert = _compileShader(web.WebGLRenderingContext.VERTEX_SHADER, vertSrc);
    final frag = _compileShader(web.WebGLRenderingContext.FRAGMENT_SHADER, fragSrc);

    final program = gl.createProgram()!;
    gl.attachShader(program, vert);
    gl.attachShader(program, frag);
    gl.linkProgram(program);

    final linked =
        (gl.getProgramParameter(program, web.WebGLRenderingContext.LINK_STATUS)?.dartify() as bool?) ?? false;
    if (!linked) {
      final log = gl.getProgramInfoLog(program) ?? '';
      gl.deleteShader(vert);
      gl.deleteShader(frag);
      gl.deleteProgram(program);
      throw StateError('WebGL2 program link failed: $log');
    }

    gl.deleteShader(vert);
    gl.deleteShader(frag);
    return program;
  }

  web.WebGLShader _compileShader(int type, String source) {
    final gl = _gl!;
    final shader = gl.createShader(type)!;
    gl.shaderSource(shader, source);
    gl.compileShader(shader);

    final compiled =
        (gl.getShaderParameter(shader, web.WebGLRenderingContext.COMPILE_STATUS)?.dartify() as bool?) ?? false;
    if (!compiled) {
      final log = gl.getShaderInfoLog(shader) ?? '';
      gl.deleteShader(shader);
      throw StateError('WebGL2 shader compile failed: $log');
    }
    return shader;
  }

  void _setupGeometry() {
    final gl = _gl!;

    _vao = gl.createVertexArray();
    gl.bindVertexArray(_vao);

    _vertexBuffer = gl.createBuffer();
    gl.bindBuffer(web.WebGLRenderingContext.ARRAY_BUFFER, _vertexBuffer);
    gl.bufferData(web.WebGLRenderingContext.ARRAY_BUFFER, _cubeVertices.toJS, web.WebGLRenderingContext.STATIC_DRAW);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 3, web.WebGLRenderingContext.FLOAT, false, 12, 0);

    _indexBuffer = gl.createBuffer();
    gl.bindBuffer(web.WebGLRenderingContext.ELEMENT_ARRAY_BUFFER, _indexBuffer);
    gl.bufferData(
        web.WebGLRenderingContext.ELEMENT_ARRAY_BUFFER, _cubeIndices.toJS, web.WebGLRenderingContext.STATIC_DRAW);

    gl.bindVertexArray(null);
  }

  @override
  void drawFrame(
    double rotation,
    List<double> cubeColor,
    List<double> backgroundColor,
  ) {
    final gl = _gl!;

    gl.viewport(0, 0, _canvasWidth, _canvasHeight);
    gl.clearColor(
      backgroundColor[0],
      backgroundColor[1],
      backgroundColor[2],
      backgroundColor[3],
    );
    gl.clear(web.WebGLRenderingContext.COLOR_BUFFER_BIT | web.WebGLRenderingContext.DEPTH_BUFFER_BIT);

    gl.useProgram(_program);
    gl.bindVertexArray(_vao);

    gl.uniformMatrix4fv(
        _mvpUniformLocation, false, _buildMvpMatrix(rotation).toJS);
    gl.uniform4fv(
        _colorUniformLocation, Float32List.fromList(cubeColor).toJS);

    gl.drawElements(
        web.WebGLRenderingContext.TRIANGLES, _cubeIndices.length, web.WebGLRenderingContext.UNSIGNED_SHORT, 0);

    gl.bindVertexArray(null);
  }

  @override
  void resize(web.HTMLCanvasElement canvas) {
    _canvasWidth = canvas.width;
    _canvasHeight = canvas.height;
  }

  @override
  void dispose() {
    final gl = _gl;
    if (gl == null) return;
    if (_program != null) gl.deleteProgram(_program);
    if (_vertexBuffer != null) gl.deleteBuffer(_vertexBuffer);
    if (_indexBuffer != null) gl.deleteBuffer(_indexBuffer);
    if (_vao != null) gl.deleteVertexArray(_vao);
    _gl = null;
    _program = null;
    _vao = null;
    _vertexBuffer = null;
    _indexBuffer = null;
    _mvpUniformLocation = null;
    _colorUniformLocation = null;
  }

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

  Float32List _perspectiveMatrix(
      double fovY, double aspect, double near, double far) {
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
    return Float32List.fromList(
        <double>[c, 0, -s, 0, 0, 1, 0, 0, s, 0, c, 0, 0, 0, 0, 1]);
  }

  Float32List _rotationXMatrix(double angle) {
    final c = math.cos(angle);
    final s = math.sin(angle);
    return Float32List.fromList(
        <double>[1, 0, 0, 0, 0, c, s, 0, 0, -s, c, 0, 0, 0, 0, 1]);
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
