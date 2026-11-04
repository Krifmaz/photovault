// PHOTOVAULT: server connection (new file).
//
// Saved servers, the active server and its current address. The address
// goes into Ente's own endpoint setting (`EndpointConfig.setEndpoint`), so
// every API call keeps Ente's paths and only the base URL changes.
//
// Persistence: SharedPreferences keys starting with `photovault.` plus Ente's
// `endpoint` key. `Configuration.logout` keeps them (see [preserveOnLogout]),
// so servers survive logging out.

import "dart:async";
import "dart:convert";

import "package:connectivity_plus/connectivity_plus.dart";
import "package:flutter/foundation.dart";
import "package:logging/logging.dart";
import "package:photos/core/network/endpoint_config.dart";
import "package:photos/services/photovault/server_address.dart";
import "package:photos/services/photovault/server_address_selector.dart";
import "package:photos/services/photovault/server_contract.dart";
import "package:photos/services/photovault/server_discovery.dart";
import "package:photos/services/photovault/server_info_client.dart";
import "package:shared_preferences/shared_preferences.dart";

class PhotovaultServerService {
  PhotovaultServerService._();

  static final PhotovaultServerService instance = PhotovaultServerService._();

  static const _keyPrefix = "photovault.";
  static const _serversKey = "${_keyPrefix}servers";
  static const _activeKey = "${_keyPrefix}activeServer";

  /// Optional label for the built-in server (`contracts/apps.json`).
  static const builtInServerName = String.fromEnvironment(
    "PHOTOVAULT_SERVER_NAME",
  );

  final _logger = Logger("PhotovaultServerService");
  final _client = PhotovaultServerInfoClient();
  late final _selector = ServerAddressSelector(probe: _client.probe);
  final discovery = const PhotovaultServerDiscovery();

  SharedPreferences? _prefs;
  EndpointConfig? _endpointConfig;
  StreamSubscription<List<ConnectivityResult>>? _connectivitySubscription;
  Future<void>? _refreshInFlight;

  /// The server the app is set up for, or null before "Connect".
  final ValueNotifier<PhotovaultServerInfo?> activeServer = ValueNotifier(null);

  void init(
    SharedPreferences prefs,
    EndpointConfig endpointConfig, {
    bool watchNetwork = true,
  }) {
    _prefs = prefs;
    _endpointConfig = endpointConfig;
    activeServer.value = _readActive();
    if (watchNetwork) {
      unawaited(_connectivitySubscription?.cancel());
      _connectivitySubscription = Connectivity().onConnectivityChanged.listen(
        (_) => unawaited(refreshAddress()),
      );
    }
  }

  /// The build-time server (`--dart-define=PHOTOVAULT_SERVER=...`), if any.
  String? get builtInAddress =>
      tryNormalizeServerAddress(EndpointConfig.defaultEndpoint);

  /// The base URL requests go to right now.
  String get currentAddress => _endpointConfig?.endpoint ?? "";

  List<PhotovaultServerInfo> get savedServers {
    final raw = _prefs?.getString(_serversKey);
    if (raw == null) return [];
    try {
      final list = jsonDecode(raw);
      if (list is! List) return [];
      return list
          .map(PhotovaultServerInfo.fromStoredJson)
          .whereType<PhotovaultServerInfo>()
          .toList();
    } catch (e) {
      _logger.warning("Ignoring unreadable saved servers", e);
      return [];
    }
  }

  /// Normalises [input] and asks the server what it is.
  /// Throws [ServerConnectException].
  Future<PhotovaultServerInfo> fetchInfo(String input) async {
    final String address;
    try {
      address = normalizeServerAddress(input);
    } on InvalidServerAddressException {
      throw ServerConnectException(
        ServerConnectErrorKind.invalidAddress,
        input.trim(),
      );
    }
    return _client.fetch(address);
  }

  /// Saves [info], makes it the active server and points Ente's endpoint at
  /// its best address (home or public).
  Future<void> activate(PhotovaultServerInfo info) async {
    await _save(info);
    await _prefs!.setString(_activeKey, info.publicUrl);
    activeServer.value = info;
    final address = await _selector.select(info);
    await _setEndpoint(address);
  }

  /// Forgets which server is active (the saved list stays). Callers log out
  /// first when an account is signed in.
  Future<void> clearActive() async {
    await _prefs?.remove(_activeKey);
    activeServer.value = null;
  }

  Future<void> forget(PhotovaultServerInfo info) async {
    final servers = savedServers
      ..removeWhere((s) => sameServerAddress(s.publicUrl, info.publicUrl));
    await _writeServers(servers);
    if (sameServerAddress(activeServer.value?.publicUrl, info.publicUrl)) {
      await clearActive();
    }
  }

  /// Re-runs the home/public choice (app start, connectivity change) and
  /// refreshes the stored server description.
  Future<void> refreshAddress() {
    return _refreshInFlight ??= _refresh().whenComplete(
      () => _refreshInFlight = null,
    );
  }

  Future<void> _refresh() async {
    final server = activeServer.value;
    if (server == null || _prefs == null) return;
    try {
      final address = await _selector.select(server);
      await _setEndpoint(address);
      if (!server.isPhotovault) return;
      final fresh = await _client.probe(address, kHomeProbeTimeout * 2);
      if (fresh != null &&
          sameServerAddress(fresh.publicUrl, server.publicUrl) &&
          sameServerAddress(activeServer.value?.publicUrl, server.publicUrl)) {
        await _save(fresh);
        activeServer.value = fresh;
      }
    } catch (e, s) {
      _logger.warning("Address refresh failed", e, s);
    }
  }

  Future<void> _setEndpoint(String address) async {
    final config = _endpointConfig!;
    if (config.endpoint == address) return;
    _logger.info("Using server address $address");
    await config.setEndpoint(address);
  }

  Future<void> _save(PhotovaultServerInfo info) async {
    final servers = savedServers;
    final i = servers.indexWhere(
      (s) => sameServerAddress(s.publicUrl, info.publicUrl),
    );
    if (i >= 0) {
      servers[i] = info;
    } else {
      servers.add(info);
    }
    await _writeServers(servers);
  }

  Future<void> _writeServers(List<PhotovaultServerInfo> servers) async {
    await _prefs!.setString(
      _serversKey,
      jsonEncode(servers.map((s) => s.toJson()).toList()),
    );
  }

  PhotovaultServerInfo? _readActive() {
    final id = _prefs?.getString(_activeKey);
    if (id == null) return null;
    for (final s in savedServers) {
      if (sameServerAddress(s.publicUrl, id)) return s;
    }
    return null;
  }

  /// Values that must survive `Configuration.logout` (which clears all
  /// SharedPreferences): the saved servers, the active one and Ente's
  /// endpoint.
  static Map<String, String> preserveOnLogout(SharedPreferences prefs) {
    final kept = <String, String>{};
    for (final key in prefs.getKeys()) {
      if (key.startsWith(_keyPrefix) || key == EndpointConfig.preferencesKey) {
        final value = prefs.get(key);
        if (value is String) kept[key] = value;
      }
    }
    return kept;
  }

  static Future<void> restoreAfterLogout(
    SharedPreferences prefs,
    Map<String, String> kept,
  ) async {
    for (final entry in kept.entries) {
      await prefs.setString(entry.key, entry.value);
    }
  }
}
