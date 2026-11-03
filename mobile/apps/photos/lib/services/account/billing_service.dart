import 'package:ente_pure_utils/ente_pure_utils.dart';
import 'package:flutter/material.dart';
import 'package:logging/logging.dart';
import 'package:photos/models/user_details.dart';
import 'package:photos/ui/family/family_plan_page.dart';
import 'package:photos/utils/dialog_util.dart';

// PHOTOVAULT: payments removed. Plans, Stripe/Play/App Store purchases,
// subscription verification and the purchase listener are gone; only the
// family page entry point (Family is kept as upstream) and the cache hook
// called on logout remain. The billing API client
// (gateways/billing/billing_gateway.dart) is unchanged.
class BillingService {
  late final _logger = Logger("BillingService");

  BillingService() {
    _logger.info("BillingService constructor");
  }

  void clearCache() {}

  Future<void> launchFamilyPortal(
    BuildContext context,
    UserDetails userDetails, {
    bool popOnFreeAdvertViewPlans = false,
    bool refreshOnOpen = true,
  }) async {
    try {
      await routeToPage(
        context,
        FamilyPlanPage(
          initialUserDetails: userDetails,
          popOnFreeAdvertViewPlans: popOnFreeAdvertViewPlans,
          refreshOnOpen: refreshOnOpen,
        ),
      );
    } catch (e) {
      if (!context.mounted) return;
      await showGenericErrorDialog(context: context, error: e);
    }
  }
}
