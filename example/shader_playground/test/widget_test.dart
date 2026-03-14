import 'package:flutter_test/flutter_test.dart';

import '../lib/main.dart';

void main() {
  testWidgets('renders shader playground shell', (tester) async {
    await tester.pumpWidget(const ShaderPlaygroundApp());

    expect(find.text('Shader Playground'), findsOneWidget);
    expect(find.text('Live Shader'), findsOneWidget);
    expect(find.text('Uniforms'), findsOneWidget);
  });
}
