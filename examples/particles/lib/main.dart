import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_wgpu_texture/flutter_wgpu_texture.dart';

void main() => runApp(const ParticlesApp());

class ParticlesApp extends StatefulWidget {
  const ParticlesApp({super.key});

  @override
  State<ParticlesApp> createState() => _ParticlesAppState();
}

class _ParticlesAppState extends State<ParticlesApp> {
  late final FlutterWgpuTextureController controller;

  final Color accentColor = const Color(0xFFFFD54A);
  final Color backgroundColor = const Color(0xFF03070F);

  double pointSize = 14.0;
  double motionScale = 1.0;

  @override
  void initState() {
    super.initState();
    controller = FlutterWgpuTextureController(
      autoStart: true,
      sceneType: 'particles',
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
        scaffoldBackgroundColor: const Color(0xFFF5F1EA),
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
                  constraints: const BoxConstraints(maxWidth: 1120),
                  child: Padding(
                    padding: const EdgeInsets.all(24),
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: [
                        _HeroHeader(
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
                                flex: 7,
                                child: _PreviewPanel(
                                  controller: controller,
                                  backgroundColor: backgroundColor,
                                ),
                              ),
                              const SizedBox(width: 20),
                              SizedBox(
                                width: 300,
                                child: _ControlPanel(
                                  controller: controller,
                                  pointSize: pointSize,
                                  motionScale: motionScale,
                                  onPointSizeChanged: (value) {
                                    unawaited(_setPointSize(value));
                                  },
                                  onMotionScaleChanged: (value) {
                                    unawaited(_setMotionScale(value));
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

  Future<void> _setPointSize(double value) async {
    setState(() => pointSize = value);
    await controller.setFloatParam('point_size', value);
  }

  Future<void> _setMotionScale(double value) async {
    setState(() => motionScale = value);
    await controller.setFloatParam('motion_scale', value);
  }

  Future<void> _resetScene() async {
    setState(() {
      pointSize = 14.0;
      motionScale = 1.0;
    });
    await controller.resetScene();
    await controller.setFloatParam('point_size', pointSize);
    await controller.setFloatParam('motion_scale', motionScale);
    if (!controller.isAnimating) {
      await controller.startAnimation();
    }
  }
}

class _HeroHeader extends StatelessWidget {
  const _HeroHeader({required this.backendLabel});

  final String backendLabel;

  @override
  Widget build(BuildContext context) {
    return DecoratedBox(
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(28),
        gradient: const LinearGradient(
          colors: [
            Color(0xFFFFF3D6),
            Color(0xFFE8F6FF),
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
                    'Particles',
                    style: Theme.of(context).textTheme.displaySmall?.copyWith(
                      fontWeight: FontWeight.w800,
                      color: const Color(0xFF1B1B1B),
                    ),
                  ),
                  const SizedBox(height: 10),
                  Text(
                    'A clean desktop particle demo with only the controls that actually matter.',
                    style: Theme.of(context).textTheme.titleMedium?.copyWith(
                      color: const Color(0xFF3F3B36),
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

class _PreviewPanel extends StatelessWidget {
  const _PreviewPanel({
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
                  'Live Scene',
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
                      child: const Center(child: CircularProgressIndicator()),
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

class _ControlPanel extends StatelessWidget {
  const _ControlPanel({
    required this.controller,
    required this.pointSize,
    required this.motionScale,
    required this.onPointSizeChanged,
    required this.onMotionScaleChanged,
    required this.onReset,
  });

  final FlutterWgpuTextureController controller;
  final double pointSize;
  final double motionScale;
  final ValueChanged<double> onPointSizeChanged;
  final ValueChanged<double> onMotionScaleChanged;
  final VoidCallback onReset;

  @override
  Widget build(BuildContext context) {
    return DecoratedBox(
      decoration: BoxDecoration(
        color: const Color(0xFFFDFBF8),
        borderRadius: BorderRadius.circular(30),
        border: Border.all(color: const Color(0xFFE9E0D3)),
      ),
      child: Padding(
        padding: const EdgeInsets.all(22),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text(
              'Controls',
              style: Theme.of(context).textTheme.headlineSmall?.copyWith(
                fontWeight: FontWeight.w700,
              ),
            ),
            const SizedBox(height: 18),
            _SliderTile(
              label: 'Particle Size',
              valueLabel: pointSize.toStringAsFixed(1),
              value: pointSize,
              min: 2.0,
              max: 18.0,
              onChanged: onPointSizeChanged,
            ),
            const SizedBox(height: 10),
            _SliderTile(
              label: 'Motion Scale',
              valueLabel: motionScale.toStringAsFixed(2),
              value: motionScale,
              min: 0.4,
              max: 1.8,
              onChanged: onMotionScaleChanged,
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
              child: const Text('Reset Scene'),
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
