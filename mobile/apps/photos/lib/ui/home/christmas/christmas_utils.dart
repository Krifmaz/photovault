bool isChristmasDateRange() {
  final now = DateTime.now();
  return now.month == 12 && now.day >= 24 && now.day <= 26;
}

// PHOTOVAULT: smoothness/branding. Ente's seasonal Christmas decorations
// (Rive lights banner, pull animation, animated snow overlay) are removed, so
// this is always false.
bool isChristmasPeriod() => false;
