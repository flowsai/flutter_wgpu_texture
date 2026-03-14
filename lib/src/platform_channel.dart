import 'package:flutter/services.dart';

/// Returned by [FlutterWgpuPlatformChannel.createSurface] and
/// [FlutterWgpuPlatformChannel.resizeSurface].
class NativeSurfaceInfo {
  const NativeSurfaceInfo({
    required this.textureId,
    this.mtlTexturePtr,
    this.bytesPerRow,
  });

  /// The Flutter texture id registered with the texture registry.
  final int textureId;

  /// macOS only: raw `id<MTLTexture>` pointer returned by the Swift bridge.
  final int? mtlTexturePtr;

  /// macOS only: bytes per row of the Metal texture backing the pixel buffer.
  final int? bytesPerRow;

  factory NativeSurfaceInfo.fromMap(Map<Object?, Object?> map) {
    return NativeSurfaceInfo(
      textureId: map['textureId'] as int,
      mtlTexturePtr: map['mtlTexturePtr'] as int?,
      bytesPerRow: map['bytesPerRow'] as int?,
    );
  }
}

class FlutterWgpuPlatformChannel {
  FlutterWgpuPlatformChannel._();

  static const MethodChannel channel = MethodChannel('flutter_wgpu_texture');

  /// Create a native surface.
  ///
  /// macOS: pass [width] and [height] only — Swift allocates the Metal texture
  /// and returns [NativeSurfaceInfo.mtlTexturePtr] / [NativeSurfaceInfo.bytesPerRow].
  ///
  /// Windows: pass [dxgiHandle] (raw HANDLE from [createDxgiSurface]).
  ///
  /// Linux: pass all DMA-BUF fields from the [DmaBufExport] returned by
  /// [exportDmabuf].
  static Future<NativeSurfaceInfo> createSurface({
    required String surfaceId,
    required int width,
    required int height,
    // Windows
    int? dxgiHandle,
    // Linux
    int? fd,
    int? stride,
    int? offset,
    int? fourcc,
    int? modifierLow,
    int? modifierHigh,
  }) async {
    final args = <String, Object?>{
      'surfaceId': surfaceId,
      'width': width,
      'height': height,
      'dxgiHandle': ?dxgiHandle,
      'fd': ?fd,
      'stride': ?stride,
      'offset': ?offset,
      'fourcc': ?fourcc,
      'modifierLow': ?modifierLow,
      'modifierHigh': ?modifierHigh,
    };
    final result = await channel.invokeMethod<Object?>('createSurface', args);
    return NativeSurfaceInfo.fromMap(result as Map<Object?, Object?>);
  }

  /// Resize an existing surface.
  ///
  /// On macOS, returns a new [NativeSurfaceInfo] with an updated
  /// [NativeSurfaceInfo.mtlTexturePtr] / [NativeSurfaceInfo.bytesPerRow].
  /// On other platforms returns `null`.
  static Future<NativeSurfaceInfo?> resizeSurface({
    required String surfaceId,
    required int width,
    required int height,
    // Windows
    int? dxgiHandle,
    // Linux
    int? fd,
    int? stride,
    int? offset,
    int? fourcc,
    int? modifierLow,
    int? modifierHigh,
  }) async {
    final args = <String, Object?>{
      'surfaceId': surfaceId,
      'width': width,
      'height': height,
      'dxgiHandle': ?dxgiHandle,
      'fd': ?fd,
      'stride': ?stride,
      'offset': ?offset,
      'fourcc': ?fourcc,
      'modifierLow': ?modifierLow,
      'modifierHigh': ?modifierHigh,
    };
    final result = await channel.invokeMethod<Object?>('resizeSurface', args);
    if (result == null) return null;
    return NativeSurfaceInfo.fromMap(result as Map<Object?, Object?>);
  }

  static Future<void> disposeSurface(String surfaceId) {
    return channel.invokeMethod<void>('disposeSurface', {'surfaceId': surfaceId});
  }

  static Future<void> markFrameAvailable(String surfaceId) {
    return channel.invokeMethod<void>('markFrameAvailable', {'surfaceId': surfaceId});
  }
}
