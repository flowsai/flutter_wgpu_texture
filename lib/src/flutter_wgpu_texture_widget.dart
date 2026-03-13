import 'package:flutter/material.dart';

import 'flutter_wgpu_texture_controller.dart';

class FlutterWgpuTexture extends StatefulWidget {
  const FlutterWgpuTexture({
    super.key,
    required this.controller,
    this.width,
    this.height,
    this.placeholder,
  });

  final FlutterWgpuTextureController controller;
  final double? width;
  final double? height;
  final Widget? placeholder;

  @override
  State<FlutterWgpuTexture> createState() => _FlutterWgpuTextureState();
}

class _FlutterWgpuTextureState extends State<FlutterWgpuTexture>
    with SingleTickerProviderStateMixin {
  @override
  void initState() {
    super.initState();
    widget.controller.addListener(_onControllerChanged);
  }

  @override
  void didUpdateWidget(covariant FlutterWgpuTexture oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.controller != widget.controller) {
      oldWidget.controller.removeListener(_onControllerChanged);
      widget.controller.addListener(_onControllerChanged);
    }
  }

  @override
  void dispose() {
    widget.controller.removeListener(_onControllerChanged);
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
        final width = widget.width ?? _fallbackDimension(constraints.maxWidth);
        final height = widget.height ?? _fallbackDimension(constraints.maxHeight);
        WidgetsBinding.instance.addPostFrameCallback((_) {
          widget.controller.ensureInitialized(Size(width, height), this);
        });
        final textureId = widget.controller.textureId;
        if (textureId == null) {
          return SizedBox(
            width: width,
            height: height,
            child:
                widget.placeholder ??
                const ColoredBox(color: Colors.black12, child: SizedBox.expand()),
          );
        }
        return SizedBox(
          width: width,
          height: height,
          child: Texture(textureId: textureId),
        );
      },
    );
  }

  void _onControllerChanged() {
    if (mounted) {
      setState(() {});
    }
  }

  double _fallbackDimension(double value) {
    if (value.isFinite && value > 0) {
      return value;
    }
    return 512;
  }
}
