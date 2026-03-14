import 'dart:async';
import 'dart:io';
import 'dart:math' as math;
import 'dart:ui';

import 'package:flutter/scheduler.dart';

import '../platform_channel.dart';
import '../rust/api.dart' as rust_api;
import '../rust/rust_init.dart';
import 'flutter_wgpu_texture_backend.dart';

FlutterWgpuTextureBackend createFlutterWgpuTextureBackend({
  required bool autoStart,
  required String sceneType,
  required String surfaceId,
}) {
  return _DesktopFlutterWgpuTextureBackend(
    autoStart: autoStart,
    sceneType: sceneType,
    surfaceId: surfaceId,
  );
}

class _DesktopFlutterWgpuTextureBackend
    implements FlutterWgpuTextureBackend {
  _DesktopFlutterWgpuTextureBackend({
    required this.autoStart,
    required this.sceneType,
    required this.surfaceId,
  });

  final bool autoStart;
  final String sceneType;
  final String surfaceId;

  int? _textureId;
  BigInt? _handle;
  rust_api.BackendInfo? _backendInfo;
  Ticker? _ticker;
  bool _initialized = false;
  bool _animating = false;
  bool _frameInFlight = false;
  Size? _size;

  @override
  rust_api.BackendInfo? get backendInfo => _backendInfo;

  @override
  BigInt? get handle => _handle;

  @override
  bool get isAnimating => _animating;

  @override
  bool get isInitialized => _initialized;

  @override
  Size? get size => _size;

  @override
  int? get textureId => _textureId;

  @override
  String? get unsupportedReason => null;

  @override
  String? get viewType => null;

  @override
  Future<void> ensureInitialized(Size size, TickerProvider vsync) async {
    final targetWidth = math.max(1, size.width.round());
    final targetHeight = math.max(1, size.height.round());
    _size = Size(targetWidth.toDouble(), targetHeight.toDouble());

    if (_initialized) {
      await _resizePlatformSurface(targetWidth, targetHeight);
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

    final surface = await _createPlatformSurface(targetWidth, targetHeight);
    _textureId = surface.textureId;
    _ticker = vsync.createTicker(_onTick);
    _initialized = true;
    if (autoStart) {
      await startAnimation();
    }
  }

  @override
  Future<void> dispose() async {
    _ticker?.dispose();
    _ticker = null;
    final currentHandle = _handle;
    final initialized = _initialized;
    _initialized = false;
    _animating = false;
    _frameInFlight = false;
    _textureId = null;
    _handle = null;
    _backendInfo = null;
    _size = null;

    if (initialized) {
      await FlutterWgpuPlatformChannel.disposeSurface(surfaceId);
    }
    if (currentHandle != null) {
      rust_api.disposeRenderer(handle: currentHandle);
    }
  }

  @override
  Future<void> startAnimation() async {
    final currentHandle = _handle;
    if (currentHandle == null) return;
    rust_api.startAnimation(handle: currentHandle);
    _animating = true;
    _ticker?.start();
  }

  @override
  Future<void> stopAnimation() async {
    final currentHandle = _handle;
    if (currentHandle == null) return;
    rust_api.stopAnimation(handle: currentHandle);
    _animating = false;
    _ticker?.stop();
  }

  @override
  Future<void> requestFrame() async {
    await _pumpFrame();
  }

  @override
  Future<void> setBoolParam(String key, bool value) async {
    final currentHandle = _handle;
    if (currentHandle == null) return;
    rust_api.setBoolParam(handle: currentHandle, key: key, value: value);
    await _pumpFrame();
  }

  @override
  Future<void> setFloatParam(String key, double value) async {
    final currentHandle = _handle;
    if (currentHandle == null) return;
    rust_api.setFloatParam(handle: currentHandle, key: key, value: value);
    await _pumpFrame();
  }

  @override
  Future<void> setVec4Param(String key, List<double> value) async {
    final currentHandle = _handle;
    if (currentHandle == null) return;
    rust_api.setVec4Param(handle: currentHandle, key: key, value: value);
    await _pumpFrame();
  }

  @override
  Future<void> invokeCommand(String command, {String payload = '{}'}) async {
    final currentHandle = _handle;
    if (currentHandle == null) return;
    rust_api.invokeCommand(
      handle: currentHandle,
      command: command,
      payload: payload,
    );
    await _pumpFrame();
  }

  Future<NativeSurfaceInfo> _createPlatformSurface(int width, int height) async {
    final currentHandle = _handle!;

    if (Platform.isMacOS || Platform.isIOS) {
      final surface = await FlutterWgpuPlatformChannel.createSurface(
        surfaceId: surfaceId,
        width: width,
        height: height,
      );
      rust_api.attachMetalTexture(
        handle: currentHandle,
        mtlTexturePtr: BigInt.from(surface.mtlTexturePtr!),
        width: width,
        height: height,
        bytesPerRow: width * 4,
      );
      return surface;
    }

    if (Platform.isWindows) {
      final dxgiHandle = rust_api.createDxgiSurface(
        handle: currentHandle,
        width: width,
        height: height,
      );
      return FlutterWgpuPlatformChannel.createSurface(
        surfaceId: surfaceId,
        width: width,
        height: height,
        dxgiHandle: dxgiHandle.toInt(),
      );
    }

    if (Platform.isLinux) {
      rust_api.ensureLinuxPresent(
        handle: currentHandle,
        width: width,
        height: height,
      );
      final dmabuf = rust_api.exportDmabuf(handle: currentHandle);
      if (dmabuf == null) {
        throw Exception('DMA-BUF export returned null');
      }
      return FlutterWgpuPlatformChannel.createSurface(
        surfaceId: surfaceId,
        width: width,
        height: height,
        fd: dmabuf.fd,
        stride: dmabuf.stride,
        offset: dmabuf.offset,
        fourcc: dmabuf.fourcc,
        modifierLow: dmabuf.modifierLow,
        modifierHigh: dmabuf.modifierHigh,
      );
    }

    throw UnsupportedError(
      'flutter_wgpu_texture: unsupported platform ${Platform.operatingSystem}',
    );
  }

  Future<void> _resizePlatformSurface(int width, int height) async {
    final currentHandle = _handle!;

    if (Platform.isMacOS || Platform.isIOS) {
      rust_api.resizeRenderer(handle: currentHandle, width: width, height: height);
      final surface = await FlutterWgpuPlatformChannel.resizeSurface(
        surfaceId: surfaceId,
        width: width,
        height: height,
      );
      if (surface != null && surface.mtlTexturePtr != null) {
        rust_api.attachMetalTexture(
          handle: currentHandle,
          mtlTexturePtr: BigInt.from(surface.mtlTexturePtr!),
          width: width,
          height: height,
          bytesPerRow: width * 4,
        );
      }
      return;
    }

    if (Platform.isWindows) {
      final dxgiHandle = rust_api.createDxgiSurface(
        handle: currentHandle,
        width: width,
        height: height,
      );
      await FlutterWgpuPlatformChannel.resizeSurface(
        surfaceId: surfaceId,
        width: width,
        height: height,
        dxgiHandle: dxgiHandle.toInt(),
      );
      return;
    }

    if (Platform.isLinux) {
      rust_api.ensureLinuxPresent(
        handle: currentHandle,
        width: width,
        height: height,
      );
      final dmabuf = rust_api.exportDmabuf(handle: currentHandle);
      if (dmabuf == null) return;
      await FlutterWgpuPlatformChannel.resizeSurface(
        surfaceId: surfaceId,
        width: width,
        height: height,
        fd: dmabuf.fd,
        stride: dmabuf.stride,
        offset: dmabuf.offset,
        fourcc: dmabuf.fourcc,
        modifierLow: dmabuf.modifierLow,
        modifierHigh: dmabuf.modifierHigh,
      );
    }
  }

  void _onTick(Duration _) {
    if (!_frameInFlight) {
      unawaited(_pumpFrame());
    }
  }

  Future<void> _pumpFrame() async {
    final currentHandle = _handle;
    if (currentHandle == null || !_initialized || _frameInFlight) {
      return;
    }
    _frameInFlight = true;
    try {
      final rendered = await rust_api.requestFrame(handle: currentHandle);
      if (rendered && _textureId != null) {
        await FlutterWgpuPlatformChannel.markFrameAvailable(surfaceId);
      }
    } catch (error) {
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
}
