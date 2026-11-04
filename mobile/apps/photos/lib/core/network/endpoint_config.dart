import "package:photos/core/event_bus.dart";
import "package:photos/events/event.dart";
import "package:shared_preferences/shared_preferences.dart";

class EndpointConfig {
  EndpointConfig(this._preferences);

  final SharedPreferences _preferences;

  // PHOTOVAULT: server connection. Built-in server chosen at build time with
  // --dart-define=PHOTOVAULT_SERVER=<url> (Ente's `endpoint` define still
  // works). Empty means none: the "Connect to your server" screen asks for
  // one, and Ente's production endpoint is never a default.
  static const _photovaultServer = String.fromEnvironment("PHOTOVAULT_SERVER");
  static const defaultEndpoint = _photovaultServer != ""
      ? _photovaultServer
      : String.fromEnvironment("endpoint");
  static const preferencesKey = "endpoint";

  String get endpoint {
    return _normalize(
      _preferences.getString(preferencesKey) ?? defaultEndpoint,
    );
  }

  // PHOTOVAULT: never Ente's production service. Upstream gates Ente-only
  // behaviour on this (Cloudflare upload proxy and "faster uploads" toggle,
  // free-plan family upsell, strict logout); Photovault always talks to a
  // self-hosted server.
  bool get isProduction => false;

  Future<void> setEndpoint(String endpoint) async {
    await _preferences.setString(preferencesKey, endpoint);
    Bus.instance.fire(EndpointUpdatedEvent(this.endpoint));
  }

  // PHOTOVAULT: strip - upstream migrated Ente's old api host to the new one here;
  // there is no Ente endpoint to migrate.
  static String _normalize(String endpoint) => endpoint;
}

class EndpointUpdatedEvent extends Event {
  EndpointUpdatedEvent(this.endpoint);

  final String endpoint;
}
