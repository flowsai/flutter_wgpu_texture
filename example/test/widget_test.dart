import 'dart:ui';

import 'package:flutter/scheduler.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_wgpu_texture/flutter_wgpu_texture.dart';
import 'package:flutter_wgpu_texture/src/renderer/flutter_wgpu_texture_backend.dart';

import 'package:flutter_wgpu_texture_spinning_cube/main.dart';

void main() {
  testWidgets('renders spinning cube shell', (WidgetTester tester) async {
    final controller = FlutterWgpuTextureController.withBackend(
      backend: _FakeFlutterWgpuTextureBackend(),
    );

    await tester.pumpWidget(SpinningCubeApp(controller: controller));
    expect(find.text('Spinning Cube'), findsOneWidget);
  });
}

class _FakeFlutterWgpuTextureBackend implements FlutterWgpuTextureBackend {
  @override
  BackendInfo? get backendInfo => null;

  @override
  BigInt? get handle => null;

  @override
  bool get isAnimating => false;

  @override
  bool get isInitialized => true;

  @override
  Size? get size => const Size(512, 512);

  @override
  int? get textureId => null;

  @override
  String? get unsupportedReason => null;

  @override
  String? get viewType => null;

  @override
  Future<void> dispose() async {}

  @override
  Future<void> ensureInitialized(Size size, TickerProvider vsync) async {}

  @override
  Future<void> invokeCommand(String command, {String payload = '{}'}) async {}

  @override
  Future<void> requestFrame() async {}

  @override
  Future<void> setBoolParam(String key, bool value) async {}

  @override
  Future<void> setFloatParam(String key, double value) async {}

  @override
  Future<void> setVec4Param(String key, List<double> value) async {}

  @override
  Future<void> startAnimation() async {}

  @override
  Future<void> stopAnimation() async {}
}
