// PHOTOVAULT: server connection (new file).
//
// "Connect to your server": the first onboarding screen. The person types an
// address (or scans the server's QR code, or taps a server found on the
// LAN); Connect fetches `/photovault/info` and shows the server's name and
// version; Continue makes it the active server and hands over to Ente's own
// sign-in / create-account flow, which is unchanged.
//
// Encore styling: theme colours, Inter, radius 14 for cards and 10 for
// controls, flat surfaces (no blur, shadow or animated background).

import "dart:async";

import "package:ente_components/ente_components.dart";
import "package:ente_components/theme/photovault/encore_tokens.dart";
import "package:flutter/material.dart";
import "package:flutter/services.dart";
import "package:photos/services/photovault/photovault_server_service.dart";
import "package:photos/services/photovault/photovault_strings.dart";
import "package:photos/services/photovault/server_address.dart";
import "package:photos/services/photovault/server_contract.dart";
import "package:photos/services/photovault/server_info_client.dart";
import "package:photos/ui/photovault/photovault_glyph.dart";
import "package:photos/ui/photovault/server_qr_scan_page.dart";

class ConnectServerPage extends StatefulWidget {
  const ConnectServerPage({
    super.key,
    this.onConnected,
    this.onUseWithoutServer,
  });

  /// Called after the chosen server became the active one. When null the
  /// page pops itself.
  final VoidCallback? onConnected;

  /// Shows "Use without a server" (Ente's offline local gallery) when set.
  final VoidCallback? onUseWithoutServer;

  @override
  State<ConnectServerPage> createState() => _ConnectServerPageState();
}

class _ConnectServerPageState extends State<ConnectServerPage> {
  final _service = PhotovaultServerService.instance;
  final _controller = TextEditingController();
  bool _busy = false;
  String? _error;
  PhotovaultServerInfo? _found;
  List<DiscoveredServer> _discovered = const [];
  bool _searching = false;

  @override
  void initState() {
    super.initState();
    final builtIn = _service.builtInAddress;
    if (builtIn != null && _service.savedServers.isEmpty) {
      _controller.text = builtIn;
    }
    if (_service.discovery.isSupported) unawaited(_discover());
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  Future<void> _discover() async {
    if (_searching) return;
    setState(() => _searching = true);
    final found = await _service.discovery.search();
    if (!mounted) return;
    setState(() {
      _searching = false;
      _discovered = found;
    });
  }

  /// Fetches the server's description for [input] and shows it.
  Future<void> _lookUp(String input) async {
    if (_busy) return;
    FocusScope.of(context).unfocus();
    setState(() {
      _busy = true;
      _error = null;
      _found = null;
    });
    try {
      final info = await _service.fetchInfo(input);
      if (!mounted) return;
      setState(() => _found = info);
    } on ServerConnectException catch (e) {
      if (!mounted) return;
      setState(() => _error = PhotovaultStrings.connectError(e));
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  /// A saved server: refresh its description (public, then home address)
  /// and use it straight away.
  Future<void> _useSaved(PhotovaultServerInfo server) async {
    if (_busy) return;
    setState(() {
      _busy = true;
      _error = null;
      _found = null;
    });
    ServerConnectException? firstError;
    PhotovaultServerInfo? info;
    for (final address in [server.publicUrl, server.homeUrl]) {
      if (address == null) continue;
      try {
        info = await _service.fetchInfo(address);
        break;
      } on ServerConnectException catch (e) {
        firstError ??= e;
      }
    }
    if (!mounted) return;
    if (info == null) {
      setState(() {
        _busy = false;
        _error = PhotovaultStrings.connectError(firstError!);
      });
      return;
    }
    await _activate(info);
  }

  Future<void> _activate(PhotovaultServerInfo info) async {
    setState(() => _busy = true);
    await _service.activate(info);
    if (!mounted) return;
    setState(() => _busy = false);
    if (widget.onConnected != null) {
      widget.onConnected!();
    } else {
      Navigator.of(context).pop(true);
    }
  }

  Future<void> _scan() async {
    final raw = await Navigator.of(
      context,
    ).push<String>(MaterialPageRoute(builder: (_) => const ServerQrScanPage()));
    if (raw == null || !mounted) return;
    final payload = parseConnectPayload(raw);
    if (payload == null || tryNormalizeServerAddress(payload.url) == null) {
      setState(() => _error = PhotovaultStrings.qrNotAServer);
      return;
    }
    _controller.text = payload.url;
    await _lookUp(payload.url);
  }

  @override
  Widget build(BuildContext context) {
    final colors = context.componentColors;
    final saved = _service.savedServers;
    final canPop = Navigator.of(context).canPop();
    return Scaffold(
      backgroundColor: colors.backgroundBase,
      appBar: canPop
          ? AppBar(
              backgroundColor: colors.backgroundBase,
              elevation: 0,
              scrolledUnderElevation: 0,
            )
          : null,
      body: SafeArea(
        child: ListView(
          padding: const EdgeInsets.fromLTRB(
            Spacing.lg,
            Spacing.xxl,
            Spacing.lg,
            Spacing.xxl,
          ),
          children: [
            Center(
              child: PhotovaultGlyph(
                width: 88,
                height: 88,
                scale: 0.8,
                color: colors.primary,
              ),
            ),
            const SizedBox(height: Spacing.lg),
            Text(
              PhotovaultStrings.connectTitle,
              textAlign: TextAlign.center,
              style: TextStyles.h1Bold.copyWith(color: colors.textBase),
            ),
            const SizedBox(height: Spacing.sm),
            Text(
              PhotovaultStrings.connectSubtitle,
              textAlign: TextAlign.center,
              style: TextStyles.body.copyWith(color: colors.textLight),
            ),
            const SizedBox(height: Spacing.xxl),
            _buildAddressField(colors),
            if (_error != null) ...[
              const SizedBox(height: Spacing.sm),
              Text(
                _error!,
                style: TextStyles.mini.copyWith(color: colors.warning),
              ),
            ],
            const SizedBox(height: Spacing.lg),
            if (_found != null)
              _FoundServerCard(
                info: _found!,
                busy: _busy,
                onContinue: () => _activate(_found!),
              )
            else
              ButtonComponent(
                label: _busy
                    ? PhotovaultStrings.connecting
                    : PhotovaultStrings.connect,
                isDisabled: _busy,
                shouldSurfaceExecutionStates: false,
                onTap: () => _lookUp(_controller.text),
              ),
            if (saved.isNotEmpty) ...[
              const _SectionHeader(PhotovaultStrings.savedServers),
              for (final server in saved)
                _ServerRow(
                  title: server.isPhotovault
                      ? server.name
                      : PhotovaultStrings.compatibleServer,
                  subtitle: _subtitle(
                    displayServerAddress(server.publicUrl),
                    server.version,
                  ),
                  onTap: _busy ? null : () => _useSaved(server),
                  onRemove: () async {
                    await _service.forget(server);
                    if (mounted) setState(() {});
                  },
                ),
            ],
            if (_service.discovery.isSupported) ...[
              _SectionHeader(
                PhotovaultStrings.onThisNetwork,
                trailing: _searching
                    ? const SizedBox(
                        width: 16,
                        height: 16,
                        child: CircularProgressIndicator(strokeWidth: 2),
                      )
                    : IconButton(
                        tooltip: PhotovaultStrings.searchAgain,
                        icon: Icon(Icons.refresh, color: colors.textLight),
                        onPressed: _discover,
                      ),
              ),
              if (_discovered.isEmpty)
                Text(
                  _searching
                      ? PhotovaultStrings.searching
                      : PhotovaultStrings.noneFound,
                  style: TextStyles.mini.copyWith(color: colors.textLight),
                ),
              for (final server in _discovered)
                _ServerRow(
                  title: server.name,
                  subtitle: _subtitle(
                    displayServerAddress(
                      tryNormalizeServerAddress(server.address) ??
                          server.address,
                    ),
                    server.version ?? "",
                  ),
                  onTap: _busy
                      ? null
                      : () {
                          _controller.text = server.address;
                          _lookUp(server.address);
                        },
                ),
            ],
            if (widget.onUseWithoutServer != null) ...[
              const SizedBox(height: Spacing.xxl),
              Center(
                child: TextButton(
                  onPressed: widget.onUseWithoutServer,
                  child: Text(
                    PhotovaultStrings.useWithoutServer,
                    style: TextStyles.bodyLink.copyWith(
                      color: colors.textLight,
                      decorationColor: colors.textLight,
                    ),
                  ),
                ),
              ),
            ],
          ],
        ),
      ),
    );
  }

  Widget _buildAddressField(ColorTokens colors) {
    final border = OutlineInputBorder(
      borderRadius: BorderRadius.circular(EncoreTokens.radiusControl),
      borderSide: BorderSide(color: colors.strokeFaint),
    );
    return TextField(
      controller: _controller,
      enabled: !_busy,
      keyboardType: TextInputType.url,
      textInputAction: TextInputAction.go,
      autocorrect: false,
      enableSuggestions: false,
      inputFormatters: [FilteringTextInputFormatter.deny(RegExp(r"\s"))],
      style: TextStyles.body.copyWith(color: colors.textBase),
      onChanged: (_) {
        if (_found != null || _error != null) {
          setState(() {
            _found = null;
            _error = null;
          });
        }
      },
      onSubmitted: _lookUp,
      decoration: InputDecoration(
        labelText: PhotovaultStrings.addressLabel,
        hintText: PhotovaultStrings.addressHint,
        filled: true,
        fillColor: colors.fillLight,
        labelStyle: TextStyles.body.copyWith(color: colors.textLight),
        hintStyle: TextStyles.body.copyWith(color: colors.textLighter),
        border: border,
        enabledBorder: border,
        focusedBorder: border.copyWith(
          borderSide: BorderSide(color: colors.primary, width: 1.5),
        ),
        suffixIcon: IconButton(
          tooltip: PhotovaultStrings.scanQr,
          icon: Icon(Icons.qr_code_scanner, color: colors.textLight),
          onPressed: _busy ? null : _scan,
        ),
      ),
    );
  }

  static String _subtitle(String address, String version) =>
      version.isEmpty ? address : "$address · ${version.trim()}";
}

class _FoundServerCard extends StatelessWidget {
  const _FoundServerCard({
    required this.info,
    required this.busy,
    required this.onContinue,
  });

  final PhotovaultServerInfo info;
  final bool busy;
  final VoidCallback onContinue;

  @override
  Widget build(BuildContext context) {
    final colors = context.componentColors;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Container(
          padding: const EdgeInsets.all(Spacing.lg),
          decoration: BoxDecoration(
            color: colors.fillLight,
            borderRadius: BorderRadius.circular(EncoreTokens.radiusCard),
            border: Border.all(color: colors.strokeFaint),
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Row(
                children: [
                  Icon(Icons.check_circle, size: 18, color: colors.primary),
                  const SizedBox(width: Spacing.sm),
                  Expanded(
                    child: Text(
                      info.isPhotovault
                          ? info.name
                          : PhotovaultStrings.compatibleServer,
                      style: TextStyles.large.copyWith(color: colors.textBase),
                    ),
                  ),
                ],
              ),
              const SizedBox(height: Spacing.xs),
              if (info.version.isNotEmpty)
                Text(
                  PhotovaultStrings.versionLabel(info.version),
                  style: TextStyles.mini.copyWith(color: colors.textLight),
                ),
              Text(
                displayServerAddress(info.publicUrl),
                style: TextStyles.mini.copyWith(color: colors.textLight),
              ),
              if (!info.isPhotovault) ...[
                const SizedBox(height: Spacing.sm),
                Text(
                  PhotovaultStrings.compatibleServerNote,
                  style: TextStyles.mini.copyWith(color: colors.textLight),
                ),
              ],
              if (!info.signupsAllowed) ...[
                const SizedBox(height: Spacing.sm),
                Text(
                  PhotovaultStrings.signupsClosed,
                  style: TextStyles.mini.copyWith(color: colors.textLight),
                ),
              ],
            ],
          ),
        ),
        const SizedBox(height: Spacing.lg),
        ButtonComponent(
          label: PhotovaultStrings.continueLabel,
          isDisabled: busy,
          shouldSurfaceExecutionStates: false,
          onTap: onContinue,
        ),
      ],
    );
  }
}

class _SectionHeader extends StatelessWidget {
  const _SectionHeader(this.title, {this.trailing});

  final String title;
  final Widget? trailing;

  @override
  Widget build(BuildContext context) {
    final colors = context.componentColors;
    return Padding(
      padding: const EdgeInsets.only(top: Spacing.xxl, bottom: Spacing.sm),
      child: Row(
        children: [
          Expanded(
            child: Text(
              title,
              style: TextStyles.bodyBold.copyWith(color: colors.textBase),
            ),
          ),
          ?trailing,
        ],
      ),
    );
  }
}

class _ServerRow extends StatelessWidget {
  const _ServerRow({
    required this.title,
    required this.subtitle,
    this.onTap,
    this.onRemove,
  });

  final String title;
  final String subtitle;
  final VoidCallback? onTap;
  final VoidCallback? onRemove;

  @override
  Widget build(BuildContext context) {
    final colors = context.componentColors;
    final radius = BorderRadius.circular(EncoreTokens.radiusCard);
    return Padding(
      padding: const EdgeInsets.only(bottom: Spacing.sm),
      child: Material(
        color: colors.fillLight,
        shape: RoundedRectangleBorder(
          borderRadius: radius,
          side: BorderSide(color: colors.strokeFaint),
        ),
        child: InkWell(
          borderRadius: radius,
          onTap: onTap,
          child: Padding(
            padding: const EdgeInsets.symmetric(
              horizontal: Spacing.lg,
              vertical: Spacing.md,
            ),
            child: Row(
              children: [
                Icon(Icons.dns_outlined, size: 20, color: colors.textLight),
                const SizedBox(width: Spacing.md),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        title,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyles.body.copyWith(color: colors.textBase),
                      ),
                      Text(
                        subtitle,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyles.mini.copyWith(
                          color: colors.textLight,
                        ),
                      ),
                    ],
                  ),
                ),
                if (onRemove != null)
                  IconButton(
                    tooltip: PhotovaultStrings.forget,
                    icon: Icon(Icons.close, size: 18, color: colors.textLight),
                    onPressed: onRemove,
                  )
                else
                  Icon(Icons.chevron_right, color: colors.textLight),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
