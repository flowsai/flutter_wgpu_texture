import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_wgpu_texture/flutter_wgpu_texture.dart';

void main() => runApp(const ShaderPlaygroundApp());

class ShaderPlaygroundApp extends StatefulWidget {
  const ShaderPlaygroundApp({super.key});

  @override
  State<ShaderPlaygroundApp> createState() => _ShaderPlaygroundAppState();
}

class _ShaderPlaygroundAppState extends State<ShaderPlaygroundApp> {
  late final FlutterWgpuTextureController controller;

  final Color accentColor = const Color(0xFFFF5A2A);
  final Color backgroundColor = const Color(0xFF091119);

  double speed = 1.0;
  double noiseScale = 2.4;
  double distortion = 1.0;

  @override
  void initState() {
    super.initState();
    controller = FlutterWgpuTextureController(
      autoStart: true,
      sceneType: 'shader_playground',
    );
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
      theme: ThemeData(
        colorScheme: ColorScheme.fromSeed(
          seedColor: accentColor,
          brightness: Brightness.light,
        ),
        scaffoldBackgroundColor: const Color(0xFFF4EFE8),
        useMaterial3: true,
      ),
      home: AnimatedBuilder(
        animation: controller,
        builder: (context, _) {
          final backend = controller.backendInfo;
          return Scaffold(
            body: SafeArea(
              child: Center(
                child: ConstrainedBox(
                  constraints: const BoxConstraints(maxWidth: 1180),
                  child: Padding(
                    padding: const EdgeInsets.all(24),
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: [
                        _ShaderHeader(
                          backendLabel: backend == null
                              ? 'Preparing renderer'
                              : '${backend.backend} on ${backend.deviceName}',
                        ),
                        const SizedBox(height: 24),
                        Expanded(
                          child: Row(
                            crossAxisAlignment: CrossAxisAlignment.stretch,
                            children: [
                              Expanded(
                                flex: 8,
                                child: _ShaderPreview(
                                  controller: controller,
                                  backgroundColor: backgroundColor,
                                ),
                              ),
                              const SizedBox(width: 20),
                              SizedBox(
                                width: 300,
                                child: _ShaderControls(
                                  controller: controller,
                                  speed: speed,
                                  noiseScale: noiseScale,
                                  distortion: distortion,
                                  onSpeedChanged: (value) {
                                    unawaited(_setSpeed(value));
                                  },
                                  onNoiseScaleChanged: (value) {
                                    unawaited(_setNoiseScale(value));
                                  },
                                  onDistortionChanged: (value) {
                                    unawaited(_setDistortion(value));
                                  },
                                  onReset: () {
                                    unawaited(_resetScene());
                                  },
                                ),
                              ),
                            ],
                          ),
                        ),
                      ],
                    ),
                  ),
                ),
              ),
            ),
          );
        },
      ),
    );
  }

  Future<void> _setSpeed(double value) async {
    setState(() => speed = value);
    await controller.setFloatParam('speed', value);
  }

  Future<void> _setNoiseScale(double value) async {
    setState(() => noiseScale = value);
    await controller.setFloatParam('noise_scale', value);
  }

  Future<void> _setDistortion(double value) async {
    setState(() => distortion = value);
    await controller.setFloatParam('distortion', value);
  }

  Future<void> _resetScene() async {
    setState(() {
      speed = 1.0;
      noiseScale = 2.4;
      distortion = 1.0;
    });
    await controller.resetScene();
    await controller.setFloatParam('speed', speed);
    await controller.setFloatParam('noise_scale', noiseScale);
    await controller.setFloatParam('distortion', distortion);
    if (!controller.isAnimating) {
      await controller.startAnimation();
    }
  }
}

class _ShaderHeader extends StatelessWidget {
  const _ShaderHeader({required this.backendLabel});

  final String backendLabel;

  @override
  Widget build(BuildContext context) {
    return DecoratedBox(
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(30),
        gradient: const LinearGradient(
          colors: [
            Color(0xFFFFE3D8),
            Color(0xFFDFF7FF),
          ],
          begin: Alignment.topLeft,
          end: Alignment.bottomRight,
        ),
      ),
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 28, vertical: 26),
        child: Row(
          children: [
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    'Shader Playground',
                    style: Theme.of(context).textTheme.displaySmall?.copyWith(
                      fontWeight: FontWeight.w800,
                      color: const Color(0xFF1D1A17),
                    ),
                  ),
                  const SizedBox(height: 10),
                  Text(
                    'A procedural WGSL scene with only the controls that visibly affect the output.',
                    style: Theme.of(context).textTheme.titleMedium?.copyWith(
                      color: const Color(0xFF4E473F),
                    ),
                  ),
                ],
              ),
            ),
            const SizedBox(width: 20),
            Container(
              padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 10),
              decoration: BoxDecoration(
                color: Colors.white.withValues(alpha: 0.78),
                borderRadius: BorderRadius.circular(999),
              ),
              child: Text(
                backendLabel,
                style: Theme.of(context).textTheme.labelLarge?.copyWith(
                  fontWeight: FontWeight.w700,
                  color: const Color(0xFF2E2A25),
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _ShaderPreview extends StatelessWidget {
  const _ShaderPreview({
    required this.controller,
    required this.backgroundColor,
  });

  final FlutterWgpuTextureController controller;
  final Color backgroundColor;

  @override
  Widget build(BuildContext context) {
    final isAnimating = controller.isAnimating;
    return DecoratedBox(
      decoration: BoxDecoration(
        color: Colors.white,
        borderRadius: BorderRadius.circular(30),
        boxShadow: const [
          BoxShadow(
            color: Color(0x14000000),
            blurRadius: 28,
            offset: Offset(0, 16),
          ),
        ],
      ),
      child: Padding(
        padding: const EdgeInsets.all(20),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Row(
              children: [
                Text(
                  'Live Shader',
                  style: Theme.of(context).textTheme.headlineSmall?.copyWith(
                    fontWeight: FontWeight.w700,
                  ),
                ),
                const Spacer(),
                FilledButton(
                  onPressed: () {
                    if (isAnimating) {
                      unawaited(controller.stopAnimation());
                    } else {
                      unawaited(controller.startAnimation());
                    }
                  },
                  child: Text(isAnimating ? 'Pause' : 'Play'),
                ),
              ],
            ),
            const SizedBox(height: 14),
            Expanded(
              child: ClipRRect(
                borderRadius: BorderRadius.circular(24),
                child: DecoratedBox(
                  decoration: BoxDecoration(color: backgroundColor),
                  child: FlutterWgpuTexture(
                    controller: controller,
                    placeholder: DecoratedBox(
                      decoration: BoxDecoration(color: backgroundColor),
                      child: const Center(
                        child: CircularProgressIndicator(),
                      ),
                    ),
                  ),
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _ShaderControls extends StatelessWidget {
  const _ShaderControls({
    required this.controller,
    required this.speed,
    required this.noiseScale,
    required this.distortion,
    required this.onSpeedChanged,
    required this.onNoiseScaleChanged,
    required this.onDistortionChanged,
    required this.onReset,
  });

  final FlutterWgpuTextureController controller;
  final double speed;
  final double noiseScale;
  final double distortion;
  final ValueChanged<double> onSpeedChanged;
  final ValueChanged<double> onNoiseScaleChanged;
  final ValueChanged<double> onDistortionChanged;
  final VoidCallback onReset;

  @override
  Widget build(BuildContext context) {
    return DecoratedBox(
      decoration: BoxDecoration(
        color: const Color(0xFFFDFBF8),
        borderRadius: BorderRadius.circular(30),
        border: Border.all(color: const Color(0xFFE8DED1)),
      ),
      child: Padding(
        padding: const EdgeInsets.all(22),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text(
              'Uniforms',
              style: Theme.of(context).textTheme.headlineSmall?.copyWith(
                fontWeight: FontWeight.w700,
              ),
            ),
            const SizedBox(height: 18),
            _SliderTile(
              label: 'Speed',
              valueLabel: speed.toStringAsFixed(2),
              value: speed,
              min: 0.2,
              max: 2.4,
              onChanged: onSpeedChanged,
            ),
            const SizedBox(height: 10),
            _SliderTile(
              label: 'Noise Scale',
              valueLabel: noiseScale.toStringAsFixed(2),
              value: noiseScale,
              min: 0.8,
              max: 4.0,
              onChanged: onNoiseScaleChanged,
            ),
            const SizedBox(height: 10),
            _SliderTile(
              label: 'Distortion',
              valueLabel: distortion.toStringAsFixed(2),
              value: distortion,
              min: 0.0,
              max: 2.0,
              onChanged: onDistortionChanged,
            ),
            const Spacer(),
            OutlinedButton(
              onPressed: () {
                unawaited(controller.requestFrame());
              },
              child: const Text('Render Single Frame'),
            ),
            const SizedBox(height: 10),
            FilledButton(
              onPressed: onReset,
              child: const Text('Reset Shader'),
            ),
          ],
        ),
      ),
    );
  }
}

class _SliderTile extends StatelessWidget {
  const _SliderTile({
    required this.label,
    required this.valueLabel,
    required this.value,
    required this.min,
    required this.max,
    required this.onChanged,
  });

  final String label;
  final String valueLabel;
  final double value;
  final double min;
  final double max;
  final ValueChanged<double> onChanged;

  @override
  Widget build(BuildContext context) {
    return DecoratedBox(
      decoration: BoxDecoration(
        color: Colors.white,
        borderRadius: BorderRadius.circular(18),
        border: Border.all(color: const Color(0xFFE4DACC)),
      ),
      child: Padding(
        padding: const EdgeInsets.fromLTRB(14, 12, 14, 8),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Text(
                  label,
                  style: Theme.of(context).textTheme.titleSmall?.copyWith(
                    fontWeight: FontWeight.w700,
                  ),
                ),
                const Spacer(),
                Text(
                  valueLabel,
                  style: Theme.of(context).textTheme.labelLarge,
                ),
              ],
            ),
            Slider(
              value: value,
              min: min,
              max: max,
              onChanged: onChanged,
            ),
          ],
        ),
      ),
    );
  }
}
