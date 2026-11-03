// PHOTOVAULT: de-Google. Firebase Cloud Messaging is removed, so push-driven
// sync simply does not happen; background sync still runs through
// WorkManager/BGTaskScheduler as before. This is Ente's own F-Droid stub
// (formerly fdroid/overlay/lib/services/push_service.dart), kept with the same
// API so main.dart is unchanged.
typedef BackgroundPushHandler = Future<void> Function(Object message);

class PushService {
  static final PushService instance = PushService._privateConstructor();

  PushService._privateConstructor();

  Future<void> init({BackgroundPushHandler? onBackgroundPush}) async {}

  static bool shouldSync(Object message) => false;
}
