import 'package:flutter_test/flutter_test.dart';

import '../lib/main.dart';

void main() {
  testWidgets('renders custom scene shell', (tester) async {
    await tester.pumpWidget(const CustomSceneApp());

    expect(find.text('Custom Scene'), findsOneWidget);
  });
}
