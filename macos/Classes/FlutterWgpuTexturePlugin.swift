import Cocoa
import CoreVideo
import FlutterMacOS
import Metal

// No @_silgen_name declarations — Rust is loaded as a separate dylib by
// flutter_rust_bridge.  All Rust operations go through the FRB Dart API and
// the results are forwarded to Swift via the method channel.

private final class FlutterWgpuTextureSurface: NSObject, FlutterTexture {
  var pixelBuffer: CVPixelBuffer

  init(width: Int, height: Int) {
    self.pixelBuffer = FlutterWgpuTextureSurface.makePixelBuffer(width: width, height: height)
    super.init()
  }

  func replace(width: Int, height: Int) {
    pixelBuffer = FlutterWgpuTextureSurface.makePixelBuffer(width: width, height: height)
  }

  func copyPixelBuffer() -> Unmanaged<CVPixelBuffer>? {
    Unmanaged.passRetained(pixelBuffer)
  }

  private static func makePixelBuffer(width: Int, height: Int) -> CVPixelBuffer {
    var buffer: CVPixelBuffer?
    let attributes: [CFString: Any] = [
      kCVPixelBufferIOSurfacePropertiesKey: [:],
      kCVPixelBufferMetalCompatibilityKey: true,
      kCVPixelBufferCGImageCompatibilityKey: true,
      kCVPixelBufferCGBitmapContextCompatibilityKey: true,
    ]
    let status = CVPixelBufferCreate(
      kCFAllocatorDefault,
      width,
      height,
      kCVPixelFormatType_32BGRA,
      attributes as CFDictionary,
      &buffer
    )
    precondition(status == kCVReturnSuccess && buffer != nil, "CVPixelBufferCreate failed: \(status)")
    return buffer!
  }
}

private final class SurfaceState {
  let surfaceId: String
  let texture: FlutterWgpuTextureSurface
  var textureId: Int64?
  var width: Int
  var height: Int
  var textureCache: CVMetalTextureCache?
  var presentTexture: CVMetalTexture?

  init(surfaceId: String, width: Int, height: Int) {
    self.surfaceId = surfaceId
    self.texture = FlutterWgpuTextureSurface(width: width, height: height)
    self.width = width
    self.height = height
  }
}

public final class FlutterWgpuTexturePlugin: NSObject, FlutterPlugin {
  private let textureRegistry: FlutterTextureRegistry
  private let stateLock = NSLock()
  private var surfaces: [String: SurfaceState] = [:]

  init(textureRegistry: FlutterTextureRegistry) {
    self.textureRegistry = textureRegistry
    super.init()
  }

  public static func register(with registrar: FlutterPluginRegistrar) {
    let channel = FlutterMethodChannel(
      name: "flutter_wgpu_texture",
      binaryMessenger: registrar.messenger
    )
    let instance = FlutterWgpuTexturePlugin(textureRegistry: registrar.textures)
    registrar.addMethodCallDelegate(instance, channel: channel)
  }

  public func handle(_ call: FlutterMethodCall, result: @escaping FlutterResult) {
    let args = call.arguments as? [String: Any] ?? [:]
    switch call.method {
    case "createSurface":
      createSurface(args: args, result: result)
    case "resizeSurface":
      resizeSurface(args: args, result: result)
    case "disposeSurface":
      disposeSurface(args: args, result: result)
    case "markFrameAvailable":
      markFrameAvailable(args: args, result: result)
    default:
      result(FlutterMethodNotImplemented)
    }
  }

  private func createSurface(args: [String: Any], result: @escaping FlutterResult) {
    let surfaceId = stringValue(args["surfaceId"], fallback: "default")
    let width = clampedInt(args["width"], fallback: 512)
    let height = clampedInt(args["height"], fallback: 512)

    let entry: SurfaceState = withLock {
      if let existing = surfaces[surfaceId] {
        return existing
      }
      let created = SurfaceState(surfaceId: surfaceId, width: width, height: height)
      surfaces[surfaceId] = created
      return created
    }

    if entry.textureId == nil {
      entry.textureId = textureRegistry.register(entry.texture)
    }

    do {
      let (mtlTexturePtr, bytesPerRow) = try makeMetalTexture(entry: entry, width: width, height: height)
      result([
        "textureId": entry.textureId!,
        "mtlTexturePtr": mtlTexturePtr,
        "bytesPerRow": bytesPerRow,
        "width": width,
        "height": height,
      ])
    } catch {
      result(FlutterError(code: "create_surface_failed", message: error.localizedDescription, details: nil))
    }
  }

  private func resizeSurface(args: [String: Any], result: @escaping FlutterResult) {
    let surfaceId = stringValue(args["surfaceId"], fallback: "default")
    let width = clampedInt(args["width"], fallback: 512)
    let height = clampedInt(args["height"], fallback: 512)

    guard let entry = withLock({ surfaces[surfaceId] }) else {
      result(nil)
      return
    }

    do {
      let (mtlTexturePtr, bytesPerRow) = try makeMetalTexture(entry: entry, width: width, height: height)
      result([
        "textureId": entry.textureId ?? -1,
        "mtlTexturePtr": mtlTexturePtr,
        "bytesPerRow": bytesPerRow,
        "width": width,
        "height": height,
      ])
    } catch {
      result(FlutterError(code: "resize_failed", message: error.localizedDescription, details: nil))
    }
  }

  private func disposeSurface(args: [String: Any], result: @escaping FlutterResult) {
    let surfaceId = stringValue(args["surfaceId"], fallback: "default")
    let removed: SurfaceState? = withLock { surfaces.removeValue(forKey: surfaceId) }
    if let removed, let textureId = removed.textureId {
      textureRegistry.unregisterTexture(textureId)
    }
    result(nil)
  }

  private func markFrameAvailable(args: [String: Any], result: @escaping FlutterResult) {
    let surfaceId = stringValue(args["surfaceId"], fallback: "default")
    if let entry = withLock({ surfaces[surfaceId] }), let textureId = entry.textureId {
      textureRegistry.textureFrameAvailable(textureId)
    }
    result(nil)
  }

  /// Allocate (or reallocate) the CVPixelBuffer + Metal texture for `entry`.
  ///
  /// Uses `MTLCreateSystemDefaultDevice()` — wgpu picks the same device on
  /// single-GPU Macs, which is the common case.  Returns the raw
  /// `id<MTLTexture>` pointer and bytes-per-row so Dart can pass them to
  /// `attachMetalTexture` via FRB.
  private func makeMetalTexture(entry: SurfaceState, width: Int, height: Int) throws -> (Int, Int) {
    guard let mtlDevice = MTLCreateSystemDefaultDevice() else {
      throw NSError(domain: "flutter_wgpu_texture", code: 1, userInfo: [
        NSLocalizedDescriptionKey: "MTLCreateSystemDefaultDevice returned nil"
      ])
    }
    var textureCache: CVMetalTextureCache?
    let cacheStatus = CVMetalTextureCacheCreate(kCFAllocatorDefault, nil, mtlDevice, nil, &textureCache)
    guard cacheStatus == kCVReturnSuccess, let resolvedCache = textureCache else {
      throw NSError(domain: "flutter_wgpu_texture", code: 2, userInfo: [
        NSLocalizedDescriptionKey: "CVMetalTextureCacheCreate failed: \(cacheStatus)"
      ])
    }

    entry.texture.replace(width: width, height: height)
    entry.width = width
    entry.height = height
    let bytesPerRow = CVPixelBufferGetBytesPerRow(entry.texture.pixelBuffer)

    var cvTexture: CVMetalTexture?
    let attrs: [CFString: Any] = [
      kCVMetalTextureUsage: NSNumber(
        value: MTLTextureUsage.shaderRead.rawValue
          | MTLTextureUsage.shaderWrite.rawValue
          | MTLTextureUsage.renderTarget.rawValue
      )
    ]
    let textureStatus = CVMetalTextureCacheCreateTextureFromImage(
      kCFAllocatorDefault,
      resolvedCache,
      entry.texture.pixelBuffer,
      attrs as CFDictionary,
      .bgra8Unorm,
      width,
      height,
      0,
      &cvTexture
    )
    guard textureStatus == kCVReturnSuccess, let resolvedCvTexture = cvTexture else {
      throw NSError(domain: "flutter_wgpu_texture", code: 3, userInfo: [
        NSLocalizedDescriptionKey: "CVMetalTextureCacheCreateTextureFromImage failed: \(textureStatus)"
      ])
    }
    guard let mtlTexture = CVMetalTextureGetTexture(resolvedCvTexture) else {
      throw NSError(domain: "flutter_wgpu_texture", code: 4, userInfo: [
        NSLocalizedDescriptionKey: "CVMetalTextureGetTexture returned nil"
      ])
    }

    // Retain the texture so the pointer stays valid until Rust releases it.
    let texturePtr = Int(bitPattern: Unmanaged.passRetained(mtlTexture as AnyObject).toOpaque())

    entry.textureCache = resolvedCache
    entry.presentTexture = resolvedCvTexture
    return (texturePtr, bytesPerRow)
  }

  private func withLock<T>(_ body: () -> T) -> T {
    stateLock.lock()
    defer { stateLock.unlock() }
    return body()
  }

  private func stringValue(_ value: Any?, fallback: String) -> String {
    if let string = value as? String, !string.isEmpty {
      return string
    }
    if let number = value as? NSNumber {
      return number.stringValue
    }
    return fallback
  }

  private func clampedInt(_ value: Any?, fallback: Int) -> Int {
    let resolved: Int
    if let number = value as? NSNumber {
      resolved = number.intValue
    } else if let string = value as? String {
      resolved = Int(string) ?? fallback
    } else {
      resolved = fallback
    }
    return max(1, min(16384, resolved))
  }
}
