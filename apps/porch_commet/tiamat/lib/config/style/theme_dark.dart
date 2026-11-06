import 'dart:ui';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:tiamat/config/style/theme_base.dart';
import 'package:tiamat/config/style/theme_common.dart';
import 'package:tiamat/config/style/theme_extensions.dart';
import 'dart:io' show Platform;

class ThemeDarkColors {
  // Porch brand: #16130f base, #f3e9d8 text, #f2a541 accent
  static const Color surfaceContainerHigh = Color(0xff2f2a22);
  static const Color secondary = Color(0xff9a8f7c);
  static const Color primary = Color(0xfff2a541);
  static const Color surface = Color(0xff201b15);
  static const Color surfaceContainer = Color(0xff1b1711);
  static const Color surfaceContainerLow = Color(0xff16130f);
  static const Color surfaceLow3 = Color(0xff120f0c);
  static const Color surfaceContainerLowest = Color(0xff0e0c09);
  static const Color onSurface = Color(0xfff3e9d8);
  static const Color highlightColor = Colors.white10;
  static const Color outlineColor = Color(0xff16130f);
}

class ThemeDark {
  static ThemeData get theme {
    var scheme = ColorScheme.fromSeed(
        seedColor: ThemeDarkColors.surface,
        dynamicSchemeVariant: DynamicSchemeVariant.monochrome,
        primary: ThemeDarkColors.primary,
        onPrimary: Color(0xff16130f),
        onSurface: ThemeDarkColors.onSurface,
        surface: ThemeDarkColors.surface,
        surfaceContainer: ThemeDarkColors.surfaceContainer,
        surfaceContainerLow: ThemeDarkColors.surfaceContainerLow,
        surfaceContainerLowest: ThemeDarkColors.surfaceContainerLowest,
        primaryContainer: ThemeDarkColors.primary,
        tertiary: Color(0xfff2a541),
        tertiaryContainer: Color.fromARGB(20, 252, 191, 73),
        brightness: Brightness.dark,
        outline: ThemeDarkColors.surfaceContainerHigh);

    return ThemeBase.theme(scheme).copyWith(extensions: [
      const ThemeSettings(caulkBorders: true, caulkBorderRadius: 1),
      FoundationSettings(color: scheme.surfaceDim),
      const ExtraColors(
          codeHighlight: Color(0xffc678dd),
          linkColor: Color.fromARGB(255, 120, 120, 255))
    ]);
  }
}
