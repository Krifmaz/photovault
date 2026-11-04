package app.photovault.android.ui.hidden

import android.app.KeyguardManager
import android.content.Context
import android.hardware.biometrics.BiometricManager
import android.hardware.biometrics.BiometricPrompt
import android.os.Build
import android.os.CancellationSignal
import androidx.annotation.RequiresApi

/**
 * The device's own lock (fingerprint, face or the screen-lock PIN/pattern/password) in front of hidden
 * albums, through the platform's BiometricPrompt: no extra library and no FragmentActivity needed.
 * Android 9 has no credential fallback in BiometricPrompt, so there the screen uses KeyguardManager's
 * confirm-credential activity instead.
 */
object DeviceLock {
    /** Whether a screen lock is set; without one there is nothing to check (the screen says so). */
    fun isSecure(context: Context): Boolean =
        context.getSystemService(KeyguardManager::class.java)?.isDeviceSecure == true

    /** Shows the prompt; [onResult] gets true once the user passed it, false if they cancelled or failed. */
    @RequiresApi(Build.VERSION_CODES.Q)
    fun authenticate(
        context: Context,
        title: String,
        subtitle: String,
        onResult: (Boolean) -> Unit,
    ): CancellationSignal {
        val cancel = CancellationSignal()
        val builder = BiometricPrompt.Builder(context).setTitle(title).setSubtitle(subtitle)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            builder.setAllowedAuthenticators(
                BiometricManager.Authenticators.BIOMETRIC_WEAK or BiometricManager.Authenticators.DEVICE_CREDENTIAL,
            )
        } else {
            @Suppress("DEPRECATION")
            builder.setDeviceCredentialAllowed(true)
        }
        val callback = object : BiometricPrompt.AuthenticationCallback() {
            override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) = onResult(true)
            override fun onAuthenticationError(errorCode: Int, errString: CharSequence) = onResult(false)
        }
        builder.build().authenticate(cancel, context.mainExecutor, callback)
        return cancel
    }
}
