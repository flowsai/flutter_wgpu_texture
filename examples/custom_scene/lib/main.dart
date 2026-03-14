import 'package:flutter/material.dart';
import 'package:flutter_wgpu_texture/flutter_wgpu_texture.dart';

void main() => runApp(const CustomSceneApp());

class CustomSceneApp extends StatefulWidget {
  const CustomSceneApp({super.key});

  @override
  State<CustomSceneApp> createState() => _CustomSceneAppState();
}

class _CustomSceneAppState extends State<CustomSceneApp> {
  late final FlutterWgpuTextureController controller;

  // Colors exposed by the gradient scene via set_vec4_param.
  List<double> colorA = [0.08, 0.26, 0.78, 1.0];
  List<double> colorB = [0.85, 0.18, 0.52, 1.0];

  @override
  void initState() {
    super.initState();
    // 'gradient' is registered by gradient_scene's #[ctor::ctor] at dylib load.
    controller = FlutterWgpuTextureController(sceneType: 'gradient');
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
        backgroundColor: const Color(0xFF0D0D14),
        body: SafeArea(
          child: Padding(
            padding: const EdgeInsets.all(24),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                const Text(
                  'Custom Scene',
                  style: TextStyle(
                    fontSize: 34,
                    fontWeight: FontWeight.bold,
                    color: Colors.white,
                  ),
                ),
                const SizedBox(height: 6),
                const Text(
                  'Animated gradient — implemented outside the plugin.',
                  style: TextStyle(fontSize: 15, color: Colors.white54),
                ),
                const SizedBox(height: 20),
                Expanded(
                  child: ClipRRect(
                    borderRadius: BorderRadius.circular(24),
                    child: FlutterWgpuTexture(
                      controller: controller,
                      placeholder: const ColoredBox(
                        color: Color(0xFF1A1A2E),
                        child: Center(child: CircularProgressIndicator()),
                      ),
                    ),
                  ),
                ),
                const SizedBox(height: 16),
                Wrap(
                  spacing: 10,
                  runSpacing: 10,
                  children: [
                    FilledButton(
                      onPressed: controller.startAnimation,
                      child: const Text('Play'),
                    ),
                    OutlinedButton(
                      onPressed: controller.stopAnimation,
                      style: OutlinedButton.styleFrom(
                        foregroundColor: Colors.white54,
                        side: const BorderSide(color: Colors.white24),
                      ),
                      child: const Text('Pause'),
                    ),
                    OutlinedButton(
                      onPressed: () async {
                        // Toggle color_a between blue and teal.
                        colorA = colorA[2] > 0.5
                            ? [0.1, 0.7, 0.5, 1.0]
                            : [0.08, 0.26, 0.78, 1.0];
                        await controller.setVec4Param(
                          'color_a',
                          colorA.map((v) => v.toDouble()).toList(),
                        );
                        setState(() {});
                      },
                      style: OutlinedButton.styleFrom(
                        foregroundColor: Colors.white54,
                        side: const BorderSide(color: Colors.white24),
                      ),
                      child: const Text('Color A'),
                    ),
                    OutlinedButton(
                      onPressed: () async {
                        // Toggle color_b between pink and orange.
                        colorB = colorB[0] > 0.5
                            ? [1.0, 0.55, 0.1, 1.0]
                            : [0.85, 0.18, 0.52, 1.0];
                        await controller.setVec4Param(
                          'color_b',
                          colorB.map((v) => v.toDouble()).toList(),
                        );
                        setState(() {});
                      },
                      style: OutlinedButton.styleFrom(
                        foregroundColor: Colors.white54,
                        side: const BorderSide(color: Colors.white24),
                      ),
                      child: const Text('Color B'),
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
