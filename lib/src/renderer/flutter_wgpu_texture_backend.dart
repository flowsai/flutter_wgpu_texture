import 'dart:ui';

import 'package:flutter/scheduler.dart';

import '../rust/api.dart' as rust_api;

abstract class FlutterWgpuTextureBackend {
  int? get textureId;
  String? get viewType;
  bool get isInitialized;
  bool get isAnimating;
  rust_api.BackendInfo? get backendInfo;
  Size? get size;
  BigInt? get handle;
  String? get unsupportedReason;

  Future<void> ensureInitialized(Size size, TickerProvider vsync);
  Future<void> dispose();
  Future<void> startAnimation();
  Future<void> stopAnimation();
  Future<void> requestFrame();
  Future<void> setBoolParam(String key, bool value);
  Future<void> setFloatParam(String key, double value);
  Future<void> setVec4Param(String key, List<double> value);
  Future<void> invokeCommand(String command, {String payload = '{}'});
}
