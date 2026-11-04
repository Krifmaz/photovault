import 'package:ente_components/theme/photovault/encore_tokens.dart';
import 'package:ente_components/theme/text_styles.dart';
import 'package:ente_pure_utils/ente_pure_utils.dart';
import "package:ente_strings/ente_strings.dart";
import "package:ente_ui/components/loading_widget.dart";
import 'package:flutter/material.dart';
import 'package:intl/intl.dart';
import 'package:logging/logging.dart';
import "dart:async";

import 'package:photos/core/constants.dart';
import "package:photos/core/errors.dart";
import "package:photos/core/event_bus.dart";
import "package:photos/events/opened_settings_event.dart";
import 'package:photos/models/user_details.dart';
import 'package:photos/service_locator.dart';
import "package:photos/services/photovault/storage_usage.dart";
import 'package:photos/states/user_details_state.dart';
import 'package:photos/theme/colors.dart';
import 'package:photos/ui/settings/storage_progress_widget.dart';

class StorageCardWidget extends StatefulWidget {
  const StorageCardWidget({super.key});

  /// PHOTOVAULT: test seam for `/photovault/usage` (widget tests have no
  /// network stack). Null = ask the server through [usersGateway].
  @visibleForTesting
  static Future<Map<String, dynamic>?> Function()? usageLoaderForTesting;

  /// PHOTOVAULT: forget the last `/photovault/usage` answer (tests).
  @visibleForTesting
  static void resetForTesting() {
    usageLoaderForTesting = null;
    _StorageCardWidgetState._lastUsage = null;
  }

  @override
  State<StorageCardWidget> createState() => _StorageCardWidgetState();
}

class _DotsPainter extends CustomPainter {
  static const double _dotRadius = 1.45;
  static const double _horizontalSpacing = 15.65;
  static const double _verticalSpacing = 15.65;

  @override
  void paint(Canvas canvas, Size size) {
    final paint = Paint()
      ..color = Colors.white.withValues(alpha: 0.024)
      ..style = PaintingStyle.fill;

    final horizontalCount = (size.width / _horizontalSpacing).ceil() + 1;
    final verticalCount = (size.height / _verticalSpacing).ceil() + 1;

    for (int row = 0; row < verticalCount; row++) {
      for (int col = 0; col < horizontalCount; col++) {
        final x = col * _horizontalSpacing + (_horizontalSpacing / 2);
        final y = row * _verticalSpacing + (_verticalSpacing / 2);

        if (x <= size.width + _dotRadius && y <= size.height + _dotRadius) {
          canvas.drawCircle(Offset(x, y), _dotRadius, paint);
        }
      }
    }
  }

  @override
  bool shouldRepaint(covariant CustomPainter oldDelegate) => false;
}

class _StorageCardWidgetState extends State<StorageCardWidget> {
  final _logger = Logger((_StorageCardWidgetState).toString());
  int? familyMemberStorageLimit;
  bool showFamilyBreakup = false;

  // PHOTOVAULT: real disk. `/photovault/usage` replaces Ente's plan numbers
  // when the server has it; the last answer is kept so reopening Settings
  // does not flash the fallback.
  static PhotovaultStorageUsage? _lastUsage;
  PhotovaultStorageUsage? _usage = _lastUsage;
  StreamSubscription<OpenedSettingsEvent>? _openedSettings;

  @override
  void initState() {
    super.initState();
    unawaited(_loadUsage());
    _openedSettings = Bus.instance.on<OpenedSettingsEvent>().listen(
      (_) => unawaited(_loadUsage()),
    );
  }

  @override
  void dispose() {
    unawaited(_openedSettings?.cancel());
    super.dispose();
  }

  Future<void> _loadUsage() async {
    try {
      final loader = StorageCardWidget.usageLoaderForTesting;
      if (loader == null && isLocalGalleryMode) return;
      final usage = PhotovaultStorageUsage.fromJson(
        await (loader ?? usersGateway.getPhotovaultUsage)(),
      );
      _lastUsage = usage;
      if (mounted) setState(() => _usage = usage);
    } catch (e) {
      _logger.warning("Cannot load /photovault/usage", e);
    }
  }

  @override
  Widget build(BuildContext context) {
    final inheritedUserDetails = InheritedUserDetails.of(context);
    if (inheritedUserDetails == null) {
      final msg = (InheritedUserDetails).toString() + 'is null';
      _logger.severe(msg);
      throw InvalidStateError(msg);
    }
    final userDetails = inheritedUserDetails.userDetails;
    return GestureDetector(
      behavior: HitTestBehavior.translucent,
      onTap: () async {
        final isFamilyMember =
            (userDetails?.isPartOfFamily() ?? false) &&
            !((userDetails?.currentFamilyMember()?.isAdmin) ?? false);
        if (isFamilyMember) {
          await billingService.launchFamilyPortal(
            context,
            userDetails!,
            refreshOnOpen: false,
          );
          return;
        }
        // PHOTOVAULT: payments removed. Ente opened the plan page here; the
        // card now only shows real used/total numbers from the server.
      },
      child: containerForUserDetails(userDetails),
    );
  }

  Widget containerForUserDetails(UserDetails? userDetails) {
    return ClipRRect(
      borderRadius: BorderRadius.circular(15),
      child: Container(
        width: double.infinity,
        constraints: const BoxConstraints(minHeight: 130),
        decoration: BoxDecoration(
          borderRadius: BorderRadius.circular(15),
          gradient: const LinearGradient(
            begin: Alignment.bottomCenter,
            end: Alignment.topCenter,
            colors: [Color(0xFF212121), Color(0xFF434343)],
          ),
        ),
        child: userDetails != null
            ? Stack(
                children: [
                  Positioned.fill(
                    child: CustomPaint(
                      painter: _DotsPainter(),
                      size: Size.infinite,
                    ),
                  ),
                  _userDetails(userDetails),
                ],
              )
            : const SizedBox(
                height: 130,
                child: Stack(
                  children: [EnteLoadingWidget(color: strokeBaseDark)],
                ),
              ),
      ),
    );
  }

  Widget _userDetails(UserDetails userDetails) {
    final usage = _usage;
    if (usage != null) {
      return _diskDetails(StorageCardView.from(usage), userDetails);
    }
    const hundredMBinBytes = 107374182;
    const oneTBinBytes = 1073741824000;
    showFamilyBreakup = userDetails.isPartOfFamily();
    if (showFamilyBreakup) {
      familyMemberStorageLimit = userDetails.familyMemberStorageLimit();
      showFamilyBreakup = familyMemberStorageLimit == null;
    }

    final usedStorageInBytes = familyMemberStorageLimit != null
        ? userDetails.usage
        : userDetails.getFamilyOrPersonalUsage();
    final totalStorageInBytes = familyMemberStorageLimit != null
        ? familyMemberStorageLimit!
        : userDetails.getTotalStorage();
    final freeStorageInBytes = totalStorageInBytes - usedStorageInBytes;

    final isMobileScreenSmall =
        MediaQuery.of(context).size.width <= mobileSmallThreshold;
    final shouldShowUsedStorageInTBs = usedStorageInBytes >= oneTBinBytes;
    final shouldShowTotalStorageInTBs = totalStorageInBytes >= oneTBinBytes;
    final shouldShowUsedStorageInMBs = usedStorageInBytes < hundredMBinBytes;

    final usedStorageInGB = roundBytesUsedToGBs(
      usedStorageInBytes,
      freeStorageInBytes,
    );
    final totalStorageInGB = convertBytesToGBs(totalStorageInBytes).truncate();

    final usedStorageInTB = roundGBsToTBs(usedStorageInGB);
    final totalStorageInTB = roundGBsToTBs(totalStorageInGB);

    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 20, 16, 16),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            isMobileScreenSmall
                ? context.strings.usedSpace
                : context.strings.storage,
            style: TextStyles.tiny.copyWith(
              fontFamily: TextStyles.outfitFontFamily,
              color: textMutedDark,
            ),
          ),
          const SizedBox(height: 2),
          RichText(
            overflow: TextOverflow.ellipsis,
            maxLines: 1,
            text: TextSpan(
              style: TextStyles.display2.copyWith(
                fontWeight: FontWeight.w700,
                color: textBaseDark,
                letterSpacing: -1,
              ),
              children: _usedStorageDetails(
                isMobileScreenSmall: isMobileScreenSmall,
                shouldShowTotalStorageInTBs: shouldShowTotalStorageInTBs,
                shouldShowUsedStorageInTBs: shouldShowUsedStorageInTBs,
                shouldShowUsedStorageInMBs: shouldShowUsedStorageInMBs,
                usedStorageInBytes: usedStorageInBytes,
                usedStorageInGB: usedStorageInGB,
                totalStorageInTB: totalStorageInTB,
                usedStorageInTB: usedStorageInTB,
                totalStorageInGB: totalStorageInGB,
              ),
            ),
          ),
          const SizedBox(height: 16),
          Stack(
            children: <Widget>[
              const StorageProgressWidget(
                color: Color.fromRGBO(193, 193, 193, 0.11),
                fractionOfStorage: 1,
              ),
              showFamilyBreakup
                  ? StorageProgressWidget(
                      color: const Color(0xFFF4D93B),
                      fractionOfStorage:
                          ((usedStorageInBytes) / totalStorageInBytes),
                    )
                  : const SizedBox.shrink(),
              StorageProgressWidget(
                color: encoreLightAccent2, // PHOTOVAULT: theme
                fractionOfStorage: (userDetails.usage / totalStorageInBytes),
              ),
            ],
          ),
          const SizedBox(height: 12),
          Row(
            mainAxisAlignment: MainAxisAlignment.spaceBetween,
            children: [
              showFamilyBreakup
                  ? Row(
                      children: [
                        Container(
                          width: 7,
                          height: 7,
                          decoration: const BoxDecoration(
                            shape: BoxShape.circle,
                            color: encoreLightAccent2, // PHOTOVAULT: theme
                          ),
                        ),
                        const SizedBox(width: 4),
                        Text(
                          context.strings.storageBreakupYou,
                          style: TextStyles.tiny.copyWith(
                            fontFamily: TextStyles.outfitFontFamily,
                            fontWeight: FontWeight.w600,
                            color: textBaseDark,
                          ),
                        ),
                        const SizedBox(width: 12),
                        Container(
                          width: 7,
                          height: 7,
                          decoration: const BoxDecoration(
                            shape: BoxShape.circle,
                            color: Color(0xFFF4D93B),
                          ),
                        ),
                        const SizedBox(width: 4),
                        Text(
                          context.strings.storageBreakupFamily,
                          style: TextStyles.tiny.copyWith(
                            fontFamily: TextStyles.outfitFontFamily,
                            fontWeight: FontWeight.w600,
                            color: textBaseDark,
                          ),
                        ),
                      ],
                    )
                  : const SizedBox.shrink(),
              Text(
                context.strings.memoryCount(
                  count: userDetails.fileCount,
                  formattedCount: NumberFormat().format(userDetails.fileCount),
                ),
                style: TextStyles.tiny.copyWith(
                  fontFamily: TextStyles.outfitFontFamily,
                  fontWeight: FontWeight.w600,
                  color: const Color.fromRGBO(165, 165, 165, 0.79),
                ),
              ),
            ],
          ),
        ],
      ),
    );
  }

  // PHOTOVAULT: the disk that holds the photos, the account's own photos as a
  // second line, and a warning from 90% full (contracts/storage-usage.md).
  Widget _diskDetails(StorageCardView view, UserDetails userDetails) {
    final mutedStyle = TextStyles.tiny.copyWith(
      fontFamily: TextStyles.outfitFontFamily,
      fontWeight: FontWeight.w600,
      color: const Color.fromRGBO(165, 165, 165, 0.79),
    );
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 20, 16, 16),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            context.strings.storage,
            style: TextStyles.tiny.copyWith(
              fontFamily: TextStyles.outfitFontFamily,
              color: textMutedDark,
            ),
          ),
          const SizedBox(height: 2),
          FittedBox(
            fit: BoxFit.scaleDown,
            alignment: Alignment.centerLeft,
            child: Text(
              view.headline,
              key: const ValueKey("photovault-storage-headline"),
              maxLines: 1,
              style: TextStyles.display2.copyWith(
                fontWeight: FontWeight.w700,
                color: textBaseDark,
                letterSpacing: -1,
              ),
            ),
          ),
          const SizedBox(height: 16),
          Stack(
            children: <Widget>[
              const StorageProgressWidget(
                color: Color.fromRGBO(193, 193, 193, 0.11),
                fractionOfStorage: 1,
              ),
              StorageProgressWidget(
                color: view.warning != null
                    ? encoreDarkWarn
                    : const Color.fromRGBO(193, 193, 193, 0.45),
                fractionOfStorage: view.diskFraction,
              ),
              StorageProgressWidget(
                color: encoreLightAccent2, // PHOTOVAULT: theme
                fractionOfStorage: view.userFraction,
              ),
            ],
          ),
          const SizedBox(height: 12),
          Row(
            mainAxisAlignment: MainAxisAlignment.spaceBetween,
            children: [
              Flexible(
                child: Row(
                  children: [
                    Container(
                      width: 7,
                      height: 7,
                      decoration: const BoxDecoration(
                        shape: BoxShape.circle,
                        color: encoreLightAccent2, // PHOTOVAULT: theme
                      ),
                    ),
                    const SizedBox(width: 4),
                    Flexible(
                      child: Text(
                        view.detail,
                        key: const ValueKey("photovault-storage-detail"),
                        overflow: TextOverflow.ellipsis,
                        style: TextStyles.tiny.copyWith(
                          fontFamily: TextStyles.outfitFontFamily,
                          fontWeight: FontWeight.w600,
                          color: textBaseDark,
                        ),
                      ),
                    ),
                  ],
                ),
              ),
              const SizedBox(width: 8),
              Text(
                context.strings.memoryCount(
                  count: userDetails.fileCount,
                  formattedCount: NumberFormat().format(userDetails.fileCount),
                ),
                style: mutedStyle,
              ),
            ],
          ),
          if (view.warning != null) ...[
            const SizedBox(height: 12),
            Row(
              key: const ValueKey("photovault-storage-warning"),
              children: [
                const Icon(
                  Icons.warning_amber_rounded,
                  size: 16,
                  color: encoreDarkWarn,
                ),
                const SizedBox(width: 6),
                Flexible(
                  child: Text(
                    view.warning!,
                    style: TextStyles.tiny.copyWith(
                      fontFamily: TextStyles.outfitFontFamily,
                      fontWeight: FontWeight.w600,
                      color: encoreDarkWarn,
                    ),
                  ),
                ),
              ],
            ),
          ],
        ],
      ),
    );
  }

  List<TextSpan> _usedStorageDetails({
    required bool isMobileScreenSmall,
    required bool shouldShowUsedStorageInTBs,
    required bool shouldShowTotalStorageInTBs,
    required bool shouldShowUsedStorageInMBs,
    required int usedStorageInBytes,
    required num usedStorageInGB,
    required int totalStorageInGB,
    required num usedStorageInTB,
    required num totalStorageInTB,
  }) {
    if (isMobileScreenSmall) {
      return [TextSpan(text: '$usedStorageInGB/$totalStorageInGB GB')];
    }
    late num currentUsage, totalStorage;
    late String currentUsageUnit, totalStorageUnit;

    if (shouldShowUsedStorageInTBs) {
      currentUsage = usedStorageInTB;
      currentUsageUnit = "TB";
    } else if (shouldShowUsedStorageInMBs) {
      currentUsage = convertBytesToMBs(usedStorageInBytes);
      currentUsageUnit = "MB";
    } else {
      currentUsage = usedStorageInGB;
      currentUsageUnit = "GB";
    }

    if (shouldShowTotalStorageInTBs) {
      totalStorage = totalStorageInTB;
      totalStorageUnit = "TB";
    } else {
      totalStorage = totalStorageInGB;
      totalStorageUnit = "GB";
    }

    return [
      TextSpan(text: '$currentUsage $currentUsageUnit  '),
      const TextSpan(
        text: 'of',
        style: TextStyle(color: textMutedDark),
      ),
      TextSpan(text: '  $totalStorage $totalStorageUnit used'),
    ];
  }
}
