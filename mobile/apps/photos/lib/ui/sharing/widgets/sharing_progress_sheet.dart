import "dart:async";

import "package:ente_components/ente_components.dart";
import "package:ente_strings/ente_strings.dart";
import "package:flutter/material.dart";
import "package:photos/ui/photovault/photovault_glyph.dart";
import "package:photos/utils/dialog_util.dart";

Future<bool> showSharingProgressSheet(
  BuildContext context, {
  required Future<bool> Function() task,
}) async {
  Object? taskError;
  final result = await showBottomSheetComponent<bool>(
    context: context,
    isDismissible: false,
    enableDrag: false,
    builder: (sheetContext) => _SharingProgressSheet(
      task: () async {
        try {
          return await task();
        } catch (error) {
          taskError = error;
          return false;
        }
      },
    ),
  );
  if (taskError != null && context.mounted) {
    await showGenericErrorDialog(context: context, error: taskError);
  }
  return result ?? false;
}

class _SharingProgressSheet extends StatefulWidget {
  const _SharingProgressSheet({required this.task});

  final Future<bool> Function() task;

  @override
  State<_SharingProgressSheet> createState() => _SharingProgressSheetState();
}

class _SharingProgressSheetState extends State<_SharingProgressSheet> {
  // PHOTOVAULT: branding. Ente's Rive duck ("invite.riv", which closed the
  // sheet from its success event) is replaced by a static glyph; the sheet
  // now closes as soon as sharing succeeds.
  @override
  void initState() {
    super.initState();
    unawaited(_runTask());
  }

  @override
  Widget build(BuildContext context) {
    return BottomSheetComponent(
      showCloseButton: false,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      padding: const EdgeInsets.fromLTRB(Spacing.xl, 51, Spacing.xl, 44),
      content: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          SizedBox.square(
            dimension: 143,
            child: const ExcludeSemantics(
              child: PhotovaultGlyph(width: 143, height: 143),
            ),
          ),
          const SizedBox(height: 30),
          Text(
            context.strings.sharing,
            textAlign: TextAlign.center,
            style: TextStyles.display2.copyWith(
              color: context.componentColors.textBase,
            ),
          ),
        ],
      ),
    );
  }

  Future<void> _runTask() async {
    final succeeded = await widget.task();
    if (!mounted) {
      return;
    }
    if (!succeeded) {
      Navigator.of(context).pop(false);
      return;
    }
    await Future<void>.delayed(const Duration(milliseconds: 250));
    if (mounted) {
      Navigator.of(context).pop(true);
    }
  }


}
