import 'package:flutter_test/flutter_test.dart';

import 'package:flutter_wgpu_texture_spinning_cube/main.dart';

void main() {
  testWidgets('renders spinning cube shell', (WidgetTester tester) async {
    await tester.pumpWidget(const SpinningCubeApp());
    expect(find.text('Spinning Cube'), findsOneWidget);
  });
}
