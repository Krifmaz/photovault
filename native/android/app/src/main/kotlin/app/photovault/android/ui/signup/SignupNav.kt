package app.photovault.android.ui.signup

import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.navigation.NavBackStackEntry
import androidx.navigation.NavGraphBuilder
import androidx.navigation.NavHostController
import androidx.navigation.compose.composable
import androidx.navigation.compose.navigation
import app.photovault.android.AppGraph
import app.photovault.android.ui.nav.SignupCodeRoute
import app.photovault.android.ui.nav.SignupEmailRoute
import app.photovault.android.ui.nav.SignupGraph
import app.photovault.android.ui.nav.SignupPasswordRoute
import app.photovault.android.ui.nav.SignupRecoveryRoute

/**
 * The create-account screens: email, password, email code, recovery key. [onDone] runs once the new account
 * is signed in (AppNav opens the timeline).
 */
fun NavGraphBuilder.signupGraph(nav: NavHostController, graph: AppGraph, onDone: () -> Unit) {
    navigation<SignupGraph>(startDestination = SignupEmailRoute) {
        composable<SignupEmailRoute> { entry ->
            val vm = nav.signupViewModel(entry, graph)
            Events(vm, nav, onDone)
            SignupEmailScreen(vm, onBack = { nav.popBackStack() })
        }
        composable<SignupPasswordRoute> { entry ->
            val vm = nav.signupViewModel(entry, graph)
            Events(vm, nav, onDone)
            SignupPasswordScreen(vm, onBack = { nav.popBackStack() })
        }
        composable<SignupCodeRoute> { entry ->
            val vm = nav.signupViewModel(entry, graph)
            Events(vm, nav, onDone)
            SignupCodeScreen(vm, onBack = { nav.popBackStack() })
        }
        composable<SignupRecoveryRoute> { entry ->
            val vm = nav.signupViewModel(entry, graph)
            Events(vm, nav, onDone)
            SignupRecoveryKeyScreen(vm, onBack = { nav.popBackStack<SignupPasswordRoute>(inclusive = false) })
        }
    }
}

@Composable
private fun Events(vm: SignupViewModel, nav: NavHostController, onDone: () -> Unit) {
    SignupEvents(
        vm = vm,
        onPassword = { nav.navigate(SignupPasswordRoute) { launchSingleTop = true } },
        onCode = { nav.navigate(SignupCodeRoute) { launchSingleTop = true } },
        onRecoveryKey = {
            // The code is used up: Back from the recovery key goes to the password, not the code.
            nav.navigate(SignupRecoveryRoute) {
                popUpTo<SignupPasswordRoute>()
                launchSingleTop = true
            }
        },
        onDone = onDone,
        onRestart = { nav.popBackStack<SignupEmailRoute>(inclusive = false) },
    )
}

/** The sign-up view model lives as long as the sign-up graph, so every step shares it. */
@Composable
private fun NavHostController.signupViewModel(entry: NavBackStackEntry, graph: AppGraph): SignupViewModel {
    val parent = remember(entry) { getBackStackEntry<SignupGraph>() }
    return viewModel(parent) { SignupViewModel(graph.core, graph.signup) }
}
