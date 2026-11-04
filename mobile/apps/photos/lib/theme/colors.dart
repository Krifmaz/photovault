import 'dart:ui';

// PHOTOVAULT: theme. Colour values come from the Encore tokens.
import 'package:ente_components/theme/photovault/encore_tokens.dart';
import 'package:flutter/material.dart';

class EnteColorScheme {
  final Color backgroundColour;
  final Color backgroundElevated;
  final Color backgroundElevated2;

  final Color backdropBase;
  final Color backdropMuted;
  final Color backdropFaint;

  final Color textBase;
  final Color textMuted;
  final Color textFaint;
  final Color blurTextBase;

  final Color fillBase;
  final Color fillBasePressed;
  final Color fillStrong;
  final Color fillMuted;
  final Color fillFaint;
  final Color fillFaintPressed;
  final Color fillBaseGrey;

  final Color strokeBase;
  final Color strokeMuted;
  final Color strokeFaint;
  final Color strokeFainter;
  final Color blurStrokeBase;
  final Color blurStrokeFaint;
  final Color blurStrokePressed;

  final Color primary700;
  final Color primary500;
  final Color primary400;
  final Color primary300;

  final Color warning700;
  final Color warning500;
  final Color warning400;
  final Color warning800;
  final Color caution500;

  final Color golden700;
  final Color golden500;

  final Color tabIcon;

  final Color menuItemIconStroke;

  final Color fill;
  final Color fillDark;
  final Color fillDarker;
  final Color fillDarkest;
  final Color fillReverse;

  final Color content;
  final Color contentDark;
  final Color contentDarker;
  final Color contentLight;
  final Color contentLighter;
  final Color contentLightest;
  final Color contentReverse;

  final Color strokeSolid;
  final Color strokeDark;

  final Color greenBase;
  final Color greenDark;
  final Color greenDarker;
  final Color greenLight;
  final Color greenStroke;

  final Color redBase;
  final Color redDark;
  final Color redDarker;
  final Color redLight;

  /// PHOTOVAULT: theme. Foreground on [greenBase] (the primary accent).
  final Color onAccent;

  const EnteColorScheme(
    this.backgroundColour,
    this.backgroundElevated,
    this.backgroundElevated2,
    this.backdropBase,
    this.backdropMuted,
    this.backdropFaint,
    this.textBase,
    this.textMuted,
    this.textFaint,
    this.blurTextBase,
    this.fillBase,
    this.fillBasePressed,
    this.fillStrong,
    this.fillMuted,
    this.fillFaint,
    this.fillFaintPressed,
    this.fillBaseGrey,
    this.strokeBase,
    this.strokeMuted,
    this.strokeFaint,
    this.strokeFainter,
    this.blurStrokeBase,
    this.blurStrokeFaint,
    this.blurStrokePressed,
    this.tabIcon,
    this.menuItemIconStroke,
    this.fill,
    this.fillDark,
    this.fillDarker,
    this.fillDarkest,
    this.fillReverse,
    this.content,
    this.contentDark,
    this.contentDarker,
    this.contentLight,
    this.contentLighter,
    this.contentLightest,
    this.contentReverse,
    this.strokeSolid,
    this.strokeDark,
    this.greenBase,
    this.greenDark,
    this.greenDarker,
    this.greenLight,
    this.greenStroke,
    this.redBase,
    this.redDark,
    this.redDarker,
    this.redLight, {
    this.primary700 = _primary700,
    this.primary500 = _primary500,
    this.primary400 = _primary400,
    this.primary300 = _primary300,
    this.warning800 = _warning800,
    this.warning700 = _warning700,
    this.warning500 = _warning500,
    this.warning400 = _warning400,
    this.caution500 = _caution500,
    this.golden700 = _golden700,
    this.golden500 = _golden500,
    this.onAccent = encoreDarkOnAccent, // PHOTOVAULT: theme
  });
}

const EnteColorScheme lightScheme = EnteColorScheme(
  backgroundColorLight,
  backgroundElevatedLight,
  backgroundElevated2Light,
  backdropBaseLight,
  backdropMutedLight,
  backdropFaintLight,
  textBaseLight,
  textMutedLight,
  textFaintLight,
  blurTextBaseLight,
  fillBaseLight,
  fillBasePressedLight,
  fillStrongLight,
  fillMutedLight,
  fillFaintLight,
  fillFaintPressedLight,
  fillBaseGreyLight,
  strokeBaseLight,
  strokeMutedLight,
  strokeFaintLight,
  strokeFainterLight,
  blurStrokeBaseLight,
  blurStrokeFaintLight,
  blurStrokePressedLight,
  tabIconLight,
  menuItemIconStrokeLight,
  fillLight,
  fillDarkLight,
  fillDarkerLight,
  fillDarkestLight,
  fillReverseLight,
  contentLight,
  contentDarkLight,
  contentDarkerLight,
  contentLightLight,
  contentLighterLight,
  contentLightestLight,
  contentReverseLight,
  strokeLight,
  strokeDarkLight,
  // PHOTOVAULT: theme. Ente's brand green -> Encore accent (light).
  encoreLightAccent,
  encoreLightAccent2,
  encoreLightAccentDarker,
  greenLightLight,
  greenStrokeLight,
  red,
  redDark,
  redDarker,
  redLightLight,
  primary700: encoreLightAccentDarker,
  primary500: encoreLightAccent2,
  primary400: encoreLightAccent2,
  primary300: encoreLightAccent2,
  onAccent: encoreLightOnAccent,
);

const EnteColorScheme darkScheme = EnteColorScheme(
  backgroundColorDark,
  backgroundElevatedDark,
  backgroundElevated2Dark,
  backdropBaseDark,
  backdropMutedDark,
  backdropFaintDark,
  textBaseDark,
  textMutedDark,
  textFaintDark,
  blurTextBaseDark,
  fillBaseDark,
  fillBasePressedDark,
  fillStrongDark,
  fillMutedDark,
  fillFaintDark,
  fillFaintPressedDark,
  fillBaseGreyDark,
  strokeBaseDark,
  strokeMutedDark,
  strokeFaintDark,
  strokeFainterDark,
  blurStrokeBaseDark,
  blurStrokeFaintDark,
  blurStrokePressedDark,
  tabIconDark,
  menuItemIconStrokeDark,
  fillDark,
  fillDarkDark,
  fillDarkerDark,
  fillDarkestDark,
  fillReverseDark,
  contentDark,
  contentDarkDark,
  contentDarkerDark,
  contentLightDark,
  contentLighterDark,
  contentLightestDark,
  contentReverseDark,
  strokeDark,
  strokeDarkDark,
  // PHOTOVAULT: theme. Ente's brand green -> Encore accent (dark).
  encoreDarkAccent,
  encoreDarkAccent2,
  encoreDarkAccentDarker,
  greenLightDark,
  greenStrokeDark,
  red,
  redDark,
  redDarker,
  redLightDark,
  primary700: encoreDarkAccentDarker,
  primary500: encoreDarkAccent2,
  primary400: encoreDarkAccent2,
  primary300: encoreDarkAccent,
  onAccent: encoreDarkOnAccent,
);

const Color backgroundColorLight = encoreLightBg; // PHOTOVAULT: theme
const Color backgroundColorDark = encoreDarkBg; // PHOTOVAULT: theme

const Color backgroundElevatedLight = encoreLightPanel; // PHOTOVAULT: theme
const Color backgroundElevated2Light = encoreLightBg2; // PHOTOVAULT: theme

const Color backgroundElevatedDark = encoreDarkBg2; // PHOTOVAULT: theme
const Color backgroundElevated2Dark = encoreDarkPanel; // PHOTOVAULT: theme

const Color backdropBaseLight = Color.fromRGBO(255, 255, 255, 0.92);
const Color backdropMutedLight = Color.fromRGBO(255, 255, 255, 0.75);
const Color backdropFaintLight = Color.fromRGBO(255, 255, 255, 0.30);

const Color backdropBaseDark = Color.fromRGBO(0, 0, 0, 0.90);
const Color backdropMutedDark = Color.fromRGBO(0, 0, 0, 0.65);
const Color backdropFaintDark = Color.fromRGBO(0, 0, 0, 0.20);

const Color textBaseLight = encoreLightText; // PHOTOVAULT: theme
const Color textMutedLight = encoreLightMuted; // PHOTOVAULT: theme
const Color textFaintLight = encoreLightFaint; // PHOTOVAULT: theme
const Color blurTextBaseLight = Color.fromRGBO(0, 0, 0, 0.65);

const Color textBaseDark = encoreDarkText; // PHOTOVAULT: theme
const Color textMutedDark = encoreDarkMuted; // PHOTOVAULT: theme
const Color textFaintDark = encoreDarkFaint; // PHOTOVAULT: theme
const Color blurTextBaseDark = Color.fromRGBO(255, 255, 255, 0.95);

const Color fillBaseLight = encoreLightText; // PHOTOVAULT: theme
const Color fillBasePressedLight = Color.fromRGBO(0, 0, 0, 0.87);
const Color fillStrongLight = Color.fromRGBO(0, 0, 0, 0.24);
const Color fillMutedLight = Color.fromRGBO(0, 0, 0, 0.12);
const Color fillFaintLight = Color.fromRGBO(0, 0, 0, 0.04);
const Color fillFaintPressedLight = Color.fromRGBO(0, 0, 0, 0.08);
const Color fillBaseGreyLight = Color.fromRGBO(242, 242, 242, 1);

const Color fillBaseDark = encoreDarkText; // PHOTOVAULT: theme
const Color fillBasePressedDark = Color.fromRGBO(255, 255, 255, 0.9);
const Color fillStrongDark = Color.fromRGBO(255, 255, 255, 0.32);
const Color fillMutedDark = Color.fromRGBO(255, 255, 255, 0.16);
const Color fillFaintDark = Color.fromRGBO(255, 255, 255, 0.12);
const Color fillFaintPressedDark = Color.fromRGBO(255, 255, 255, 0.06);
const Color fillBaseGreyDark = Color.fromRGBO(66, 66, 66, 1);

const Color strokeBaseLight = encoreLightText; // PHOTOVAULT: theme
const Color strokeMutedLight = Color.fromRGBO(0, 0, 0, 0.24);
const Color strokeFaintLight = Color.fromRGBO(0, 0, 0, 0.12);
const Color strokeFainterLight = Color.fromRGBO(0, 0, 0, 0.06);
const Color blurStrokeBaseLight = Color.fromRGBO(0, 0, 0, 0.65);
const Color blurStrokeFaintLight = Color.fromRGBO(0, 0, 0, 0.08);
const Color blurStrokePressedLight = Color.fromRGBO(0, 0, 0, 0.50);

const Color strokeBaseDark = encoreDarkText; // PHOTOVAULT: theme
const Color strokeMutedDark = Color.fromRGBO(255, 255, 255, 0.24);
const Color strokeFaintDark = Color.fromRGBO(255, 255, 255, 0.16);
const Color strokeFainterDark = Color.fromRGBO(255, 255, 255, 0.08);
const Color blurStrokeBaseDark = Color.fromRGBO(255, 255, 255, 0.90);
const Color blurStrokeFaintDark = Color.fromRGBO(255, 255, 255, 0.06);
const Color blurStrokePressedDark = Color.fromRGBO(255, 255, 255, 0.50);

const Color tabIconLight = Color.fromRGBO(0, 0, 0, 0.85);
const Color tabIconDark = Color.fromRGBO(255, 255, 255, 0.80);

const Color menuItemIconStrokeLight = Color(0xFF979797);
const Color menuItemIconStrokeDark = encoreDarkText; // PHOTOVAULT: theme

const Color fixedStrokeMutedWhite = Color.fromRGBO(255, 255, 255, 0.50);
const Color strokeSolidMutedLight = Color.fromRGBO(147, 147, 147, 1);
const Color strokeSolidFaintLight = Color.fromRGBO(221, 221, 221, 1);

// Keep light for scanability.
const Color qrBoxColor = Color.fromRGBO(245, 245, 247, 1);

const Color _primary700 = encoreDarkAccentDarker; // PHOTOVAULT: theme
const Color _primary500 = encoreDarkAccent2; // PHOTOVAULT: theme
const Color _primary400 = encoreDarkAccent2; // PHOTOVAULT: theme
const Color _primary300 = encoreDarkAccent; // PHOTOVAULT: theme

const Color _warning700 = encoreDarkErrDark; // PHOTOVAULT: theme
const Color _warning500 = encoreDarkErr; // PHOTOVAULT: theme
const Color warning500 = encoreDarkErr; // PHOTOVAULT: theme
const Color _warning400 = encoreDarkErr; // PHOTOVAULT: theme
const Color _warning800 = encoreDarkErrDarker; // PHOTOVAULT: theme

const Color _caution500 = encoreDarkWarn; // PHOTOVAULT: theme

const Color _golden700 = Color(0xFFFDB816);
const Color _golden500 = Color(0xFFFFC336);

const Color green = encoreDarkAccent; // PHOTOVAULT: theme
const Color greenLightLight = encoreLightPanel2; // PHOTOVAULT: theme
const Color greenLightDark = encoreDarkPanel2; // PHOTOVAULT: theme
const Color greenDark = encoreDarkAccent2; // PHOTOVAULT: theme
const Color greenDarker = encoreDarkAccentDarker; // PHOTOVAULT: theme
const Color greenStrokeLight = encoreLightAccentStroke; // PHOTOVAULT: theme
const Color greenStrokeDark = encoreDarkAccentStroke; // PHOTOVAULT: theme

const Color red = encoreDarkErr; // PHOTOVAULT: theme
const Color redLightLight = encoreLightErrTint; // PHOTOVAULT: theme
const Color redLightDark = encoreDarkErrTint; // PHOTOVAULT: theme
const Color redDark = encoreDarkErrDark; // PHOTOVAULT: theme
const Color redDarker = encoreDarkErrDarker; // PHOTOVAULT: theme

const Color fillLight = encoreLightPanel; // PHOTOVAULT: theme
const Color fillDark = encoreDarkPanel; // PHOTOVAULT: theme

const Color fillDarkLight = encoreLightPanel2; // PHOTOVAULT: theme
const Color fillDarkDark = Color.fromRGBO(10, 10, 10, 1);

const Color fillDarkerLight = encoreLightBorder; // PHOTOVAULT: theme
const Color fillDarkerDark = Color.fromRGBO(20, 20, 20, 1);

const Color fillDarkestLight = Color.fromRGBO(210, 210, 210, 1);
const Color fillDarkestDark = encoreDarkPanel2; // PHOTOVAULT: theme

const Color fillReverseLight = encoreLightText; // PHOTOVAULT: theme
const Color fillReverseDark = encoreDarkText; // PHOTOVAULT: theme

const Color contentLight = encoreLightText; // PHOTOVAULT: theme
const Color contentDark = encoreDarkText; // PHOTOVAULT: theme

const Color contentDarkLight = encoreLightText; // PHOTOVAULT: theme
const Color contentDarkDark = encoreDarkText; // PHOTOVAULT: theme

const Color contentDarkerLight = encoreLightText; // PHOTOVAULT: theme
const Color contentDarkerDark = encoreDarkText; // PHOTOVAULT: theme

const Color contentLightLight = encoreLightMuted; // PHOTOVAULT: theme
const Color contentLightDark = encoreDarkMuted; // PHOTOVAULT: theme

const Color contentLighterLight = encoreLightFaint; // PHOTOVAULT: theme
const Color contentLighterDark = encoreDarkFaint; // PHOTOVAULT: theme

const Color contentLightestLight = Color.fromRGBO(222, 222, 222, 1);
const Color contentLightestDark = Color.fromRGBO(10, 10, 10, 1);

const Color contentReverseLight = Color.fromRGBO(255, 255, 255, 1);
const Color contentReverseDark = encoreDarkOnAccent; // PHOTOVAULT: theme

const Color strokeLight = encoreLightBorder; // PHOTOVAULT: theme
const Color strokeDark = Color.fromRGBO(20, 20, 20, 1);

const Color strokeDarkLight = encoreLightBorder; // PHOTOVAULT: theme
const Color strokeDarkDark = encoreDarkBorder; // PHOTOVAULT: theme
