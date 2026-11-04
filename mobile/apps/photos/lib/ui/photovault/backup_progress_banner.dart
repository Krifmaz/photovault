// PHOTOVAULT: backup progress on the Photos screen and the Backup status
// screen. Same text as the notification (BackupProgressStrings).
import "package:ente_components/ente_components.dart";
import "package:flutter/foundation.dart";
import "package:flutter/material.dart";
import "package:flutter/services.dart";
import "package:hugeicons/hugeicons.dart";
import "package:photos/services/photovault/backup_progress/backup_progress_service.dart";
import "package:photos/services/photovault/backup_progress/backup_progress_state.dart";

/// Banner at the top of the Photos screen while a backup runs or is paused.
class BackupProgressBanner extends StatelessWidget {
  const BackupProgressBanner({
    super.key,
    required this.progress,
    required this.onTap,
    this.onPause,
    this.onResume,
  });

  final ValueListenable<BackupProgressSnapshot> progress;
  final VoidCallback onTap;
  final VoidCallback? onPause;
  final VoidCallback? onResume;

  @override
  Widget build(BuildContext context) {
    return ValueListenableBuilder<BackupProgressSnapshot>(
      valueListenable: progress,
      builder: (context, s, _) {
        if (!s.isVisible) return const SizedBox.shrink();
        return Padding(
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 6),
          child: _BackupProgressCard(
            key: const ValueKey("backup-progress-banner"),
            snapshot: s,
            onTap: onTap,
            onPause: onPause,
            onResume: onResume,
          ),
        );
      },
    );
  }
}

class _BackupProgressCard extends StatelessWidget {
  const _BackupProgressCard({
    super.key,
    required this.snapshot,
    this.onTap,
    this.onPause,
    this.onResume,
  });

  final BackupProgressSnapshot snapshot;
  final VoidCallback? onTap;
  final VoidCallback? onPause;
  final VoidCallback? onResume;

  @override
  Widget build(BuildContext context) {
    final colors = context.componentColors;
    final s = snapshot;
    final failed =
        s.phase == BackupPhase.cantReachServer || s.phase == BackupPhase.error;
    final accent = failed ? colors.warning : colors.primaryDark;
    final file = BackupProgressStrings.text(s);
    final detail = s.isUploading
        ? BackupProgressStrings.speedAndTimeLeft(s)
        : null;
    final icon = switch (s.phase) {
      BackupPhase.complete => HugeIcons.strokeRoundedCheckmarkCircle02,
      BackupPhase.cantReachServer ||
      BackupPhase.error => HugeIcons.strokeRoundedAlertCircle,
      BackupPhase.pausedByUser ||
      BackupPhase.waitingForWifi => HugeIcons.strokeRoundedPause,
      _ => HugeIcons.strokeRoundedCloudUpload,
    };
    final Widget? action = s.isUploading && onPause != null
        ? IconButton(
            key: const ValueKey("backup-progress-pause"),
            tooltip: BackupProgressStrings.pause,
            onPressed: onPause,
            icon: const Icon(Icons.pause_rounded),
          )
        : s.isPaused && onResume != null
        ? IconButton(
            key: const ValueKey("backup-progress-resume"),
            tooltip: failed
                ? BackupProgressStrings.retry
                : BackupProgressStrings.resume,
            onPressed: onResume,
            icon: Icon(failed ? Icons.refresh_rounded : Icons.play_arrow_rounded),
          )
        : null;

    return Semantics(
      button: onTap != null,
      label: BackupProgressStrings.title(s),
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: onTap,
        child: DecoratedBox(
          decoration: BoxDecoration(
            color: colors.fillLight,
            borderRadius: BorderRadius.circular(Radii.button),
          ),
          child: Padding(
            padding: const EdgeInsets.fromLTRB(
              Spacing.lg,
              Spacing.md,
              Spacing.sm,
              Spacing.md,
            ),
            child: Row(
              children: [
                HugeIcon(icon: icon, size: IconSizes.medium, color: accent),
                const SizedBox(width: Spacing.lg),
                Expanded(
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        BackupProgressStrings.title(s),
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyles.bodyBold.copyWith(
                          color: failed ? colors.warning : colors.textBase,
                        ),
                      ),
                      if (s.isUploading) ...[
                        const SizedBox(height: Spacing.sm),
                        ClipRRect(
                          borderRadius: BorderRadius.circular(2),
                          child: LinearProgressIndicator(
                            key: const ValueKey("backup-progress-bar"),
                            value: s.fraction,
                            minHeight: 4,
                            color: colors.primary,
                            backgroundColor: colors.fillDark,
                          ),
                        ),
                      ],
                      if (file != null) ...[
                        const SizedBox(height: Spacing.xs),
                        Text(
                          file,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: TextStyles.mini.copyWith(
                            color: colors.textLight,
                          ),
                        ),
                      ],
                      if (detail != null)
                        Text(
                          detail,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: TextStyles.mini.copyWith(
                            color: colors.textLight,
                          ),
                        ),
                    ],
                  ),
                ),
                if (action != null)
                  IconTheme.merge(
                    data: IconThemeData(color: colors.textBase),
                    child: action,
                  )
                else
                  const SizedBox(width: Spacing.sm),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// Top of the Backup status screen: progress, Pause/Resume, Wi-Fi only and
/// the copyable error details.
class BackupProgressPanel extends StatelessWidget {
  const BackupProgressPanel({super.key, this.service});

  final BackupProgressService? service;

  @override
  Widget build(BuildContext context) {
    final service = this.service ?? BackupProgressService.instance;
    final colors = context.componentColors;
    return ValueListenableBuilder<BackupProgressSnapshot>(
      valueListenable: service.progress,
      builder: (context, s, _) {
        if (!s.isVisible) return const SizedBox.shrink();
        final failed =
            s.phase == BackupPhase.cantReachServer ||
            s.phase == BackupPhase.error;
        final report = failed
            ? BackupProgressStrings.errorReport(
                s,
                service.lastErrorAt ?? DateTime.now(),
              )
            : null;
        return Padding(
          padding: const EdgeInsets.only(bottom: Spacing.lg),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              _BackupProgressCard(
                snapshot: s,
                onPause: service.pause,
                onResume: service.resume,
              ),
              if (s.phase != BackupPhase.complete)
                SwitchListTile.adaptive(
                  key: const ValueKey("backup-progress-wifi-only"),
                  contentPadding: const EdgeInsets.symmetric(
                    horizontal: Spacing.sm,
                  ),
                  title: Text(
                    BackupProgressStrings.wifiOnly,
                    style: TextStyles.body.copyWith(color: colors.textBase),
                  ),
                  value: s.wifiOnly,
                  onChanged: (_) => service.toggleWifiOnly(),
                ),
              if (report != null) ...[
                Container(
                  key: const ValueKey("backup-progress-error-details"),
                  padding: const EdgeInsets.all(Spacing.md),
                  decoration: BoxDecoration(
                    color: colors.fillLight,
                    borderRadius: BorderRadius.circular(Radii.button),
                  ),
                  child: SelectableText(
                    report,
                    style: TextStyles.mini.copyWith(
                      color: colors.textLight,
                      fontFamily: "monospace",
                    ),
                  ),
                ),
                Align(
                  alignment: Alignment.centerLeft,
                  child: TextButton.icon(
                    key: const ValueKey("backup-progress-copy-details"),
                    onPressed: () async {
                      await Clipboard.setData(ClipboardData(text: report));
                      if (!context.mounted) return;
                      ScaffoldMessenger.maybeOf(context)?.showSnackBar(
                        const SnackBar(
                          content: Text(BackupProgressStrings.copied),
                        ),
                      );
                    },
                    icon: const Icon(Icons.copy_rounded, size: 18),
                    label: const Text(BackupProgressStrings.copyDetails),
                  ),
                ),
              ],
            ],
          ),
        );
      },
    );
  }
}
