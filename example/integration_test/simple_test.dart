import 'package:integration_test/integration_test.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_wgpu_texture_example/main.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('renders demo shell', (tester) async {
    await tester.pumpWidget(const MyApp());
    expect(find.text('flutter_wgpu_texture'), findsOneWidget);
  });
}
