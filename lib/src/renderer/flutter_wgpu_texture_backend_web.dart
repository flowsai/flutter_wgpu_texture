import 'dart:async';
import 'dart:js_util' as js_util;
import 'dart:math' as math;
import 'dart:typed_data';
import 'dart:ui';
import 'dart:ui_web' as ui_web;

import 'package:flutter/scheduler.dart';
import 'package:web/web.dart' as web;

import '../rust/api.dart' as rust_api;
import 'flutter_wgpu_texture_backend.dart';

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
  bool _frameInFlight = false;

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
      await _recreateDepthTexture();
      await requestFrame();
      return;
    }

    if (sceneType != 'cube') {
      _unsupportedReason =
          'Web currently supports the "cube" scene only (requested: $sceneType).';
      return;
    }

    try {
      await _initializeWebGpu();
      _initialized = true;
      _backendInfo = const rust_api.BackendInfo(
        backend: 'WebGPU',
        deviceName: 'Browser GPU Adapter',
        driver: 'web',
      );
      await _recreateDepthTexture();
      await requestFrame();
      if (autoStart) {
        await startAnimation();
      }
    } catch (error) {
      _unsupportedReason = 'WebGPU initialization failed: $error';
    }
  }

  Future<void> _initializeWebGpu() async {
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
      _canvas,
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

  @override
  Future<void> dispose() async {
    await stopAnimation();
    _initialized = false;
    _backendInfo = null;
    _size = null;
    _device = null;
    _queue = null;
    _context = null;
    _pipeline = null;
    _uniformBuffer = null;
    _vertexBuffer = null;
    _indexBuffer = null;
    _bindGroup = null;
    _depthTextureView = null;
    if (_depthTexture != null) {
      js_util.callMethod<void>(_depthTexture!, 'destroy', const <Object?>[]);
      _depthTexture = null;
    }
  }

  @override
  Future<void> startAnimation() async {
    if (!_initialized || _animating) return;
    _animating = true;
    _animationTimer = Timer.periodic(const Duration(milliseconds: 16), (_) {
      if (!_rotationEnabled) {
        unawaited(requestFrame());
        return;
      }
      _rotation += _rotationSpeed / 60.0;
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
    if (!_initialized || _frameInFlight) return;
    _frameInFlight = true;
    try {
      _drawFrame();
    } finally {
      _frameInFlight = false;
    }
  }

  void _drawFrame() {
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
    uniformData.setAll(0, _buildModelViewProjectionMatrix(_rotation));
    uniformData.setAll(16, _cubeColor);
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
                'r': _backgroundColor[0],
                'g': _backgroundColor[1],
                'b': _backgroundColor[2],
                'a': _backgroundColor[3],
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

    if (_context != null) {
      _configureContext();
    }
  }

  Future<void> _recreateDepthTexture() async {
    if (!_initialized || _device == null || _size == null) return;
    if (_depthTexture != null) {
      js_util.callMethod<void>(_depthTexture!, 'destroy', const <Object?>[]);
    }
    _depthTexture = js_util.callMethod<Object?>(
      _device!,
      'createTexture',
      <Object?>[
        js_util.jsify(<String, Object?>{
          'size': <String, Object?>{
            'width': _canvas.width,
            'height': _canvas.height,
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

  Float32List _buildModelViewProjectionMatrix(double angle) {
    final aspect = _canvas.height == 0 ? 1.0 : _canvas.width / _canvas.height;
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
