import 'dart:async';
import 'dart:ui';

import 'package:flutter/foundation.dart';
import 'package:flutter/scheduler.dart';

import 'renderer/flutter_wgpu_texture_backend.dart';
import 'renderer/flutter_wgpu_texture_backend_stub.dart'
    if (dart.library.js_interop) 'renderer/flutter_wgpu_texture_backend_web.dart'
    if (dart.library.io) 'renderer/flutter_wgpu_texture_backend_desktop.dart';
import 'rust/api.dart' as rust_api;

class FlutterWgpuTextureController extends ChangeNotifier {
  FlutterWgpuTextureController({
    this.autoStart = true,
    this.sceneType = 'cube',
  }) : _backend = createFlutterWgpuTextureBackend(
         autoStart: autoStart,
         sceneType: sceneType,
         surfaceId: _makeSurfaceId(),
       );

  @visibleForTesting
  FlutterWgpuTextureController.withBackend({
    required FlutterWgpuTextureBackend backend,
    this.autoStart = true,
    this.sceneType = 'cube',
  }) : _backend = backend;

  final bool autoStart;
  final String sceneType;
  final FlutterWgpuTextureBackend _backend;

  int? get textureId => _backend.textureId;
  String? get viewType => _backend.viewType;
  bool get isInitialized => _backend.isInitialized;
  bool get isAnimating => _backend.isAnimating;
  rust_api.BackendInfo? get backendInfo => _backend.backendInfo;
  Size? get size => _backend.size;
  BigInt? get handle => _backend.handle;
  String? get unsupportedReason => _backend.unsupportedReason;

  /// Measured viewport FPS (null until enough frames have rendered).
  int? get fps => _backend.fps;

  Future<void> ensureInitialized(Size size, TickerProvider vsync) async {
    await _backend.ensureInitialized(size, vsync);
    notifyListeners();
  }

  Future<void> disposeRenderer() async {
    await _backend.dispose();
    notifyListeners();
  }

  Future<void> startAnimation() async {
    await _backend.startAnimation();
    notifyListeners();
  }

  Future<void> stopAnimation() async {
    await _backend.stopAnimation();
    notifyListeners();
  }

  Future<void> requestFrame() async {
    await _backend.requestFrame();
    notifyListeners();
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

  Future<rust_api.BackendInfo?> getBackendInfo() async => _backend.backendInfo;

  Future<void> setBoolParam(String key, bool value) async {
    await _backend.setBoolParam(key, value);
    notifyListeners();
  }

  Future<void> setFloatParam(String key, double value) async {
    await _backend.setFloatParam(key, value);
    notifyListeners();
  }

  Future<void> setVec4Param(String key, List<double> value) async {
    await _backend.setVec4Param(key, value);
    notifyListeners();
  }

  Future<void> invokeRustCommand(
    String command, {
    String payload = '{}',
  }) async {
    await _backend.invokeCommand(command, payload: payload);
    notifyListeners();
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
