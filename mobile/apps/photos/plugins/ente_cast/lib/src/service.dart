import "dart:async";
import "dart:developer" as dev;
import "dart:io";

import "package:ente_cast/src/cast/cast_pairing.dart";
import "package:ente_cast/src/cast/chromecast_channel.dart";
import "package:ente_cast/src/cast/chromecast_client.dart";
import "package:ente_cast/src/cast/chromecast_connection.dart";
import "package:ente_cast/src/cast/chromecast_device.dart";
import "package:ente_cast/src/cast/chromecast_discovery.dart";
import "package:flutter/foundation.dart";

class CastService {
  // PHOTOVAULT: server connection. Upstream launched Ente's registered
  // receiver (app id F5BCEC64), which loads Ente's hosted cast page and pairs
  // through Ente's servers. The receiver is now the app's choice (the server's
  // own receiver, see `castAppId` in /photovault/info); with none, Chromecast
  // auto-pairing is off and casting uses "Pair with code".
  CastService({String? Function()? appID}) : _resolveAppID = appID;

  final String? Function()? _resolveAppID;

  /// The receiver to launch, or null when auto-pairing is unavailable.
  String? get appID {
    final id = _resolveAppID?.call()?.trim();
    return id == null || id.isEmpty ? null : id;
  }

  /// True when a Chromecast can be paired automatically (a receiver is set).
  bool get canAutoPair => isSupported && appID != null;
  static const _pairingTimeout = Duration(seconds: 20);

  final _client = ChromecastClient();
  final _sessions = <String, _CastSession>{};
  final _connectionAttempts = <String, Future<String>>{};

  bool get isSupported => !kIsWeb && (Platform.isAndroid || Platform.isIOS);

  Future<List<(String, Object)>> searchDevices() async {
    if (!isSupported) {
      return [];
    }

    final devices = await const ChromecastDiscovery().search(
      timeout: const Duration(seconds: 7),
    );
    return devices.map((device) => (device.name, device as Object)).toList();
  }

  Future<String> connectDevice(Object device, {int? collectionID}) {
    if (!isSupported) {
      return Future.error(
        UnsupportedError("Cast is only supported on Android and iOS"),
      );
    }
    if (device is! ChromecastDevice) {
      return Future.error(ArgumentError.value(device, "device"));
    }

    final serviceName = device.serviceName;
    if (_connectionAttempts.containsKey(serviceName)) {
      return Future.error(StateError("Already connecting to ${device.name}"));
    }
    if (_sessions.containsKey(serviceName)) {
      return Future.error(StateError("Already connected to ${device.name}"));
    }

    final appID = this.appID; // PHOTOVAULT: server connection
    if (appID == null) {
      return Future.error(
        UnsupportedError("No Cast receiver is configured for this server"),
      );
    }

    late final Future<String> attempt;
    attempt = _connect(device, appID, collectionID: collectionID).whenComplete(
      () {
        if (identical(_connectionAttempts[serviceName], attempt)) {
          _connectionAttempts.remove(serviceName);
        }
      },
    );
    _connectionAttempts[serviceName] = attempt;
    return attempt;
  }

  bool isCastingToDevice(Object device) {
    if (!isSupported || device is! ChromecastDevice) {
      return false;
    }
    return _sessions[device.serviceName]?.receiverSessionID != null;
  }

  Future<void> stopCastingToDevice(Object device) async {
    if (!isSupported || device is! ChromecastDevice) {
      return;
    }

    final session = _sessions[device.serviceName];
    if (session == null) {
      return;
    }
    await _stopSession(session);
  }

  Future<void> closeActiveCasts() async {
    if (!isSupported) {
      return;
    }
    await Future.wait(_sessions.values.toList().map(_stopSession));
  }

  Map<String, String> getActiveSessions() {
    if (!isSupported) {
      return {};
    }

    return {
      for (final session in _sessions.values)
        if (session.receiverSessionID != null)
          session.connection.sourceID: session.connection.state.toString(),
    };
  }

  Future<String> _connect(
    ChromecastDevice device,
    String appID, {
    required int? collectionID,
  }) async {
    final connection = await _client.connect(device);
    final session = _CastSession(device, connection, appID);
    _sessions[device.serviceName] = session;

    session.messageSubscription = connection.messages.listen(
      (message) => _handleMessage(session, message),
      onError: (Object error, StackTrace stackTrace) {
        dev.log(
          "Cast connection failed",
          name: "CastService",
          error: error,
          stackTrace: stackTrace,
        );
        unawaited(_disconnectSession(session));
      },
      cancelOnError: false,
    );
    session.stateSubscription = connection.states.listen((state) {
      if (state == ChromecastConnectionState.closed) {
        dev.log("Session closed", name: "CastService");
        unawaited(_disconnectSession(session));
      }
    });

    try {
      return await waitForPairCode(
        messages: connection.messages,
        states: connection.states,
        start: () => connection.launch(appID),
        sendPairRequest: () => connection.send(castPairRequestNamespace, {
          "collectionID": ?collectionID,
        }),
        timeout: _pairingTimeout,
      );
    } catch (_) {
      await _disconnectSession(session);
      rethrow;
    }
  }

  void _handleMessage(_CastSession session, ChromecastMessage message) {
    if (message.namespace != ChromecastConnection.receiverNamespace ||
        message.payload["type"] != "RECEIVER_STATUS") {
      return;
    }

    final status = message.payload["status"];
    final applications = status is Map ? status["applications"] : null;
    String? receiverSessionID;
    if (applications is List) {
      for (final application in applications) {
        if (application is Map &&
            application["appId"] == session.appID &&
            application["sessionId"] is String) {
          receiverSessionID = application["sessionId"] as String;
          break;
        }
      }
    }

    if (receiverSessionID != null) {
      session.receiverSessionID = receiverSessionID;
    } else if (session.receiverSessionID != null) {
      unawaited(_disconnectSession(session));
    }
  }

  Future<void> _disconnectSession(_CastSession session) {
    return session.closeFuture ??= _closeSession(session);
  }

  Future<void> _stopSession(_CastSession session) async {
    try {
      final receiverSessionID = session.receiverSessionID;
      if (receiverSessionID != null) {
        session.connection.sendToPlatformReceiver(
          ChromecastConnection.receiverNamespace,
          {"type": "STOP", "sessionId": receiverSessionID},
        );
        await session.connection.flush();
      }
    } finally {
      await _disconnectSession(session);
    }
  }

  Future<void> _closeSession(_CastSession session) async {
    if (identical(_sessions[session.device.serviceName], session)) {
      _sessions.remove(session.device.serviceName);
    }
    await session.cancelSubscriptions();
    await _client.disconnect(session.connection);
  }
}

final class _CastSession {
  final ChromecastDevice device;
  final ChromecastConnection connection;
  final String appID; // PHOTOVAULT: server connection
  StreamSubscription<ChromecastMessage>? messageSubscription;
  StreamSubscription<ChromecastConnectionState>? stateSubscription;
  String? receiverSessionID;
  Future<void>? closeFuture;

  _CastSession(this.device, this.connection, this.appID);

  Future<void> cancelSubscriptions() async {
    await messageSubscription?.cancel();
    await stateSubscription?.cancel();
    messageSubscription = null;
    stateSubscription = null;
  }
}
