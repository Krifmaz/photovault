// PHOTOVAULT: tree-specific setup for before_after_test.dart on the pristine
// Ente import. scripts/photovault/render_before_after.sh copies this file
// over tree_env.dart in the upstream worktree. Ente has no server step.

import "package:photos/core/error-reporting/super_logging.dart";
import "package:shared_preferences/shared_preferences.dart";

const treeName = "upstream";

Future<void> prepareTree(SharedPreferences prefs) async {
  // Ente's Security page reads the crash-reporting preference that
  // SuperLogging.main sets up at app start (debug config: no Sentry, no
  // log files).
  try {
    await SuperLogging.main(LogConfig());
  } catch (_) {}
}
