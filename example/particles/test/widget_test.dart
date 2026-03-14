import 'package:flutter_test/flutter_test.dart';

import 'package:flutter_wgpu_texture_particles/main.dart';

void main() {
  testWidgets('renders particles shell', (tester) async {
    await tester.pumpWidget(const ParticlesApp());

    expect(find.text('Particles'), findsOneWidget);
    expect(find.text('Live Scene'), findsOneWidget);
    expect(find.text('Controls'), findsOneWidget);
  });
}
