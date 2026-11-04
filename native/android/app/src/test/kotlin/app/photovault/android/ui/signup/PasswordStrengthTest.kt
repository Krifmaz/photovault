package app.photovault.android.ui.signup

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class PasswordStrengthTest {
    private fun level(p: String) = PasswordStrengthEstimator.level(p)

    @Test
    fun weak() {
        for (p in listOf("", "a", "password", "12345678", "abcdefgh", "aaaaaaaaaaaa", "qwertyuiop", "Photovault")) {
            assertEquals(p, PasswordStrength.Weak, level(p))
        }
    }

    @Test
    fun moderate() {
        for (p in listOf("secret12", "blue-fox9", "mangoes1")) {
            assertEquals(p, PasswordStrength.Moderate, level(p))
        }
    }

    @Test
    fun strong() {
        for (p in listOf("correct horse battery staple", "Tr0ub4dor&3x", "Correct horse battery staple 7")) {
            assertEquals(p, PasswordStrength.Strong, level(p))
        }
    }

    @Test
    fun longerIsStronger() {
        val a = PasswordStrengthEstimator.estimate("sunset42")
        val b = PasswordStrengthEstimator.estimate("sunset42 river")
        assertTrue(b > a)
        assertTrue(PasswordStrengthEstimator.estimate("x".repeat(200)) < PasswordStrengthEstimator.MILD)
    }
}
