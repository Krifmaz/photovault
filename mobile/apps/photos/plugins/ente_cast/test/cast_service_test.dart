// PHOTOVAULT: server connection (new file). The Cast receiver comes from the
// app (the server's own), never Ente's built-in F5BCEC64.
import "package:ente_cast/ente_cast.dart";
import "package:flutter_test/flutter_test.dart";

void main() {
  test("no receiver unless the app names one", () {
    expect(CastService().appID, isNull);
    expect(CastService(appID: () => "  ").appID, isNull);
    expect(CastService(appID: () => null).appID, isNull);
    expect(CastService().canAutoPair, isFalse);
  });

  test("uses the receiver the app names, read on each use", () {
    var id = "1A2B3C4D";
    final service = CastService(appID: () => id);
    expect(service.appID, "1A2B3C4D");
    id = "5E6F7A8B";
    expect(service.appID, "5E6F7A8B");
  });
}
