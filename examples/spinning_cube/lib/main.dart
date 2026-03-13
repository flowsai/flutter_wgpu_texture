import 'package:flutter/material.dart';
import 'package:flutter_wgpu_texture/flutter_wgpu_texture.dart';

void main() => runApp(const SpinningCubeApp());

class SpinningCubeApp extends StatefulWidget {
  const SpinningCubeApp({super.key});

  @override
  State<SpinningCubeApp> createState() => _SpinningCubeAppState();
}

class _SpinningCubeAppState extends State<SpinningCubeApp> {
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
        backgroundColor: const Color(0xFFF0F4F8),
        body: SafeArea(
          child: Padding(
            padding: const EdgeInsets.all(24),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                const Text(
                  'Spinning Cube',
                  style: TextStyle(fontSize: 34, fontWeight: FontWeight.bold),
                ),
                const SizedBox(height: 8),
                const Text(
                  'Desktop-only Flutter texture plugin using Rust + wgpu.',
                  style: TextStyle(fontSize: 16, color: Colors.black54),
                ),
                const SizedBox(height: 20),
                Expanded(
                  child: DecoratedBox(
                    decoration: BoxDecoration(
                      color: Colors.white,
                      borderRadius: BorderRadius.circular(28),
                      boxShadow: const [
                        BoxShadow(
                          color: Color(0x12000000),
                          blurRadius: 24,
                          offset: Offset(0, 12),
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
                            ? const Color(0xFF6BFF3E)
                            : const Color(0xFFFFD400);
                        await controller.setCubeColor(cubeColor);
                        setState(() {});
                      },
                      child: const Text('Cube Color'),
                    ),
                    OutlinedButton(
                      onPressed: () async {
                        backgroundColor = backgroundColor == const Color(0xFF1B5CFF)
                            ? const Color(0xFF053A7A)
                            : const Color(0xFF1B5CFF);
                        await controller.setBackgroundColor(backgroundColor);
                        setState(() {});
                      },
                      child: const Text('Background'),
                    ),
                    OutlinedButton(
                      onPressed: () async {
                        await controller.setRotationSpeed(
                          controller.isAnimating ? 0.6 : 1.25,
                        );
                      },
                      child: const Text('Speed'),
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
    );
  }
}
