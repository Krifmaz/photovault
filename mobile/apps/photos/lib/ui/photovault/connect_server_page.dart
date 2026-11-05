// PHOTOVAULT: server connection (new file).
//
// "Connect to your server": the first onboarding screen. The person types an
// address (or scans the server's QR code, taps a recent server, or taps a
// server found on the LAN or, while Tailscale is up, over the VPN); Connect
// fetches `/photovault/info` and shows the server's name and
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
import "package:photos/services/photovault/recent_servers.dart";
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
    this.initialPayload,
  });

  /// A `photovault://connect` link (or QR text) to look up straight away.
  final ConnectPayload? initialPayload;

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

  /// The normalised address that answered for [_found].
  String? _foundAddress;
  List<DiscoveredServer> _discovered = const [];
  bool _searching = false;

  /// Tailscale is up: recent servers are probed directly (mDNS does not
  /// cross the VPN) and the ones that answered are listed.
  bool _tailscale = false;
  bool _probingDirect = false;
  List<DirectlyReachedServer> _direct = const [];

  @override
  void initState() {
    super.initState();
    final builtIn = _service.builtInAddress;
    if (builtIn != null && _service.savedServers.isEmpty) {
      _controller.text = builtIn;
    }
    final payload = widget.initialPayload;
    if (payload != null) {
      _controller.text = payload.url;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) unawaited(_lookUpPayload(payload));
      });
    }
    if (_service.discovery.isSupported) unawaited(_discover());
    unawaited(_probeDirect());
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

  /// While Tailscale is up, probes the recent servers' last known address
  /// (then home and public) directly.
  Future<void> _probeDirect() async {
    if (_probingDirect) return;
    if (_service.recentServers.isEmpty) return;
    setState(() => _probingDirect = true);
    final vpn = await _service.isTailscaleActive();
    if (!mounted) return;
    if (!vpn) {
      setState(() {
        _probingDirect = false;
        _tailscale = false;
        _direct = const [];
      });
      return;
    }
    setState(() => _tailscale = true);
    final found = await _service.probeRecentDirectly();
    if (!mounted) return;
    setState(() {
      _probingDirect = false;
      _direct = found;
    });
  }

  /// A server that just answered a direct probe: show it, ready to
  /// continue, without asking it again.
  void _showReached(DirectlyReachedServer reached) {
    _controller.text = displayServerAddress(reached.address);
    setState(() {
      _error = null;
      _found = reached.info;
      _foundAddress = reached.address;
    });
  }

  /// Fetches the server's description for [input] and shows it.
  Future<void> _lookUp(String input) => _lookUpFirst([input]);

  /// A scanned or linked payload: the public address first, then the home
  /// address it carries (a phone on the home LAN may only reach that one).
  Future<void> _lookUpPayload(ConnectPayload payload) =>
      _lookUpFirst([payload.url, payload.homeUrl]);

  /// Tries each address in order and shows the first server that answers.
  /// The field shows the address that answered.
  Future<void> _lookUpFirst(List<String?> candidates) async {
    if (_busy) return;
    FocusScope.of(context).unfocus();
    setState(() {
      _busy = true;
      _error = null;
      _found = null;
      _foundAddress = null;
    });
    ServerConnectException? firstError;
    try {
      for (final input in candidates) {
        if (input == null || input.trim().isEmpty) continue;
        try {
          final info = await _service.fetchInfo(input);
          if (!mounted) return;
          if (_controller.text != input) _controller.text = input;
          setState(() {
            _found = info;
            _foundAddress = tryNormalizeServerAddress(input);
          });
          return;
        } on ServerConnectException catch (e) {
          firstError ??= e;
        }
      }
      if (!mounted || firstError == null) return;
      setState(() => _error = PhotovaultStrings.connectError(firstError!));
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  /// A recent server: refresh its description (the last address that
  /// answered, then public, then home) and use it straight away.
  Future<void> _useRecent(RecentServer server) async {
    if (_busy) return;
    setState(() {
      _busy = true;
      _error = null;
      _found = null;
    });
    ServerConnectException? firstError;
    ServerConnectException? publicError;
    PhotovaultServerInfo? info;
    String? answered;
    for (final address in server.connectOrder) {
      final isPublic = sameServerAddress(address, server.info.publicUrl);
      try {
        // A last address other than the public one (a tailnet or LAN
        // address) may be out of reach now: give it a short timeout.
        info = await _service.fetchInfo(
          address,
          timeout: isPublic ? kConnectTimeout : kHomeProbeTimeout * 2,
        );
        answered = tryNormalizeServerAddress(address);
        break;
      } on ServerConnectException catch (e) {
        firstError ??= e;
        if (isPublic) publicError = e;
      }
    }
    if (!mounted) return;
    if (info == null) {
      setState(() {
        _busy = false;
        // Explain the public address when it was tried.
        _error = PhotovaultStrings.connectError((publicError ?? firstError)!);
      });
      return;
    }
    await _activate(info, answered);
  }

  Future<void> _activate(PhotovaultServerInfo info, String? answered) async {
    setState(() => _busy = true);
    await _service.activate(info, answeredAddress: answered);
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
    await _lookUpPayload(payload);
  }

  @override
  Widget build(BuildContext context) {
    final colors = context.componentColors;
    final recent = _service.recentServers;
    // The build-time server (PHOTOVAULT_SERVER / PHOTOVAULT_SERVER_NAME),
    // unless it is already saved.
    final builtInAddress = _service.builtInAddress;
    final builtIn =
        builtInAddress != null &&
            !recent.any(
              (s) => sameServerAddress(s.info.publicUrl, builtInAddress),
            )
        ? builtInAddress
        : null;
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
                onContinue: () => _activate(_found!, _foundAddress),
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
            if (builtIn != null) ...[
              const _SectionHeader(PhotovaultStrings.builtIn),
              _ServerRow(
                title: PhotovaultServerService.builtInServerName.isNotEmpty
                    ? PhotovaultServerService.builtInServerName
                    : displayServerAddress(builtIn),
                subtitle: displayServerAddress(builtIn),
                onTap: _busy
                    ? null
                    : () {
                        _controller.text = builtIn;
                        _lookUp(builtIn);
                      },
              ),
            ],
            if (recent.isNotEmpty) ...[
              const _SectionHeader(PhotovaultStrings.recentServers),
              for (final server in recent)
                _ServerRow(
                  title: server.info.isPhotovault
                      ? server.info.name
                      : PhotovaultStrings.compatibleServer,
                  subtitle: _subtitle(
                    displayServerAddress(server.info.publicUrl),
                    server.info.version,
                  ),
                  onTap: _busy ? null : () => _useRecent(server),
                  onRemove: () async {
                    await _service.forget(server.info);
                    if (!mounted) return;
                    setState(() {
                      _direct = _direct
                          .where(
                            (d) => !sameServerAddress(
                              d.info.publicUrl,
                              server.info.publicUrl,
                            ),
                          )
                          .toList();
                    });
                  },
                ),
            ],
            if (_tailscale && recent.isNotEmpty) ...[
              _SectionHeader(
                PhotovaultStrings.foundOverTailscale,
                trailing: _refreshTrailing(colors, _probingDirect, () {
                  unawaited(_probeDirect());
                }),
              ),
              if (_direct.isEmpty)
                Text(
                  _probingDirect
                      ? PhotovaultStrings.checkingRecentServers
                      : PhotovaultStrings.noneAnsweredOverTailscale,
                  style: TextStyles.mini.copyWith(color: colors.textLight),
                ),
              for (final reached in _direct)
                _ServerRow(
                  title: reached.info.name,
                  subtitle: _subtitle(
                    displayServerAddress(reached.address),
                    reached.info.version,
                  ),
                  onTap: _busy ? null : () => _showReached(reached),
                ),
            ],
            if (_service.discovery.isSupported) ...[
              _SectionHeader(
                PhotovaultStrings.onThisNetwork,
                trailing: _refreshTrailing(colors, _searching, _discover),
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

  static Widget _refreshTrailing(
    ColorTokens colors,
    bool running,
    VoidCallback onPressed,
  ) {
    if (running) {
      return const SizedBox(
        width: 16,
        height: 16,
        child: CircularProgressIndicator(strokeWidth: 2),
      );
    }
    return IconButton(
      tooltip: PhotovaultStrings.searchAgain,
      icon: Icon(Icons.refresh, color: colors.textLight),
      onPressed: onPressed,
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
