import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';

import 'frb_generated.dart';
import 'rust_dylib.dart';

Future<void>? _rustInitFuture;

Future<void> ensureRustInitialized() async {
  try {
    _rustInitFuture ??= RustLib.init(
      externalLibrary: ExternalLibrary.open(RustDynamicLibrary.open()),
    );
    await _rustInitFuture;
  } catch (_) {
    _rustInitFuture = null;
    rethrow;
  }
}
