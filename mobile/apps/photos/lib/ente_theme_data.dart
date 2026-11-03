// PHOTOVAULT: theme (ColorTokens no longer referenced)
import 'package:ente_components/theme/colors.dart' show ComponentApp;
// PHOTOVAULT: theme
import 'package:ente_components/theme/photovault/encore_tokens.dart';
import 'package:ente_components/theme/text_styles.dart' as component;
import 'package:ente_components/theme/theme.dart'
    show ComponentColorTokens, ComponentTheme;
import 'package:flutter/material.dart';
import 'package:photos/theme/colors.dart';
import 'package:photos/theme/ente_theme.dart';

final lightThemeData = ThemeData(
  fontFamily: 'Inter',
  brightness: Brightness.light,
  // PHOTOVAULT: theme
  popupMenuTheme: const PopupMenuThemeData(color: encoreLightPanel),
  hintColor: const Color.fromRGBO(158, 158, 158, 1),
  primaryColor: const Color.fromRGBO(255, 110, 64, 1),
  primaryColorLight: const Color.fromRGBO(0, 0, 0, 0.541),
  iconTheme: const IconThemeData(color: Colors.black),
  primaryIconTheme: const IconThemeData(
    color: Colors.red,
    opacity: 1.0,
    size: 50.0,
  ),
  colorScheme: const ColorScheme.light(
    primary: encoreLightAccent, // PHOTOVAULT: theme
    secondary: Color.fromARGB(255, 163, 163, 163),
    surface: backgroundColorLight,
    surfaceTint: Colors.transparent,
  ),
  outlinedButtonTheme: buildOutlinedButtonThemeData(
    bgDisabled: const Color.fromRGBO(158, 158, 158, 1),
    bgEnabled: encoreLightAccent, // PHOTOVAULT: theme
    fgDisabled: const Color.fromRGBO(255, 255, 255, 1),
    fgEnabled: encoreLightOnAccent, // PHOTOVAULT: theme
  ),
  elevatedButtonTheme: buildElevatedButtonThemeData(
    onPrimary: encoreLightOnAccent, // PHOTOVAULT: theme
    primary: encoreLightAccent, // PHOTOVAULT: theme
  ),
  switchTheme: getSwitchThemeData(encoreLightAccent2), // PHOTOVAULT: theme
  scaffoldBackgroundColor: backgroundColorLight,
  appBarTheme: const AppBarTheme().copyWith(
    backgroundColor: backgroundColorLight,
    foregroundColor: Colors.black,
    iconTheme: const IconThemeData(color: Colors.black),
    elevation: 0,
  ),
  textTheme: _buildTextTheme(encoreLightText), // PHOTOVAULT: theme
  primaryTextTheme: const TextTheme().copyWith(
    bodyMedium: const TextStyle(color: Colors.yellow),
    bodyLarge: const TextStyle(color: Colors.orange),
  ),
  cardColor: encoreLightPanel, // PHOTOVAULT: theme
  cardTheme: _encoreCardTheme, // PHOTOVAULT: theme
  dialogTheme: const DialogThemeData(
    backgroundColor: encoreLightPanel, // PHOTOVAULT: theme
    titleTextStyle: TextStyle(
      color: Colors.black,
      fontSize: 24,
      fontWeight: FontWeight.w600,
    ),
    contentTextStyle: TextStyle(
      fontFamily: 'Inter-Medium',
      color: Colors.black,
      fontSize: 16,
      fontWeight: FontWeight.w500,
    ),
    shape: RoundedRectangleBorder(
      // PHOTOVAULT: theme: Encore card radius.
      borderRadius: BorderRadius.all(Radius.circular(EncoreTokens.radiusCard)),
    ),
  ),
  inputDecorationTheme: const InputDecorationTheme().copyWith(
    focusedBorder: const UnderlineInputBorder(
      borderSide: BorderSide(color: encoreLightAccent2), // PHOTOVAULT: theme
    ),
  ),
  checkboxTheme: CheckboxThemeData(
    side: const BorderSide(color: Colors.black, width: 2),
    fillColor: WidgetStateProperty.resolveWith((states) {
      return states.contains(WidgetState.selected)
          ? encoreLightAccent // PHOTOVAULT: theme
          : const Color.fromRGBO(255, 255, 255, 1);
    }),
    checkColor: WidgetStateProperty.resolveWith((states) {
      return states.contains(WidgetState.selected)
          ? encoreLightOnAccent // PHOTOVAULT: theme
          : const Color.fromRGBO(0, 0, 0, 1);
    }),
  ),
  extensions: [
    ComponentColorTokens(
      ComponentTheme.colorsForApp(
        ComponentApp.photos,
        brightness: Brightness.light,
      ),
    ),
  ],
);

final darkThemeData = ThemeData(
  fontFamily: 'Inter',
  brightness: Brightness.dark,
  // PHOTOVAULT: theme
  popupMenuTheme: const PopupMenuThemeData(color: encoreDarkPanel),
  primaryColorLight: const Color.fromRGBO(255, 255, 255, 0.702),
  iconTheme: const IconThemeData(color: Colors.white),
  primaryIconTheme: const IconThemeData(
    color: Colors.red,
    opacity: 1.0,
    size: 50.0,
  ),
  hintColor: const Color.fromRGBO(158, 158, 158, 1),
  colorScheme: const ColorScheme.dark(
    primary: encoreDarkAccent, // PHOTOVAULT: theme
    surface: backgroundColorDark,
    secondary: Color.fromARGB(255, 163, 163, 163),
    surfaceTint: Colors.transparent,
  ),
  buttonTheme: const ButtonThemeData().copyWith(
    buttonColor: encoreDarkAccent, // PHOTOVAULT: theme
  ),
  textTheme: _buildTextTheme(encoreDarkText), // PHOTOVAULT: theme
  switchTheme: getSwitchThemeData(encoreDarkAccent2), // PHOTOVAULT: theme
  outlinedButtonTheme: buildOutlinedButtonThemeData(
    bgDisabled: const Color.fromRGBO(158, 158, 158, 1),
    bgEnabled: encoreDarkAccent, // PHOTOVAULT: theme
    fgDisabled: const Color.fromRGBO(255, 255, 255, 1),
    fgEnabled: encoreDarkOnAccent, // PHOTOVAULT: theme
  ),
  elevatedButtonTheme: buildElevatedButtonThemeData(
    onPrimary: encoreDarkOnAccent, // PHOTOVAULT: theme
    primary: encoreDarkAccent, // PHOTOVAULT: theme
  ),
  scaffoldBackgroundColor: backgroundColorDark,
  appBarTheme: const AppBarTheme().copyWith(
    backgroundColor: backgroundColorDark,
    elevation: 0,
  ),
  cardColor: encoreDarkPanel, // PHOTOVAULT: theme
  cardTheme: _encoreCardTheme, // PHOTOVAULT: theme
  dialogTheme: const DialogThemeData(
    backgroundColor: encoreDarkBg2, // PHOTOVAULT: theme
    titleTextStyle: TextStyle(
      color: Colors.white,
      fontSize: 24,
      fontWeight: FontWeight.w600,
    ),
    contentTextStyle: TextStyle(
      fontFamily: 'Inter-Medium',
      color: Colors.white,
      fontSize: 16,
      fontWeight: FontWeight.w500,
    ),
    shape: RoundedRectangleBorder(
      // PHOTOVAULT: theme: Encore card radius.
      borderRadius: BorderRadius.all(Radius.circular(EncoreTokens.radiusCard)),
    ),
  ),
  inputDecorationTheme: const InputDecorationTheme().copyWith(
    focusedBorder: const UnderlineInputBorder(
      borderSide: BorderSide(color: encoreDarkAccent2), // PHOTOVAULT: theme
    ),
  ),
  checkboxTheme: CheckboxThemeData(
    side: const BorderSide(color: Colors.grey, width: 2),
    fillColor: WidgetStateProperty.resolveWith((states) {
      if (states.contains(WidgetState.selected)) {
        return encoreDarkAccent; // PHOTOVAULT: theme
      } else {
        return const Color.fromRGBO(0, 0, 0, 1);
      }
    }),
    checkColor: WidgetStateProperty.resolveWith((states) {
      if (states.contains(WidgetState.selected)) {
        return encoreDarkOnAccent; // PHOTOVAULT: theme
      } else {
        return const Color.fromRGBO(158, 158, 158, 1);
      }
    }),
  ),
  extensions: [
    ComponentColorTokens(
      ComponentTheme.colorsForApp(
        ComponentApp.photos,
        brightness: Brightness.dark,
      ),
    ),
  ],
);

TextTheme _buildTextTheme(Color textColor) {
  return const TextTheme().copyWith(
    headlineMedium: TextStyle(
      color: textColor,
      fontSize: 32,
      fontWeight: FontWeight.w600,
      fontFamily: 'Inter',
    ),
    headlineSmall: TextStyle(
      color: textColor,
      fontSize: 24,
      fontWeight: FontWeight.w600,
      fontFamily: 'Inter',
    ),
    titleLarge: component.TextStyles.h2.copyWith(color: textColor),
    titleMedium: component.TextStyles.large.copyWith(color: textColor),
    titleSmall: component.TextStyles.body.copyWith(color: textColor),
    bodyLarge: component.TextStyles.large.copyWith(color: textColor),
    bodyMedium: component.TextStyles.body.copyWith(color: textColor),
    bodySmall: component.TextStyles.body.copyWith(
      color: textColor.withValues(alpha: 0.6),
    ),
    labelSmall: component.TextStyles.bodyLink.copyWith(color: textColor),
  );
}

extension CustomColorScheme on ColorScheme {
  // PHOTOVAULT: theme
  Color get videoPlayerPrimaryColor =>
      brightness == Brightness.light ? encoreLightAccent2 : encoreDarkAccent2;

  Color get videoPlayerBorderColor => brightness == Brightness.light
      ? const Color(0xFF424242)
      : const Color(0xFFFFFFFF);

  Color get imageEditorPrimaryColor => encoreDarkAccent2; // PHOTOVAULT: theme

  Color get editorBackgroundColor => brightness == Brightness.light
      ? const Color(0xFFF5F5F5)
      : const Color(0xFF252525);

  Color get defaultBackgroundColor => brightness == Brightness.light
      ? backgroundColorLight
      : backgroundColorDark;

  Color get inverseBackgroundColor => brightness != Brightness.light
      ? backgroundColorLight
      : backgroundColorDark;

  Color get defaultTextColor =>
      brightness == Brightness.light ? textBaseLight : textBaseDark;

  Color get inverseTextColor =>
      brightness != Brightness.light ? textBaseLight : textBaseDark;

  // PHOTOVAULT: theme
  Color get boxSelectColor =>
      brightness == Brightness.light ? encoreLightAccent2 : encoreDarkPanel2;

  Color get boxUnSelectColor => brightness == Brightness.light
      ? const Color.fromRGBO(240, 240, 240, 1)
      : const Color.fromRGBO(8, 18, 18, 0.4);

  // PHOTOVAULT: theme
  Color get greenAlternative =>
      brightness == Brightness.light ? encoreLightAccent2 : encoreDarkAccent2;

  Color get dynamicFABBackgroundColor => brightness == Brightness.light
      ? const Color.fromRGBO(0, 0, 0, 1)
      : const Color.fromRGBO(48, 48, 48, 1);

  Color get dynamicFABTextColor => const Color.fromRGBO(255, 255, 255, 1);

  // todo: use brightness == Brightness.light for changing color for dark/light theme
  ButtonStyle? get optionalActionButtonStyle => buildElevatedButtonThemeData(
    onPrimary: const Color(0xFF777777),
    primary: const Color(0xFFF0F0F0),
    elevation: 0,
  ).style;

  // PHOTOVAULT: theme
  Color get recoveryKeyBoxColor => brightness == Brightness.light
      ? encoreLightAccent2.withValues(alpha: 0.2)
      : encoreDarkAccent2;

  Color get frostyBlurBackdropFilterColor => brightness == Brightness.light
      ? const Color.fromRGBO(238, 238, 238, 0.5)
      : const Color.fromRGBO(48, 48, 48, 0.5);

  Color get iconColor => brightness == Brightness.light
      ? const Color.fromRGBO(0, 0, 0, 1).withValues(alpha: 0.75)
      : const Color.fromRGBO(255, 255, 255, 1);

  Color get bgColorForQuestions => brightness == Brightness.light
      ? const Color.fromRGBO(255, 255, 255, 1)
      : const Color.fromRGBO(10, 15, 15, 1.0);

  // PHOTOVAULT: theme
  Color get greenText =>
      brightness == Brightness.light ? encoreLightAccent2 : encoreDarkAccent2;

  Color get cupertinoPickerTopColor => brightness == Brightness.light
      ? const Color.fromARGB(255, 238, 238, 238)
      : const Color.fromRGBO(255, 255, 255, 1).withValues(alpha: 0.1);

  Color get stepProgressUnselectedColor => brightness == Brightness.light
      ? const Color.fromRGBO(196, 196, 196, 0.6)
      : const Color.fromRGBO(255, 255, 255, 0.7);

  Color get galleryThumbBackgroundColor => brightness == Brightness.light
      ? const Color.fromRGBO(240, 240, 240, 1)
      : const Color.fromRGBO(20, 20, 20, 1);

  Color get galleryThumbDrawColor => brightness == Brightness.light
      ? const Color.fromRGBO(0, 0, 0, 1).withValues(alpha: 0.8)
      : const Color.fromRGBO(255, 255, 255, 1).withValues(alpha: 0.5);

  Color get backupEnabledBgColor => brightness == Brightness.light
      ? const Color.fromRGBO(230, 230, 230, 0.95)
      : const Color.fromRGBO(10, 40, 40, 0.3);

  Color get dotsIndicatorActiveColor => brightness == Brightness.light
      ? const Color.fromRGBO(0, 0, 0, 1).withValues(alpha: 0.5)
      : const Color.fromRGBO(255, 255, 255, 1).withValues(alpha: 0.5);

  Color get dotsIndicatorInactiveColor => brightness == Brightness.light
      ? const Color.fromRGBO(0, 0, 0, 1).withValues(alpha: 0.12)
      : const Color.fromRGBO(255, 255, 255, 1).withValues(alpha: 0.12);

  Color get toastTextColor => brightness == Brightness.light
      ? const Color.fromRGBO(255, 255, 255, 1)
      : const Color.fromRGBO(0, 0, 0, 1);

  Color get toastBackgroundColor => brightness == Brightness.light
      ? const Color.fromRGBO(24, 24, 24, 0.95)
      : const Color.fromRGBO(255, 255, 255, 0.95);

  Color get subTextColor => brightness == Brightness.light
      ? const Color.fromRGBO(180, 180, 180, 1)
      : const Color.fromRGBO(100, 100, 100, 1);

  Color get searchResultsColor => brightness == Brightness.light
      ? const Color.fromRGBO(245, 245, 245, 1.0)
      : const Color.fromRGBO(30, 30, 30, 1.0);

  Color get searchResultsCountTextColor => brightness == Brightness.light
      ? const Color.fromRGBO(80, 80, 80, 1)
      : const Color.fromRGBO(150, 150, 150, 1);

  Color get searchResultsBackgroundColor => brightness == Brightness.light
      ? Colors.black.withValues(alpha: 0.32)
      : Colors.black.withValues(alpha: 0.64);

  EnteTheme get enteTheme =>
      brightness == Brightness.light ? lightTheme : darkTheme;

  EnteTheme get inverseEnteTheme =>
      brightness == Brightness.light ? darkTheme : lightTheme;
}

OutlinedButtonThemeData buildOutlinedButtonThemeData({
  required Color bgDisabled,
  required Color bgEnabled,
  required Color fgDisabled,
  required Color fgEnabled,
}) {
  return OutlinedButtonThemeData(
    style:
        OutlinedButton.styleFrom(
          // PHOTOVAULT: theme: Encore control radius.
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(EncoreTokens.radiusControl),
          ),
          alignment: Alignment.center,
          padding: const EdgeInsets.fromLTRB(50, 16, 50, 16),
          textStyle: const TextStyle(
            fontWeight: FontWeight.w600,
            fontFamily: 'Inter-SemiBold',
            fontSize: 18,
          ),
        ).copyWith(
          backgroundColor: WidgetStateProperty.resolveWith<Color>((
            Set<WidgetState> states,
          ) {
            if (states.contains(WidgetState.disabled)) {
              return bgDisabled;
            }
            return bgEnabled;
          }),
          foregroundColor: WidgetStateProperty.resolveWith<Color>((
            Set<WidgetState> states,
          ) {
            if (states.contains(WidgetState.disabled)) {
              return fgDisabled;
            }
            return fgEnabled;
          }),
          alignment: Alignment.center,
        ),
  );
}

ElevatedButtonThemeData buildElevatedButtonThemeData({
  required Color onPrimary,
  required Color primary,
  double elevation = 2,
}) {
  return ElevatedButtonThemeData(
    style: ElevatedButton.styleFrom(
      foregroundColor: onPrimary,
      backgroundColor: primary,
      elevation: elevation,
      alignment: Alignment.center,
      textStyle: const TextStyle(
        fontWeight: FontWeight.w600,
        fontFamily: 'Inter-SemiBold',
        fontSize: 18,
      ),
      padding: const EdgeInsets.symmetric(vertical: 18),
      shape: const RoundedRectangleBorder(
        // PHOTOVAULT: theme: Encore control radius.
        borderRadius: BorderRadius.all(
          Radius.circular(EncoreTokens.radiusControl),
        ),
      ),
    ),
  );
}

SwitchThemeData getSwitchThemeData(Color activeColor) {
  return SwitchThemeData(
    thumbColor: WidgetStateProperty.resolveWith<Color?>((
      Set<WidgetState> states,
    ) {
      if (states.contains(WidgetState.disabled)) {
        return null;
      }
      if (states.contains(WidgetState.selected)) {
        return activeColor;
      }
      return null;
    }),
    trackColor: WidgetStateProperty.resolveWith<Color?>((
      Set<WidgetState> states,
    ) {
      if (states.contains(WidgetState.disabled)) {
        return null;
      }
      if (states.contains(WidgetState.selected)) {
        return activeColor;
      }
      return null;
    }),
  );
}

// PHOTOVAULT: theme: Encore rounded cards.
const CardThemeData _encoreCardTheme = CardThemeData(
  shape: RoundedRectangleBorder(
    borderRadius: BorderRadius.all(Radius.circular(EncoreTokens.radiusCard)),
  ),
);
