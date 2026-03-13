import 'package:flutter/services.dart';

class NativeSurfaceInfo {
  const NativeSurfaceInfo({
    required this.textureId,
    required this.backend,
    required this.width,
    required this.height,
  });

  final int textureId;
  final String backend;
  final int width;
  final int height;

  factory NativeSurfaceInfo.fromMap(Map<Object?, Object?> map) {
    return NativeSurfaceInfo(
      textureId: map['textureId'] as int,
      backend: map['backend'] as String,
      width: map['width'] as int,
      height: map['height'] as int,
    );
  }
}

class FlutterWgpuPlatformChannel {
  FlutterWgpuPlatformChannel._();

  static const MethodChannel channel = MethodChannel('flutter_wgpu_texture');

  static Future<NativeSurfaceInfo> createSurface({
    required String surfaceId,
    required int handle,
    required int width,
    required int height,
  }) async {
    final result = await channel.invokeMethod<Object?>('createSurface', {
      'surfaceId': surfaceId,
      'handle': handle.toString(),
      'width': width,
      'height': height,
    });
    return NativeSurfaceInfo.fromMap((result as Map<Object?, Object?>));
  }

  static Future<void> resizeSurface({
    required String surfaceId,
    required int handle,
    required int width,
    required int height,
  }) async {
    await channel.invokeMethod<void>('resizeSurface', {
      'surfaceId': surfaceId,
      'handle': handle.toString(),
      'width': width,
      'height': height,
    });
  }

  static Future<void> disposeSurface(String surfaceId) {
    return channel.invokeMethod<void>('disposeSurface', {'surfaceId': surfaceId});
  }

  static Future<void> markFrameAvailable(String surfaceId) {
    return channel.invokeMethod<void>('markFrameAvailable', {'surfaceId': surfaceId});
  }
}
