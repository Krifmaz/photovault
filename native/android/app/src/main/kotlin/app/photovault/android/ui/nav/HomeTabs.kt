package app.photovault.android.ui.nav

import androidx.annotation.DrawableRes
import androidx.annotation.StringRes
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.size
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.NavigationBarItemDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.navigation.NavHostController
import app.photovault.android.R
import app.photovault.android.ui.theme.palette

/**
 * The bottom bar's tabs, in order. To add a tab: add an entry here (its route must be a top-level
 * destination in AppNav) and wrap that destination in [HomeTabScaffold] with the entry.
 */
enum class HomeTab(
    val route: Any,
    @StringRes val label: Int,
    @DrawableRes val icon: Int,
    val testTag: String,
) {
    Photos(TimelineRoute, R.string.tab_photos, R.drawable.ic_photos, "tab_photos"),
    Albums(AlbumsRoute, R.string.tab_albums, R.drawable.ic_albums, "tab_albums"),
    Search(SearchRoute, R.string.tab_search, R.drawable.ic_search, "tab_search"),
}

/**
 * A tab's screen with the bottom bar under it. The bar belongs to each tab destination (not to the whole
 * NavHost), so opening the viewer or another full-screen destination never re-lays out the tab's content.
 */
@Composable
fun HomeTabScaffold(current: HomeTab, nav: NavHostController, content: @Composable () -> Unit) {
    Column(Modifier.fillMaxSize()) {
        // The bar pads itself for the navigation bar, so the screen above must not.
        Box(Modifier.weight(1f).fillMaxWidth().consumeWindowInsets(WindowInsets.navigationBars)) {
            content()
        }
        HomeBottomBar(current) { tab -> nav.navigateToTab(tab) }
    }
}

@Composable
fun HomeBottomBar(current: HomeTab, onSelect: (HomeTab) -> Unit) {
    val p = MaterialTheme.palette
    Column {
        HorizontalDivider(color = p.border, thickness = 1.dp)
        NavigationBar(containerColor = p.bg, tonalElevation = 0.dp) {
            HomeTab.entries.forEach { tab ->
                NavigationBarItem(
                    selected = tab == current,
                    onClick = { if (tab != current) onSelect(tab) },
                    icon = { Icon(painterResource(tab.icon), contentDescription = null, modifier = Modifier.size(24.dp)) },
                    label = { Text(stringResource(tab.label)) },
                    modifier = Modifier.testTag(tab.testTag),
                    colors = NavigationBarItemDefaults.colors(
                        selectedIconColor = p.text,
                        selectedTextColor = p.text,
                        indicatorColor = p.panel2,
                        unselectedIconColor = p.muted,
                        unselectedTextColor = p.muted,
                    ),
                )
            }
        }
    }
}

/**
 * Switches tabs like a bottom bar should: one entry per tab above the Photos tab (Back from any tab goes
 * to Photos), each tab's own back stack saved and restored.
 */
fun NavHostController.navigateToTab(tab: HomeTab) {
    navigate(tab.route) {
        popUpTo<TimelineRoute> { saveState = true }
        launchSingleTop = true
        restoreState = true
    }
}
