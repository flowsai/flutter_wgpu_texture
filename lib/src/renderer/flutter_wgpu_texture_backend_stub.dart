import 'dart:ui';

import 'package:flutter/scheduler.dart';

import '../rust/api.dart' as rust_api;
import 'flutter_wgpu_texture_backend.dart';

FlutterWgpuTextureBackend createFlutterWgpuTextureBackend({
  required bool autoStart,
  required String sceneType,
  required String surfaceId,
}) {
  return _UnsupportedFlutterWgpuTextureBackend();
}

class _UnsupportedFlutterWgpuTextureBackend
    implements FlutterWgpuTextureBackend {
  @override
  rust_api.BackendInfo? get backendInfo => null;

  @override
  BigInt? get handle => null;

  @override
  bool get isAnimating => false;

  @override
  bool get isInitialized => false;

  @override
  Size? get size => null;

  @override
  int? get textureId => null;

  @override
  String? get unsupportedReason =>
      'flutter_wgpu_texture is unavailable here.';

  @override
  int? get fps => null;

  @override
  String? get viewType => null;

  @override
  Future<void> dispose() async {}

  @override
  Future<void> ensureInitialized(Size size, TickerProvider vsync) async {
    throw UnsupportedError(unsupportedReason!);
  }

  @override
  Future<void> invokeCommand(String command, {String payload = '{}'}) async {
    throw UnsupportedError(unsupportedReason!);
  }

  @override
  Future<void> requestFrame() async {
    throw UnsupportedError(unsupportedReason!);
  }

  @override
  Future<void> setBoolParam(String key, bool value) async {
    throw UnsupportedError(unsupportedReason!);
  }

  @override
  Future<void> setFloatParam(String key, double value) async {
    throw UnsupportedError(unsupportedReason!);
  }

  @override
  Future<void> setVec4Param(String key, List<double> value) async {
    throw UnsupportedError(unsupportedReason!);
  }

  @override
  Future<void> startAnimation() async {
    throw UnsupportedError(unsupportedReason!);
  }

  @override
  Future<void> stopAnimation() async {
    throw UnsupportedError(unsupportedReason!);
  }
}
