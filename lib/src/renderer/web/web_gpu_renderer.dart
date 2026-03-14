import 'dart:js_util' as js_util;
import 'dart:math' as math;
import 'dart:typed_data';
import 'dart:ui';

import 'package:web/web.dart' as web;

import 'web_renderer.dart';

/// WebGPU-based implementation of [WebRenderer].
class WebGpuRenderer implements WebRenderer {
  @override
  String get backendName => 'WebGPU';

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

  int _canvasWidth = 1;
  int _canvasHeight = 1;

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
    _canvasWidth = canvas.width;
    _canvasHeight = canvas.height;

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
    if (adapter == null) {
      throw UnsupportedError('No compatible WebGPU adapter was found.');
    }

    _device = await js_util.promiseToFuture<Object?>(
      js_util.callMethod<Object>(adapter, 'requestDevice', const <Object?>[]),
    );
    if (_device == null) {
      throw UnsupportedError('Unable to acquire a WebGPU device.');
    }

    _queue = js_util.getProperty<Object?>(_device!, 'queue');
    _context = js_util.callMethod<Object?>(
      canvas,
      'getContext',
      <Object?>['webgpu'],
    );
    if (_context == null) {
      throw UnsupportedError('canvas.getContext("webgpu") returned null.');
    }

    _presentationFormat = js_util.callMethod<String>(
      _gpu!,
      'getPreferredCanvasFormat',
      const <Object?>[],
    );
    _configureContext();
    _createPipelineResources();
    _recreateDepthTexture();
  }

  @override
  void drawFrame(
    double rotation,
    List<double> cubeColor,
    List<double> backgroundColor,
  ) {
    final currentTexture = js_util.callMethod<Object?>(
      _context!,
      'getCurrentTexture',
      const <Object?>[],
    );
    final currentView = js_util.callMethod<Object?>(
      currentTexture!,
      'createView',
      const <Object?>[],
    );

    final uniformData = Float32List(20);
    uniformData.setAll(0, _buildMvpMatrix(rotation));
    uniformData.setAll(16, cubeColor);
    js_util.callMethod<void>(_queue!, 'writeBuffer', <Object?>[
      _uniformBuffer,
      0,
      uniformData,
    ]);

    final commandEncoder = js_util.callMethod<Object?>(
      _device!,
      'createCommandEncoder',
      const <Object?>[],
    );
    final renderPass = js_util.callMethod<Object?>(
      commandEncoder!,
      'beginRenderPass',
      <Object?>[
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
      ],
    );

    js_util.callMethod<void>(renderPass!, 'setPipeline', <Object?>[_pipeline]);
    js_util.callMethod<void>(renderPass, 'setBindGroup', <Object?>[0, _bindGroup]);
    js_util.callMethod<void>(renderPass, 'setVertexBuffer', <Object?>[
      0,
      _vertexBuffer,
    ]);
    js_util.callMethod<void>(renderPass, 'setIndexBuffer', <Object?>[
      _indexBuffer,
      'uint16',
    ]);
    js_util.callMethod<void>(renderPass, 'drawIndexed', <Object?>[
      _cubeIndices.length,
      1,
      0,
      0,
      0,
    ]);
    js_util.callMethod<void>(renderPass, 'end', const <Object?>[]);

    final commandBuffer = js_util.callMethod<Object?>(
      commandEncoder,
      'finish',
      const <Object?>[],
    );
    js_util.callMethod<void>(_queue!, 'submit', <Object?>[
      <Object?>[commandBuffer],
    ]);
  }

  @override
  void resize(web.HTMLCanvasElement canvas) {
    _canvasWidth = canvas.width;
    _canvasHeight = canvas.height;
    _configureContext();
    _recreateDepthTexture();
  }

  @override
  void dispose() {
    if (_depthTexture != null) {
      js_util.callMethod<void>(_depthTexture!, 'destroy', const <Object?>[]);
      _depthTexture = null;
    }
    _depthTextureView = null;
    if (_uniformBuffer != null) {
      js_util.callMethod<void>(_uniformBuffer!, 'destroy', const <Object?>[]);
    }
    if (_vertexBuffer != null) {
      js_util.callMethod<void>(_vertexBuffer!, 'destroy', const <Object?>[]);
    }
    if (_indexBuffer != null) {
      js_util.callMethod<void>(_indexBuffer!, 'destroy', const <Object?>[]);
    }
    _gpu = null;
    _device = null;
    _queue = null;
    _context = null;
    _pipeline = null;
    _uniformBuffer = null;
    _vertexBuffer = null;
    _indexBuffer = null;
    _bindGroup = null;
    _presentationFormat = null;
  }

  void _configureContext() {
    js_util.callMethod<void>(_context!, 'configure', <Object?>[
      js_util.jsify(<String, Object?>{
        'device': _device,
        'format': _presentationFormat,
        'alphaMode': 'premultiplied',
      }),
    ]);
  }

  void _createPipelineResources() {
    final shaderModule = js_util.callMethod<Object?>(
      _device!,
      'createShaderModule',
      <Object?>[
        js_util.jsify(<String, Object?>{'code': _shaderSource}),
      ],
    );

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
    _uniformBuffer = js_util.callMethod<Object?>(
      _device!,
      'createBuffer',
      <Object?>[
        js_util.jsify(<String, Object?>{
          'label': 'cube uniforms',
          'size': 80,
          'usage': _gpuBufferUsage('UNIFORM') | _gpuBufferUsage('COPY_DST'),
        }),
      ],
    );

    _pipeline = js_util.callMethod<Object?>(
      _device!,
      'createRenderPipeline',
      <Object?>[
        js_util.jsify(<String, Object?>{
          'layout': 'auto',
          'vertex': <String, Object?>{
            'module': shaderModule,
            'entryPoint': 'vs_main',
            'buffers': <Object?>[
              <String, Object?>{
                'arrayStride': 12,
                'attributes': <Object?>[
                  <String, Object?>{
                    'shaderLocation': 0,
                    'offset': 0,
                    'format': 'float32x3',
                  },
                ],
              },
            ],
          },
          'fragment': <String, Object?>{
            'module': shaderModule,
            'entryPoint': 'fs_main',
            'targets': <Object?>[
              <String, Object?>{'format': _presentationFormat},
            ],
          },
          'primitive': <String, Object?>{
            'topology': 'triangle-list',
            'cullMode': 'back',
          },
          'depthStencil': <String, Object?>{
            'format': 'depth24plus',
            'depthWriteEnabled': true,
            'depthCompare': 'less',
          },
        }),
      ],
    );

    final bindGroupLayout = js_util.callMethod<Object?>(
      _pipeline!,
      'getBindGroupLayout',
      <Object?>[0],
    );
    _bindGroup = js_util.callMethod<Object?>(
      _device!,
      'createBindGroup',
      <Object?>[
        js_util.jsify(<String, Object?>{
          'layout': bindGroupLayout,
          'entries': <Object?>[
            <String, Object?>{
              'binding': 0,
              'resource': <String, Object?>{'buffer': _uniformBuffer},
            },
          ],
        }),
      ],
    );
  }

  Object? _createBuffer({
    required TypedData data,
    required int usage,
    required String label,
  }) {
    final buffer = js_util.callMethod<Object?>(
      _device!,
      'createBuffer',
      <Object?>[
        js_util.jsify(<String, Object?>{
          'label': label,
          'size': data.lengthInBytes,
          'usage': usage,
          'mappedAtCreation': true,
        }),
      ],
    );
    final mappedRange = js_util.callMethod<Object?>(
      buffer!,
      'getMappedRange',
      const <Object?>[],
    );
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

  void _recreateDepthTexture() {
    if (_device == null) return;
    if (_depthTexture != null) {
      js_util.callMethod<void>(_depthTexture!, 'destroy', const <Object?>[]);
    }
    _depthTexture = js_util.callMethod<Object?>(
      _device!,
      'createTexture',
      <Object?>[
        js_util.jsify(<String, Object?>{
          'size': <String, Object?>{
            'width': _canvasWidth,
            'height': _canvasHeight,
          },
          'format': 'depth24plus',
          'usage': _gpuTextureUsage('RENDER_ATTACHMENT'),
        }),
      ],
    );
    _depthTextureView = js_util.callMethod<Object?>(
      _depthTexture!,
      'createView',
      const <Object?>[],
    );
  }

  int _gpuBufferUsage(String key) {
    final gpuBufferUsage = js_util.getProperty<Object?>(
      js_util.globalThis,
      'GPUBufferUsage',
    )!;
    return js_util.getProperty<int>(gpuBufferUsage, key);
  }

  int _gpuTextureUsage(String key) {
    final gpuTextureUsage = js_util.getProperty<Object?>(
      js_util.globalThis,
      'GPUTextureUsage',
    )!;
    return js_util.getProperty<int>(gpuTextureUsage, key);
  }

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
