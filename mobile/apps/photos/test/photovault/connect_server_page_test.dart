// PHOTOVAULT: server connection. Smoke test of the connect screen.
import "package:ente_components/ente_components.dart";
import "package:flutter/material.dart";
import "package:flutter_test/flutter_test.dart";
import "package:photos/services/photovault/photovault_strings.dart";
import "package:photos/ui/photovault/connect_server_page.dart";

void main() {
  testWidgets("renders and explains an invalid address", (tester) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: ComponentTheme.lightTheme(app: ComponentApp.photos),
        home: const ConnectServerPage(),
      ),
    );
    expect(find.text(PhotovaultStrings.connectTitle), findsOneWidget);
    expect(find.text(PhotovaultStrings.connect), findsOneWidget);

    await tester.enterText(find.byType(TextField), "ftp://x");
    await tester.tap(find.text(PhotovaultStrings.connect));
    await tester.pumpAndSettle();
    expect(find.textContaining("isn't a server address"), findsOneWidget);
  });
}
