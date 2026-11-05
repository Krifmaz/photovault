// PHOTOVAULT: server connection (new file).
//
// Saved servers ("Recent servers": when each last connected and the last
// address that answered, see recent_servers.dart), the active server and its
// current address. The address
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
import "package:photos/services/photovault/recent_servers.dart";
import "package:photos/services/photovault/server_address.dart";
import "package:photos/services/photovault/server_address_selector.dart";
import "package:photos/services/photovault/server_contract.dart";
import "package:photos/services/photovault/server_discovery.dart";
import "package:photos/services/photovault/server_info_client.dart";
import "package:photos/services/photovault/tailscale.dart";
import "package:shared_preferences/shared_preferences.dart";

class PhotovaultServerService {
  PhotovaultServerService._();

  static final PhotovaultServerService instance = PhotovaultServerService._();

  static const _keyPrefix = "photovault.";
  static const _serversKey = "${_keyPrefix}servers";
  static const _activeKey = "${_keyPrefix}activeServer";

  /// publicUrl -> the address the person connected through, when that was
  /// neither the public nor the home URL (see [ServerAddressSelector.select]).
  static const _reachedAtKey = "${_keyPrefix}reachedAt";

  /// publicUrl -> when it last connected and the address that answered
  /// ([RecentServerEntry]).
  static const _recentKey = "${_keyPrefix}recent";

  /// Optional label for the built-in server (`contracts/apps.json`).
  static const builtInServerName = String.fromEnvironment(
    "PHOTOVAULT_SERVER_NAME",
  );

  final _logger = Logger("PhotovaultServerService");
  PhotovaultServerInfoClient _client = PhotovaultServerInfoClient();
  late final _selector = ServerAddressSelector(
    probe: (baseUrl, timeout) => _client.probe(baseUrl, timeout),
  );
  PhotovaultServerDiscovery _discovery = const PhotovaultServerDiscovery();
  PhotovaultServerDiscovery get discovery => _discovery;
  Future<bool> Function() _tailscaleDetector = detectTailscale;
  DateTime Function() _now = DateTime.now;

  /// Test seam: a fake HTTP client, LAN discovery, Tailscale detection
  /// and/or clock (screenshot and widget tests).
  @visibleForTesting
  void overrideForTesting({
    PhotovaultServerInfoClient? client,
    PhotovaultServerDiscovery? discovery,
    Future<bool> Function()? tailscale,
    DateTime Function()? now,
  }) {
    if (client != null) _client = client;
    if (discovery != null) _discovery = discovery;
    if (tailscale != null) _tailscaleDetector = tailscale;
    if (now != null) _now = now;
  }

  /// True while Tailscale is up on this device (see tailscale.dart).
  Future<bool> isTailscaleActive() async {
    try {
      return await _tailscaleDetector();
    } catch (_) {
      return false;
    }
  }

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

  /// The saved servers for "Recent servers": most recently connected first.
  List<RecentServer> get recentServers => orderRecentServers(
    savedServers,
    _recentEntries(),
    reachedAt: _reachedAtMap(),
    activeUrl: _prefs?.getString(_activeKey),
  );

  /// Probes the recent servers' last known address, then home and public,
  /// directly over HTTP (for a VPN, which mDNS does not cross). Short
  /// timeouts ([kHomeProbeTimeout] per address).
  Future<List<DirectlyReachedServer>> probeRecentDirectly() =>
      probeRecentServersDirectly(
        recentServers,
        (baseUrl, timeout) => _client.probe(baseUrl, timeout),
      );

  /// Normalises [input] and asks the server what it is.
  /// Throws [ServerConnectException].
  Future<PhotovaultServerInfo> fetchInfo(
    String input, {
    Duration timeout = kConnectTimeout,
  }) async {
    final String address;
    try {
      address = normalizeServerAddress(input);
    } on InvalidServerAddressException {
      throw ServerConnectException(
        ServerConnectErrorKind.invalidAddress,
        input.trim(),
      );
    }
    return _client.fetch(address, timeout: timeout);
  }

  /// Saves [info], makes it the active server and points Ente's endpoint at
  /// its best address (home or public). [answeredAddress] is the address
  /// that answered the Connect request; it is kept as a last resort for
  /// when the public URL cannot be reached from this network.
  Future<void> activate(
    PhotovaultServerInfo info, {
    String? answeredAddress,
  }) async {
    await _save(info);
    await _prefs!.setString(_activeKey, info.publicUrl);
    await _setReachedAt(info, answeredAddress);
    final answered = tryNormalizeServerAddress(answeredAddress);
    await _setRecent(info, answered);
    // Endpoint first, so the landing page that listens to [activeServer]
    // never signs in against the previous address.
    final address = await _selector.select(
      info,
      reachedAt: _reachedAt(info),
      lastAddress: answered,
      vpnActive: await _vpnCouldHelp(info, answered),
    );
    await _setEndpoint(address);
    activeServer.value = info;
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
    await _setReachedAt(info, null);
    final recent = _recentEntries();
    if (recent.remove(recentServerKey(info.publicUrl)) != null) {
      await _prefs!.setString(_recentKey, encodeRecentEntries(recent));
    }
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
      final last = _recentEntries()[recentServerKey(server.publicUrl)]?.address;
      final address = await _selector.select(
        server,
        reachedAt: _reachedAt(server),
        lastAddress: last,
        vpnActive: await _vpnCouldHelp(server, last),
      );
      await _setEndpoint(address);
      if (!server.isPhotovault) return;
      final fresh = await _client.probe(address, kHomeProbeTimeout * 2);
      if (fresh != null &&
          sameServerAddress(fresh.publicUrl, server.publicUrl) &&
          sameServerAddress(activeServer.value?.publicUrl, server.publicUrl)) {
        await _save(fresh);
        await _setRecent(fresh, address);
        activeServer.value = fresh;
      }
    } catch (e, s) {
      _logger.warning("Address refresh failed", e, s);
    }
  }

  Map<String, String> _reachedAtMap() {
    final raw = _prefs?.getString(_reachedAtKey);
    if (raw == null) return {};
    try {
      final map = jsonDecode(raw);
      if (map is! Map) return {};
      return {
        for (final e in map.entries)
          if (e.key is String && e.value is String)
            e.key as String: e.value as String,
      };
    } catch (_) {
      return {};
    }
  }

  String? _reachedAt(PhotovaultServerInfo server) =>
      _reachedAtMap()[tryNormalizeServerAddress(server.publicUrl) ??
          server.publicUrl];

  Future<void> _setReachedAt(
    PhotovaultServerInfo server,
    String? address,
  ) async {
    final key = tryNormalizeServerAddress(server.publicUrl) ?? server.publicUrl;
    final map = _reachedAtMap();
    final normalized = tryNormalizeServerAddress(address);
    if (normalized == null ||
        sameServerAddress(normalized, server.publicUrl) ||
        sameServerAddress(normalized, server.homeUrl)) {
      if (map.remove(key) == null) return;
    } else {
      map[key] = normalized;
    }
    await _prefs!.setString(_reachedAtKey, jsonEncode(map));
  }

  /// Tailscale is only checked when [lastAddress] is neither the public nor
  /// the home address, the only case where it changes the choice.
  Future<bool> _vpnCouldHelp(
    PhotovaultServerInfo server,
    String? lastAddress,
  ) async {
    if (lastAddress == null ||
        sameServerAddress(lastAddress, server.publicUrl) ||
        sameServerAddress(lastAddress, server.homeUrl)) {
      return false;
    }
    return isTailscaleActive();
  }

  Map<String, RecentServerEntry> _recentEntries() =>
      parseRecentEntries(_prefs?.getString(_recentKey));

  /// Records that [server] connected now; [address] is the one that
  /// answered (the previous one is kept when null).
  Future<void> _setRecent(PhotovaultServerInfo server, String? address) async {
    final entries = _recentEntries();
    final key = recentServerKey(server.publicUrl);
    entries[key] = RecentServerEntry(
      connectedAt: _now().toUtc(),
      address: address ?? entries[key]?.address,
    );
    await _prefs!.setString(_recentKey, encodeRecentEntries(entries));
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
