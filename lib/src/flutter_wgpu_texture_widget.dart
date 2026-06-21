import 'package:flutter/foundation.dart';
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
  Size? _lastSize;

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
    // The render Ticker was vended by this State's SingleTickerProviderStateMixin
    // but is stored in the (longer-lived) controller backend. Dispose it before
    // super.dispose(), or the mixin asserts "disposed with an active Ticker".
    widget.controller.detachTicker();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
        final width = widget.width ?? _fallbackDimension(constraints.maxWidth);
        final height =
            widget.height ?? _fallbackDimension(constraints.maxHeight);
        final size = Size(width, height);
        if (_lastSize != size) {
          _lastSize = size;
          WidgetsBinding.instance.addPostFrameCallback((_) {
            // The State may have been disposed between scheduling and firing
            // (e.g. the viewport unmounted on a mode switch). Initializing then
            // would create a Ticker against a dead TickerProvider.
            if (!mounted) return;
            widget.controller.ensureInitialized(size, this);
          });
        }
        final textureId = widget.controller.textureId;
        final viewType = widget.controller.viewType;
        final unsupportedReason = widget.controller.unsupportedReason;
        if (unsupportedReason != null) {
          return SizedBox(
            width: width,
            height: height,
            child: ColoredBox(
              color: Colors.black12,
              child: Center(
                child: Padding(
                  padding: const EdgeInsets.all(16),
                  child: Text(
                    unsupportedReason,
                    textAlign: TextAlign.center,
                  ),
                ),
              ),
            ),
          );
        }
        if ((kIsWeb && !widget.controller.isInitialized) ||
            (!kIsWeb && textureId == null)) {
          return SizedBox(
            width: width,
            height: height,
            child:
                widget.placeholder ??
                const ColoredBox(
                  color: Colors.black12,
                  child: SizedBox.expand(),
                ),
          );
        }
        return SizedBox(
          width: width,
          height: height,
          child: kIsWeb
              ? HtmlElementView(viewType: viewType!)
              : Texture(textureId: textureId!),
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
