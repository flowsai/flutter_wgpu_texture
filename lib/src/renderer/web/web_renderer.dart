import 'dart:ui';

import 'package:web/web.dart' as web;

/// Common interface for web rendering backends (WebGPU and WebGL2).
abstract class WebRenderer {
  /// Human-readable name of the active backend, e.g. 'WebGPU' or 'WebGL2'.
  String get backendName;

  /// Initialize the renderer with the given canvas, scene type, and size.
  /// Throws if initialization fails.
  Future<void> init(web.HTMLCanvasElement canvas, String sceneType, Size size);

  /// Render one frame.
  void drawFrame(
    double rotation,
    List<double> cubeColor,
    List<double> backgroundColor,
  );

  /// Update GPU resources after a canvas resize.
  void resize(web.HTMLCanvasElement canvas);

  /// Release all GPU resources.
  void dispose();
}
