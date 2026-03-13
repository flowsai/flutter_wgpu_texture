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

  final List<_PalettePreset> presets = const [
    _PalettePreset(
      name: 'Solar Flare',
      description: 'Electric amber sparks against ice-blue highlights.',
      primary: Color(0xFFFFD54A),
      secondary: Color(0xFF7FDBFF),
      background: Color(0xFF03070F),
    ),
    _PalettePreset(
      name: 'Aurora',
      description: 'Neon mint and ultraviolet over a near-black dusk canvas.',
      primary: Color(0xFF9AFFE0),
      secondary: Color(0xFFA78BFF),
      background: Color(0xFF02070A),
    ),
    _PalettePreset(
      name: 'Ember',
      description: 'Hot pink and pale gold over a charred neutral backdrop.',
      primary: Color(0xFFFF7DA7),
      secondary: Color(0xFFFFE08A),
      background: Color(0xFF090506),
    ),
  ];

  late _PalettePreset selectedPreset;
  late Color backgroundColor;
  double pointSize = 14.0;
  double motionScale = 1.0;
  bool didSyncInitialPreset = false;

  @override
  void initState() {
    super.initState();
    selectedPreset = presets.first;
    backgroundColor = selectedPreset.background;
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
          seedColor: selectedPreset.primary,
          brightness: Brightness.light,
        ),
        scaffoldBackgroundColor: const Color(0xFFF5F1EA),
        useMaterial3: true,
      ),
      home: AnimatedBuilder(
        animation: controller,
        builder: (context, _) {
          final backend = controller.backendInfo;
          if (controller.isInitialized && !didSyncInitialPreset) {
            didSyncInitialPreset = true;
            WidgetsBinding.instance.addPostFrameCallback((_) {
              unawaited(_applyPreset(selectedPreset));
            });
          }
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
                        _HeroHeader(
                          preset: selectedPreset,
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
                                width: 320,
                                child: _ControlPanel(
                                  controller: controller,
                                  presets: presets,
                                  selectedPreset: selectedPreset,
                                  pointSize: pointSize,
                                  motionScale: motionScale,
                                  onPresetSelected: (preset) {
                                    unawaited(_applyPreset(preset));
                                  },
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

  Future<void> _applyPreset(_PalettePreset preset) async {
    setState(() {
      selectedPreset = preset;
      backgroundColor = preset.background;
    });
    await controller.setVec4Param('color1', _colorToVec4(preset.primary));
    await controller.setVec4Param('color2', _colorToVec4(preset.secondary));
    await controller.setBackgroundColor(preset.background);
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
      selectedPreset = presets.first;
      backgroundColor = presets.first.background;
      pointSize = 14.0;
      motionScale = 1.0;
    });
    await controller.resetScene();
    await _applyPreset(selectedPreset);
    await controller.setFloatParam('point_size', pointSize);
    await controller.setFloatParam('motion_scale', motionScale);
    if (!controller.isAnimating) {
      await controller.startAnimation();
    }
  }

  List<double> _colorToVec4(Color color) {
    return [color.r / 255.0, color.g / 255.0, color.b / 255.0, color.a / 255.0];
  }
}

class _HeroHeader extends StatelessWidget {
  const _HeroHeader({
    required this.preset,
    required this.backendLabel,
  });

  final _PalettePreset preset;
  final String backendLabel;

  @override
  Widget build(BuildContext context) {
    return DecoratedBox(
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(28),
        gradient: LinearGradient(
          colors: [
            Color.alphaBlend(
              preset.primary.withValues(alpha: 0.16),
              const Color(0xFFFFFFFF),
            ),
            Color.alphaBlend(
              preset.secondary.withValues(alpha: 0.14),
              const Color(0xFFF8F3EA),
            ),
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
                    'A wgpu-driven particle field tuned like the spinning cube demo: one strong viewport, real scene controls, no junk UI.',
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
    required this.presets,
    required this.selectedPreset,
    required this.pointSize,
    required this.motionScale,
    required this.onPresetSelected,
    required this.onPointSizeChanged,
    required this.onMotionScaleChanged,
    required this.onReset,
  });

  final FlutterWgpuTextureController controller;
  final List<_PalettePreset> presets;
  final _PalettePreset selectedPreset;
  final double pointSize;
  final double motionScale;
  final ValueChanged<_PalettePreset> onPresetSelected;
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
        child: LayoutBuilder(
          builder: (context, constraints) {
            return SingleChildScrollView(
              child: ConstrainedBox(
                constraints: BoxConstraints(minHeight: constraints.maxHeight),
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
                    Text(
                      'Palette',
                      style: Theme.of(context).textTheme.titleMedium?.copyWith(
                        fontWeight: FontWeight.w700,
                      ),
                    ),
                    const SizedBox(height: 10),
                    ...presets.map(
                      (preset) => Padding(
                        padding: const EdgeInsets.only(bottom: 10),
                        child: _PresetButton(
                          preset: preset,
                          isSelected: preset == selectedPreset,
                          onTap: () => onPresetSelected(preset),
                        ),
                      ),
                    ),
                    const SizedBox(height: 10),
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
                    SizedBox(height: constraints.maxHeight > 560 ? 24 : 16),
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
          },
        ),
      ),
    );
  }
}

class _PresetButton extends StatelessWidget {
  const _PresetButton({
    required this.preset,
    required this.isSelected,
    required this.onTap,
  });

  final _PalettePreset preset;
  final bool isSelected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return InkWell(
      onTap: onTap,
      borderRadius: BorderRadius.circular(18),
      child: Ink(
        padding: const EdgeInsets.all(14),
        decoration: BoxDecoration(
          borderRadius: BorderRadius.circular(18),
          border: Border.all(
            color: isSelected ? preset.primary : const Color(0xFFE4DACC),
            width: isSelected ? 1.8 : 1.0,
          ),
          color: Colors.white,
        ),
        child: Row(
          children: [
            _ColorSwatch(color: preset.primary),
            const SizedBox(width: 8),
            _ColorSwatch(color: preset.secondary),
            const SizedBox(width: 12),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    preset.name,
                    style: Theme.of(context).textTheme.titleSmall?.copyWith(
                      fontWeight: FontWeight.w700,
                    ),
                  ),
                  const SizedBox(height: 4),
                  Text(
                    preset.description,
                    style: Theme.of(context).textTheme.bodySmall?.copyWith(
                      color: const Color(0xFF5B544C),
                    ),
                  ),
                ],
              ),
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

class _ColorSwatch extends StatelessWidget {
  const _ColorSwatch({required this.color});

  final Color color;

  @override
  Widget build(BuildContext context) {
    return Container(
      width: 18,
      height: 36,
      decoration: BoxDecoration(
        color: color,
        borderRadius: BorderRadius.circular(999),
      ),
    );
  }
}

class _PalettePreset {
  const _PalettePreset({
    required this.name,
    required this.description,
    required this.primary,
    required this.secondary,
    required this.background,
  });

  final String name;
  final String description;
  final Color primary;
  final Color secondary;
  final Color background;
}
