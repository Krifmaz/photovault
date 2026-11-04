import "package:ente_components/theme/photovault/encore_tokens.dart";
import "package:flutter/material.dart";
import "package:photos/service_locator.dart";
import "package:photos/services/wrapped/models.dart";
import "package:photos/services/wrapped/wrapped_service.dart";
import "package:photos/theme/ente_theme.dart";
import "package:photos/ui/notification/toast.dart";
import "package:photos/ui/wrapped/wrapped_viewer_page.dart";

class WrappedRewindBannerButton extends StatefulWidget {
  const WrappedRewindBannerButton({
    super.key,
    this.height,
    this.semanticsLabel,
  });

  final double? height;
  final String? semanticsLabel;

  @override
  State<WrappedRewindBannerButton> createState() =>
      _WrappedRewindBannerButtonState();
}

class _WrappedRewindBannerButtonState extends State<WrappedRewindBannerButton> {
  // PHOTOVAULT: branding. Ente's animated Rive banner background
  // ("ente_rewind_banner.riv", which also set the height from its artboard)
  // is replaced by a plain panel colour at the default height.
  @override
  Widget build(BuildContext context) {
    final textTheme = getEnteTextTheme(context);
    const BorderRadius borderRadius = BorderRadius.all(Radius.circular(24));

    return Semantics(
      button: true,
      label: widget.semanticsLabel,
      child: Material(
        color: Colors.transparent,
        borderRadius: borderRadius,
        child: InkWell(
          borderRadius: borderRadius,
          onTap: () => _handleTap(context),
          child: LayoutBuilder(
            builder: (BuildContext context, BoxConstraints constraints) {
              final double resolvedHeight = widget.height ?? 128;

              return ClipRRect(
                borderRadius: borderRadius,
                child: SizedBox(
                  height: resolvedHeight,
                  width: double.infinity,
                  child: Stack(
                    alignment: Alignment.center,
                    children: [
                      const Positioned.fill(
                        child: ColoredBox(color: encoreLightAccent),
                      ),
                      Container(
                        padding: const EdgeInsets.symmetric(
                          horizontal: 20,
                          vertical: 16,
                        ),
                        child: Row(
                          mainAxisAlignment: MainAxisAlignment.center,
                          children: [
                            Flexible(
                              child: Text(
                                "Rewind 2025", // PHOTOVAULT: branding
                                style: textTheme.h3Bold.copyWith(
                                  fontSize:
                                      (textTheme.h3Bold.fontSize ?? 24) + 2,
                                  color: Colors.white,
                                ),
                                textAlign: TextAlign.center,
                              ),
                            ),
                            const SizedBox(width: 12),
                            Container(
                              width: 28,
                              height: 28,
                              decoration: const BoxDecoration(
                                color: Color(0x99000000),
                                shape: BoxShape.circle,
                              ),
                              child: const Icon(
                                Icons.play_arrow_rounded,
                                color: Colors.white,
                                size: 18,
                              ),
                            ),
                          ],
                        ),
                      ),
                    ],
                  ),
                ),
              );
            },
          ),
        ),
      ),
    );
  }

  void _handleTap(BuildContext context) {
    final WrappedEntryState currentState = wrappedService.state;
    final WrappedResult? result = currentState.result;
    if (result == null || result.cards.isEmpty) {
      // PHOTOVAULT: branding
      showShortToast(context, "Rewind isn't ready yet");
      return;
    }
    Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (BuildContext context) =>
            WrappedViewerPage(initialState: currentState),
      ),
    );
  }
}
