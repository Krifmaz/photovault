import "dart:async";

import "package:ente_components/ente_components.dart";
import "package:ente_components/theme/photovault/encore_tokens.dart"; // PHOTOVAULT
import "package:ente_pure_utils/ente_pure_utils.dart";
import "package:ente_strings/ente_strings.dart";
import "package:ente_ui/pages/language_selector_page.dart";
import "package:flutter/foundation.dart";
import 'package:flutter/material.dart';
import "package:photos/app.dart";
import 'package:photos/core/configuration.dart';
import 'package:photos/core/event_bus.dart'; // PHOTOVAULT: payments removed
import 'package:photos/events/subscription_purchased_event.dart'; // PHOTOVAULT: payments removed
import "package:photos/locale.dart";
import "package:photos/service_locator.dart";
import "package:photos/services/photovault/photovault_server_service.dart"; // PHOTOVAULT
import "package:photos/services/photovault/server_contract.dart"; // PHOTOVAULT
import "package:photos/theme/ente_theme.dart";
import 'package:photos/ui/account/email_entry_page.dart';
import 'package:photos/ui/account/login_page.dart';
import 'package:photos/ui/account/password_entry_page.dart';
import 'package:photos/ui/account/password_reentry_page.dart';
import 'package:photos/ui/components/buttons/button_widget.dart';
import 'package:photos/ui/components/dialog_widget.dart';
import 'package:photos/ui/components/models/button_type.dart';
import "package:photos/ui/photovault/connect_server_page.dart"; // PHOTOVAULT
import "package:photos/ui/photovault/photovault_glyph.dart";
import "package:photos/ui/photovault/photovault_server_chip.dart"; // PHOTOVAULT

class LandingPageWidget extends StatefulWidget {
  const LandingPageWidget({required this.onStartWithoutAccount, super.key});
  final VoidCallback onStartWithoutAccount;

  @override
  State<LandingPageWidget> createState() => _LandingPageWidgetState();
}

class _LandingPageWidgetState extends State<LandingPageWidget> {
  @override
  void initState() {
    super.initState();
    Future(_showAutoLogoutDialogIfRequired);
  }

  // PHOTOVAULT: server connection. "Connect to your server" comes first;
  // Ente's landing page (create account / log in) follows once a server is
  // active.
  @override
  Widget build(BuildContext context) {
    return ValueListenableBuilder<PhotovaultServerInfo?>(
      valueListenable: PhotovaultServerService.instance.activeServer,
      builder: (context, server, _) {
        if (server == null) {
          return ConnectServerPage(
            onConnected: () {},
            onUseWithoutServer: localSettings.showLocalGalleryModeOption
                ? _navigateWithoutAccount
                : null,
          );
        }
        return _buildEnteLanding(context, server);
      },
    );
  }

  // PHOTOVAULT: was build(); takes the active server.
  Widget _buildEnteLanding(BuildContext context, PhotovaultServerInfo server) {
    final lightComponentTheme = ComponentTheme.lightTheme(
      app: ComponentApp.photos,
    );
    return Theme(
      data: lightComponentTheme,
      child: Builder(
        builder: (context) {
          final colorScheme = getEnteColorScheme(context);
          return Scaffold(
            backgroundColor: colorScheme.greenBase,
            // PHOTOVAULT: removed the 7-tap developer settings (custom
            // endpoint) area and the custom-endpoint label below.
            body: SafeArea(
              child: Builder(
                builder: (context) => Padding(
                  padding: const EdgeInsets.symmetric(horizontal: 16),
                  child: Column(
                    children: [
                      Expanded(
                        child: LayoutBuilder(
                          builder: (context, constraints) {
                            return SingleChildScrollView(
                              child: ConstrainedBox(
                                constraints: BoxConstraints(
                                  minHeight: constraints.maxHeight,
                                ),
                                child: Column(
                                  mainAxisAlignment: MainAxisAlignment.center,
                                  children: [
                                    if (kDebugMode) _buildDebugLanguageButton(),

                                    _buildOnboardingAnimation(),

                                    Text(
                                      context.strings.onboardingTitle,
                                      textAlign: TextAlign.center,
                                      textScaler: TextScaler.noScaling,
                                      style: const TextStyle(
                                        fontWeight: FontWeight.w800,
                                        fontFamily: TextStyles.outfitFontFamily,
                                        package: TextStyles.fontPackage,
                                        fontSize: 28,
                                        height: 30 / 28,
                                        color: Colors.white,
                                      ),
                                    ),

                                    const SizedBox(height: 24),

                                    Padding(
                                      padding: const EdgeInsets.symmetric(
                                        horizontal: 32,
                                      ),
                                      child: Text(
                                        context.strings.onboardingOrganizeDesc,
                                        textAlign: TextAlign.center,
                                        style: TextStyles.body.copyWith(
                                          color: colorScheme.greenLight,
                                        ),
                                      ),
                                    ),

                                    const SizedBox(height: 32),
                                  ],
                                ),
                              ),
                            );
                          },
                        ),
                      ),
                      // PHOTOVAULT: server connection. Which server, and a
                      // way back to the connect screen.
                      PhotovaultServerChip(
                        server: server,
                        onChange: _changeServer,
                      ),
                      const SizedBox(height: 12),
                      // PHOTOVAULT: hidden when the server takes no sign-ups.
                      if (server.signupsAllowed) ...[
                        Padding(
                          padding: const EdgeInsets.symmetric(horizontal: 8),
                          child: ButtonComponent(
                            variant: ButtonComponentVariant.neutral,
                            density: ButtonComponentDensity.compact,
                            label: context.strings.createMyAccount,
                            onTap: _navigateToSignUpPage,
                            shouldSurfaceExecutionStates: false,
                          ),
                        ),
                        const SizedBox(height: 12),
                      ],
                      Padding(
                        padding: const EdgeInsets.symmetric(horizontal: 8),
                        child: SizedBox(
                          width: double.infinity,
                          child: TextButton(
                            onPressed: _navigateToSignInPage,
                            style: TextButton.styleFrom(
                              backgroundColor: Colors.white,
                              foregroundColor: Colors.black,
                              minimumSize: const Size.fromHeight(48),
                              padding: const EdgeInsets.symmetric(
                                horizontal: 24,
                                vertical: 14,
                              ),
                              shape: RoundedRectangleBorder(
                                borderRadius: BorderRadius.circular(20),
                              ),
                            ),
                            child: Text(
                              context.strings.logInLabel,
                              style: TextStyles.body.copyWith(
                                color: Colors.black,
                              ),
                            ),
                          ),
                        ),
                      ),
                      if (localSettings.showLocalGalleryModeOption) ...[
                        const SizedBox(height: 12),
                        Padding(
                          padding: const EdgeInsets.symmetric(horizontal: 8),
                          child: SizedBox(
                            width: double.infinity,
                            child: TextButton(
                              onPressed: _navigateWithoutAccount,
                              style: TextButton.styleFrom(
                                foregroundColor: Colors.white,
                                minimumSize: const Size.fromHeight(48),
                                padding: const EdgeInsets.symmetric(
                                  horizontal: 24,
                                  vertical: 14,
                                ),
                                shape: RoundedRectangleBorder(
                                  borderRadius: BorderRadius.circular(20),
                                ),
                              ),
                              child: Text(
                                context.strings.continueWithoutAccount,
                                style: TextStyles.bodyLink.copyWith(
                                  color: Colors.white,
                                  decorationColor: Colors.white,
                                ),
                              ),
                            ),
                          ),
                        ),
                      ],
                    ],
                  ),
                ),
              ),
            ),
          );
        },
      ),
    );
  }

  // PHOTOVAULT: branding. Static glyph instead of Ente's Rive duck
  // ("onboarding.riv"), in the same height-limited box.
  Widget _buildOnboardingAnimation() {
    final size = MediaQuery.of(context).size.height * 0.325;
    return PhotovaultGlyph(width: size, height: size, color: encoreDarkAccent2);
  }

  Widget _buildDebugLanguageButton() {
    return GestureDetector(
      child: const Align(
        alignment: Alignment.topRight,
        child: Padding(
          padding: EdgeInsets.only(right: 16),
          child: Text("Lang", style: TextStyle(color: Colors.black54)),
        ),
      ),
      onTap: () async {
        final locale = (await getLocale())!;
        if (!mounted) return;
        unawaited(
          routeToPage(
            context,
            LanguageSelectorPage(appSupportedLocales, (locale) async {
              await setLocale(locale);
              if (!mounted) return;
              EnteApp.setLocale(context, locale);
              unawaited(StringsLocalizations.delegate.load(locale));
            }, locale),
          ).then((value) {
            if (!mounted) return;
            setState(() {});
          }),
        );
      },
    );
  }

  // PHOTOVAULT: server connection. Back to the connect screen; a half-done
  // sign-up on the old server is discarded with Ente's logout.
  Future<void> _changeServer() async {
    if (Configuration.instance.getToken() != null ||
        Configuration.instance.getEncryptedToken() != null) {
      await Configuration.instance.logout();
    }
    await PhotovaultServerService.instance.clearActive();
  }

  void _navigateWithoutAccount() {
    widget.onStartWithoutAccount();
  }

  Future<void> _navigateToSignUpPage() async {
    updateService.hideChangeLog().ignore();
    Widget page;
    if (Configuration.instance.getEncryptedToken() == null) {
      page = const EmailEntryPage();
    } else {
      if (Configuration.instance.getKeyAttributes() == null) {
        page = const PasswordEntryPage(mode: PasswordEntryMode.set);
      } else if (Configuration.instance.getKey() == null) {
        page = const PasswordReentryPage();
      } else {
        // PHOTOVAULT: payments removed (was the onboarding plan picker).
        Bus.instance.fire(SubscriptionPurchasedEvent());
        return;
      }
    }
    unawaited(
      Navigator.of(context).push(
        MaterialPageRoute(
          builder: (BuildContext context) {
            return page;
          },
        ),
      ),
    );
  }

  void _navigateToSignInPage() {
    updateService.hideChangeLog().ignore();
    Widget page;
    if (Configuration.instance.getEncryptedToken() == null) {
      page = const LoginPage();
    } else {
      if (Configuration.instance.getKeyAttributes() == null) {
        page = const PasswordEntryPage(mode: PasswordEntryMode.set);
      } else if (Configuration.instance.getKey() == null) {
        page = const PasswordReentryPage();
      } else {
        // PHOTOVAULT: payments removed (was the onboarding plan picker).
        Bus.instance.fire(SubscriptionPurchasedEvent());
        return;
      }
    }
    Navigator.of(context).push(
      MaterialPageRoute(
        builder: (BuildContext context) {
          return page;
        },
      ),
    );
  }

  Future<void> _showAutoLogoutDialogIfRequired() async {
    final bool autoLogout = Configuration.instance.showAutoLogoutDialog();
    if (autoLogout) {
      final result = await showDialogWidget(
        context: context,
        title: context.strings.pleaseLoginAgain,
        body: context.strings.autoLogoutMessage,
        buttons: [
          ButtonWidget(
            buttonType: ButtonType.neutral,
            buttonAction: ButtonAction.first,
            labelText: context.strings.ok,
            isInAlert: true,
          ),
        ],
      );
      Configuration.instance.clearAutoLogoutFlag().ignore();
      if (result?.action != null && result!.action == ButtonAction.first) {
        _navigateToSignInPage();
      }
    }
  }
}
