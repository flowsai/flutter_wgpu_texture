import 'package:flutter/material.dart';
import 'package:flutter_wgpu_texture/flutter_wgpu_texture.dart';

void main() => runApp(const MyApp());

class MyApp extends StatefulWidget {
  const MyApp({super.key});

  @override
  State<MyApp> createState() => _MyAppState();
}

class _MyAppState extends State<MyApp> {
  late final FlutterWgpuTextureController controller;
  Color cubeColor = const Color(0xFFFFD400);
  Color backgroundColor = const Color(0xFF1B5CFF);

  @override
  void initState() {
    super.initState();
    controller = FlutterWgpuTextureController();
  }

  @override
  void dispose() {
    controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      debugShowCheckedModeBanner: false,
      home: Scaffold(
        backgroundColor: const Color(0xFFF3F4F6),
        body: SafeArea(
          child: Center(
            child: ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: 980),
              child: Padding(
                padding: const EdgeInsets.all(24),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    const Text(
                      'flutter_wgpu_texture',
                      style: TextStyle(fontSize: 32, fontWeight: FontWeight.w700),
                    ),
                    const SizedBox(height: 8),
                    const Text(
                      'Rotating yellow cube on a blue background rendered in Rust with wgpu.',
                      style: TextStyle(fontSize: 16, color: Colors.black54),
                    ),
                    const SizedBox(height: 24),
                    Expanded(
                      child: DecoratedBox(
                        decoration: BoxDecoration(
                          color: Colors.white,
                          borderRadius: BorderRadius.circular(24),
                          boxShadow: const [
                            BoxShadow(
                              color: Color(0x1A000000),
                              blurRadius: 32,
                              offset: Offset(0, 20),
                            ),
                          ],
                        ),
                        child: Padding(
                          padding: const EdgeInsets.all(20),
                          child: FlutterWgpuTexture(
                            controller: controller,
                            placeholder: const ColoredBox(
                              color: Color(0xFFCCE0FF),
                              child: Center(child: CircularProgressIndicator()),
                            ),
                          ),
                        ),
                      ),
                    ),
                    const SizedBox(height: 16),
                    Wrap(
                      spacing: 12,
                      runSpacing: 12,
                      children: [
                        FilledButton(
                          onPressed: controller.startAnimation,
                          child: const Text('Start'),
                        ),
                        OutlinedButton(
                          onPressed: controller.stopAnimation,
                          child: const Text('Stop'),
                        ),
                        OutlinedButton(
                          onPressed: () async {
                            cubeColor = cubeColor == const Color(0xFFFFD400)
                                ? const Color(0xFFFF7A00)
                                : const Color(0xFFFFD400);
                            await controller.setCubeColor(cubeColor);
                            setState(() {});
                          },
                          child: const Text('Toggle Cube Color'),
                        ),
                        OutlinedButton(
                          onPressed: () async {
                            backgroundColor = backgroundColor == const Color(0xFF1B5CFF)
                                ? const Color(0xFF007C91)
                                : const Color(0xFF1B5CFF);
                            await controller.setBackgroundColor(backgroundColor);
                            setState(() {});
                          },
                          child: const Text('Toggle Background'),
                        ),
                        OutlinedButton(
                          onPressed: controller.resetScene,
                          child: const Text('Reset'),
                        ),
                      ],
                    ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}
