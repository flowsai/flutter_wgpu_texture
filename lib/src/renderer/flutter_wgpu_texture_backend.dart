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

  /// Measured viewport frame rate (frames per second), or null until enough
  /// frames have been rendered to estimate it.
  int? get fps;

  Future<void> ensureInitialized(Size size, TickerProvider vsync);

  /// Disposes the render [Ticker] without tearing down the renderer.
  ///
  /// The ticker is vended by the host [State]'s [TickerProvider], so it must be
  /// disposed when that State is disposed (e.g. the viewport widget unmounts
  /// when switching editor modes). The renderer/texture stay alive so a later
  /// [ensureInitialized] on remount can recreate the ticker and resume.
  void detachTicker();

  Future<void> dispose();
  Future<void> startAnimation();
  Future<void> stopAnimation();
  Future<void> requestFrame();
  Future<void> setBoolParam(String key, bool value);
  Future<void> setFloatParam(String key, double value);
  Future<void> setVec4Param(String key, List<double> value);
  Future<void> invokeCommand(String command, {String payload = '{}'});
}
