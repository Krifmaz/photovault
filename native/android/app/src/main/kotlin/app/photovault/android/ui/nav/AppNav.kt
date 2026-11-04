package app.photovault.android.ui.nav

import androidx.compose.animation.ExperimentalSharedTransitionApi
import androidx.compose.animation.SharedTransitionLayout
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.remember
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.navigation.NavBackStackEntry
import androidx.navigation.NavHostController
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.navigation
import androidx.navigation.compose.rememberNavController
import androidx.navigation.toRoute
import app.photovault.android.AppGraph
import app.photovault.android.BuildConfig
import app.photovault.android.ui.connect.ConnectScreen
import app.photovault.android.ui.connect.ConnectViewModel
import app.photovault.android.ui.connect.QrScanScreen
import app.photovault.android.ui.signin.EmailCodeScreen
import app.photovault.android.ui.signin.PasskeyScreen
import app.photovault.android.ui.signin.RecoveryScreen
import app.photovault.android.ui.signin.SignInEvents
import app.photovault.android.ui.signin.SignInScreen
import app.photovault.android.ui.signin.SignInViewModel
import app.photovault.android.ui.signin.TotpScreen
import app.photovault.android.ui.timeline.TimelineScreen
import app.photovault.android.ui.timeline.TimelineViewModel
import app.photovault.android.ui.viewer.ViewerScreen
import app.photovault.android.ui.viewer.ViewerViewModel

/**
 * Where the app opens: the timeline when signed in to the active server, sign-in when a server is chosen,
 * else connect. (After "Switch server" the active server can differ from the signed-in account's.)
 */
fun startDestination(graph: AppGraph): Any {
    val core = graph.core
    val active = core.activeBaseUrl() ?: return ConnectRoute
    val account = core.currentAccount() ?: return AuthGraph
    val server = core.savedServers().firstOrNull { it.publicUrl == active || it.homeUrl == active }
    val activePublic = (server?.publicUrl ?: active).trimEnd('/')
    return if (account.serverUrl.trimEnd('/') == activePublic) TimelineRoute else AuthGraph
}

@OptIn(ExperimentalSharedTransitionApi::class)
@Composable
fun AppNav(graph: AppGraph, start: Any) {
    val nav = rememberNavController()
    SharedTransitionLayout {
        CompositionLocalProvider(LocalSharedTransitionScope provides this) {
            NavHost(
                navController = nav,
                startDestination = start,
                enterTransition = { fadeIn() },
                exitTransition = { fadeOut() },
                popEnterTransition = { fadeIn() },
                popExitTransition = { fadeOut() },
            ) {
                composable<ConnectRoute> { entry ->
                    val vm = viewModel { ConnectViewModel(graph.core, graph.discovery, BuildConfig.DEFAULT_SERVER_URL) }
                    val handle = entry.savedStateHandle
                    val qr = remember(handle) { handle.getStateFlow<String?>(NavResults.QR_PAYLOAD, null) }
                    ConnectScreen(
                        vm = vm,
                        qrPayloads = qr,
                        onQrConsumed = { handle[NavResults.QR_PAYLOAD] = null },
                        onScanQr = { nav.navigate(QrScanRoute) },
                        onSignIn = {
                            // Opened from the timeline ("Switch server"): another server was chosen, so the
                            // old timeline (another account, another server) must not be reachable by Back.
                            nav.navigate(AuthGraph) {
                                if (nav.hasRoute<TimelineRoute>()) popUpTo<TimelineRoute> { inclusive = true }
                            }
                        },
                        onTimeline = { nav.navigateClearing(TimelineRoute) },
                    )
                }
                composable<QrScanRoute> {
                    QrScanScreen(
                        onResult = { payload ->
                            nav.previousBackStackEntry?.savedStateHandle?.set(NavResults.QR_PAYLOAD, payload)
                            nav.popBackStack()
                        },
                        onBack = { nav.popBackStack() },
                    )
                }
                navigation<AuthGraph>(startDestination = SignInRoute) {
                    composable<SignInRoute> { entry ->
                        val vm = nav.signInViewModel(entry, graph)
                        AuthEvents(vm, nav, graph)
                        SignInScreen(
                            vm = vm,
                            onChangeServer = {
                                if (!nav.popBackStack<ConnectRoute>(inclusive = false)) nav.navigateClearing(ConnectRoute)
                            },
                        )
                    }
                    composable<EmailCodeRoute> { entry ->
                        val vm = nav.signInViewModel(entry, graph)
                        AuthEvents(vm, nav, graph)
                        EmailCodeScreen(vm, onBack = { nav.popBackStack() })
                    }
                    composable<TotpRoute> { entry ->
                        val vm = nav.signInViewModel(entry, graph)
                        AuthEvents(vm, nav, graph)
                        TotpScreen(vm, entry.toRoute<TotpRoute>().sessionId, onBack = { nav.popBackStack() })
                    }
                    composable<PasskeyRoute> { entry ->
                        val vm = nav.signInViewModel(entry, graph)
                        val route = entry.toRoute<PasskeyRoute>()
                        AuthEvents(vm, nav, graph)
                        PasskeyScreen(vm, route.sessionId, route.url, onBack = { nav.popBackStack() })
                    }
                    composable<RecoveryRoute> { entry ->
                        val vm = nav.signInViewModel(entry, graph)
                        AuthEvents(vm, nav, graph)
                        RecoveryScreen(vm, onBack = { nav.popBackStack() })
                    }
                }
                composable<TimelineRoute> { entry ->
                    val vm = viewModel {
                        TimelineViewModel(graph.core, graph.timeline, graph.timelineLabels(), clearDecrypted = graph::clearDecrypted)
                    }
                    val handle = entry.savedStateHandle
                    val returned = remember(handle) { handle.getStateFlow<Int?>(NavResults.VIEWER_INDEX, null) }
                    CompositionLocalProvider(LocalNavAnimatedScope provides this) {
                        TimelineScreen(
                            vm = vm,
                            returnIndex = returned,
                            onReturnConsumed = { handle[NavResults.VIEWER_INDEX] = null },
                            onOpen = { index, fileId -> nav.navigate(ViewerRoute(index, fileId)) },
                            onSignedOut = { nav.navigateClearing(AuthGraph) },
                            onSwitchServer = { nav.navigate(ConnectRoute) },
                            onSearch = { nav.navigate(SearchRoute) },
                        )
                    }
                }
                composable<ViewerRoute> { entry ->
                    val route = entry.toRoute<ViewerRoute>()
                    val vm = viewModel {
                        ViewerViewModel(graph.core, graph.timeline, graph.viewerCache, cleanupScope = graph.appScope, startIndex = route.index)
                    }
                    CompositionLocalProvider(LocalNavAnimatedScope provides this) {
                        ViewerScreen(
                            vm = vm,
                            startIndex = route.index,
                            onClose = { index ->
                                nav.previousBackStackEntry?.savedStateHandle?.set(NavResults.VIEWER_INDEX, index)
                                nav.popBackStack()
                            },
                        )
                    }
                }
                // search, people and their viewer (MlNav.kt)
                mlDestinations(graph, nav)
            }
        }
    }
}

@Composable
private fun AuthEvents(vm: SignInViewModel, nav: NavHostController, graph: AppGraph) {
    SignInEvents(
        vm = vm,
        onEmailCode = { nav.navigate(EmailCodeRoute) },
        onTotp = { nav.navigate(TotpRoute(it)) },
        onPasskey = { session, url -> nav.navigate(PasskeyRoute(session, url)) { launchSingleTop = true } },
        onRecovery = { nav.navigate(RecoveryRoute) },
        onDone = {
            // Possibly another account or server than before: no decrypted image of the old one may show.
            graph.clearDecrypted()
            nav.navigateClearing(TimelineRoute)
        },
        onRestart = { nav.popBackStack<SignInRoute>(inclusive = false) },
    )
}

/** The sign-in view model lives as long as the auth graph, so every step shares it. */
@Composable
private fun NavHostController.signInViewModel(entry: NavBackStackEntry, graph: AppGraph): SignInViewModel {
    val parent = remember(entry) { getBackStackEntry<AuthGraph>() }
    return viewModel(parent) { SignInViewModel(graph.core) }
}

private inline fun <reified T : Any> NavHostController.hasRoute(): Boolean =
    runCatching { getBackStackEntry<T>() }.isSuccess

/** Navigates to [route] and drops everything before it. */
private fun NavHostController.navigateClearing(route: Any) {
    navigate(route) {
        popUpTo(graph.id) { inclusive = true }
        launchSingleTop = true
    }
}
