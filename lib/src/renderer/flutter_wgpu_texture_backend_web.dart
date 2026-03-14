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

    if (_unsupportedReason != null) return;

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
