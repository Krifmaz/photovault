package app.photovault.android.ui.nav

import kotlinx.serialization.Serializable

/** Nested graph of the create-account screens; [app.photovault.android.ui.signup.SignupViewModel] is scoped to it. */
@Serializable
data object SignupGraph

@Serializable
data object SignupEmailRoute

@Serializable
data object SignupPasswordRoute

@Serializable
data object SignupCodeRoute

@Serializable
data object SignupRecoveryRoute
