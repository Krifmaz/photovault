package app.photovault.android.ui.signup

import kotlin.math.ln
import kotlin.math.min

/** The hint under the new-password field, with the stock apps' thresholds (0.4 mild, 0.7 strong). */
enum class PasswordStrength { Weak, Moderate, Strong }

/**
 * A small password strength estimate in 0..1, in the spirit of the stock apps' `estimatePasswordStrength`:
 * roughly the bits of entropy of the password (length times the bits per character of the character classes
 * it uses), where repeated characters and runs like "abcd" or "4321" count for little, and very common
 * passwords score almost nothing. 90 bits or more is 1.
 */
object PasswordStrengthEstimator {
    const val MILD = 0.4f
    const val STRONG = 0.7f

    private val common = setOf(
        "password", "password1", "password123", "passw0rd", "12345678", "123456789", "1234567890",
        "qwerty", "qwertyui", "qwertyuiop", "iloveyou", "11111111", "00000000", "abc12345", "letmein",
        "welcome", "admin123", "sunshine", "princess", "football", "baseball", "dragon", "monkey",
        "photovault", "photos", "trustno1", "asdfghjkl", "zxcvbnm",
    )

    fun estimate(password: String): Float {
        if (password.isEmpty()) return 0f
        if (password.lowercase() in common) return 0.05f
        var pool = 0
        if (password.any { it.isLowerCase() }) pool += 26
        if (password.any { it.isUpperCase() }) pool += 26
        if (password.any { it.isDigit() }) pool += 10
        if (password.any { !it.isLetterOrDigit() && it.code < 128 }) pool += 33
        if (password.any { it.code >= 128 }) pool += 100
        if (pool == 0) pool = 26
        // Characters that repeat or continue a run add a quarter of their worth.
        var effective = 0f
        for (i in password.indices) {
            val c = password[i]
            val prev = if (i > 0) password[i - 1] else null
            effective += if (prev != null && (c == prev || c.code == prev.code + 1 || c.code == prev.code - 1)) 0.25f else 1f
        }
        val distinct = password.toSet().size
        if (distinct <= 2) effective = min(effective, 2f)
        val bits = effective * (ln(pool.toDouble()) / ln(2.0)).toFloat()
        return min(1f, bits / 90f)
    }

    fun level(password: String): PasswordStrength {
        val s = estimate(password)
        return when {
            s >= STRONG -> PasswordStrength.Strong
            s >= MILD -> PasswordStrength.Moderate
            else -> PasswordStrength.Weak
        }
    }
}
