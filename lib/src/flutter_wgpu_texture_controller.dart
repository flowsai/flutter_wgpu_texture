import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/foundation.dart';
import 'package:flutter/scheduler.dart';
import 'dart:ui';

import 'platform_channel.dart';
import 'rust/api.dart' as rust_api;
import 'rust/rust_init.dart';

class FlutterWgpuTextureController extends ChangeNotifier {
  FlutterWgpuTextureController({
    this.autoStart = true,
    this.sceneType = 'cube',
  });

  final bool autoStart;
  final String sceneType;
  final String surfaceId = _makeSurfaceId();

  int? _textureId;
  BigInt? _handle;
  rust_api.BackendInfo? _backendInfo;
  Ticker? _ticker;
  bool _initialized = false;
  bool _animating = false;
  bool _frameInFlight = false;
  Size? _size;

  int? get textureId => _textureId;
  bool get isInitialized => _initialized;
  bool get isAnimating => _animating;
  rust_api.BackendInfo? get backendInfo => _backendInfo;
  Size? get size => _size;
  BigInt? get handle => _handle;

  Future<void> ensureInitialized(Size size, TickerProvider vsync) async {
    final targetWidth = math.max(1, size.width.round());
    final targetHeight = math.max(1, size.height.round());
    _size = Size(targetWidth.toDouble(), targetHeight.toDouble());
    if (_initialized) {
      await FlutterWgpuPlatformChannel.resizeSurface(
        surfaceId: surfaceId,
        handle: _handle!.toInt(),
        width: targetWidth,
        height: targetHeight,
      );
      notifyListeners();
      return;
    }

    await ensureRustInitialized();
    final renderer = rust_api.createRenderer(
      width: targetWidth,
      height: targetHeight,
      sceneType: sceneType,
    );
    _handle = renderer.handle;
    _backendInfo = renderer.backend;
    final surface = await FlutterWgpuPlatformChannel.createSurface(
      surfaceId: surfaceId,
      handle: renderer.handle.toInt(),
      width: targetWidth,
      height: targetHeight,
    );
    _textureId = surface.textureId;
    _ticker = vsync.createTicker(_onTick);
    _initialized = true;
    if (autoStart) {
      await startAnimation();
    }
    notifyListeners();
  }

  Future<void> disposeRenderer() async {
    _ticker?.dispose();
    _ticker = null;
    final handle = _handle;
    final initialized = _initialized;
    _initialized = false;
    _animating = false;
    _frameInFlight = false;
    _textureId = null;
    _handle = null;
    notifyListeners();

    if (initialized) {
      await FlutterWgpuPlatformChannel.disposeSurface(surfaceId);
    }
    if (handle != null) {
      rust_api.disposeRenderer(handle: handle);
    }
  }

  Future<void> startAnimation() async {
    final handle = _handle;
    if (handle == null) return;
    rust_api.startAnimation(handle: handle);
    _animating = true;
    _ticker?.start();
    notifyListeners();
  }

  Future<void> stopAnimation() async {
    final handle = _handle;
    if (handle == null) return;
    rust_api.stopAnimation(handle: handle);
    _animating = false;
    _ticker?.stop();
    notifyListeners();
  }

  Future<void> requestFrame() async {
    await _pumpFrame();
  }

  Future<void> setRotationEnabled(bool enabled) async {
    await setBoolParam('rotation_enabled', enabled);
  }

  Future<void> setRotationSpeed(double radiansPerSecond) async {
    await setFloatParam('rotation_speed', radiansPerSecond);
  }

  Future<void> setCubeColor(Color color) async {
    await setVec4Param('cube_color', _colorToVec4(color));
  }

  Future<void> setBackgroundColor(Color color) async {
    await setVec4Param('background_color', _colorToVec4(color));
  }

  Future<void> resetScene() async {
    await invokeRustCommand('reset_scene');
  }

  Future<rust_api.BackendInfo?> getBackendInfo() async => _backendInfo;

  Future<void> setBoolParam(String key, bool value) async {
    final handle = _handle;
    if (handle == null) return;
    rust_api.setBoolParam(handle: handle, key: key, value: value);
    await _pumpFrame();
  }

  Future<void> setFloatParam(String key, double value) async {
    final handle = _handle;
    if (handle == null) return;
    rust_api.setFloatParam(handle: handle, key: key, value: value);
    await _pumpFrame();
  }

  Future<void> setVec4Param(String key, List<double> value) async {
    final handle = _handle;
    if (handle == null) return;
    rust_api.setVec4Param(handle: handle, key: key, value: value);
    await _pumpFrame();
  }

  Future<void> invokeRustCommand(
    String command, {
    String payload = '{}',
  }) async {
    final handle = _handle;
    if (handle == null) return;
    rust_api.invokeCommand(handle: handle, command: command, payload: payload);
    await _pumpFrame();
  }

  void _onTick(Duration _) {
    if (!_frameInFlight) {
      unawaited(_pumpFrame());
    }
  }

  Future<void> _pumpFrame() async {
    final handle = _handle;
    if (handle == null || !_initialized || _frameInFlight) {
      return;
    }
    _frameInFlight = true;
    try {
      final rendered = await rust_api.requestFrame(handle: handle);
      if (rendered && _textureId != null) {
        await FlutterWgpuPlatformChannel.markFrameAvailable(surfaceId);
      }
    } catch (error) {
      // Surface attachment can lag slightly behind controller startup on desktop.
      // Treat this as a dropped frame instead of surfacing an unhandled exception.
      if (!_isTransientPresentTargetError(error)) {
        rethrow;
      }
    } finally {
      _frameInFlight = false;
    }
  }

  bool _isTransientPresentTargetError(Object error) {
    return error.toString().contains('no present target');
  }

  static List<double> _colorToVec4(Color color) {
    return <double>[
      color.r / 255.0,
      color.g / 255.0,
      color.b / 255.0,
      color.a / 255.0,
    ];
  }

  static String _makeSurfaceId() {
    final micros = DateTime.now().microsecondsSinceEpoch;
    return 'surface_$micros';
  }

  @override
  void dispose() {
    unawaited(disposeRenderer());
    super.dispose();
  }
}
