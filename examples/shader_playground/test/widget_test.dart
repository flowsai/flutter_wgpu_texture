import 'package:flutter_test/flutter_test.dart';

import 'package:flutter_wgpu_texture_shader_playground/main.dart';

void main() {
  testWidgets('renders shader playground shell', (tester) async {
    await tester.pumpWidget(const ShaderPlaygroundApp());

    expect(find.text('Shader Playground'), findsOneWidget);
    expect(find.text('Live Shader'), findsOneWidget);
    expect(find.text('Uniforms'), findsOneWidget);
  });
}
