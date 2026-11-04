package app.photovault.android.data

import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import java.io.File
import java.security.GeneralSecurityException
import java.security.KeyStore
import java.security.SecureRandom
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * The 32-byte key that seals core's local store (session token, account keys, collection and file keys).
 * It is random, handed to core in memory only (`Core.withDeviceKey`) and stored on disk only wrapped with
 * AES-GCM under a non-exportable key in the Android Keystore, so a copy of the app's data directory alone
 * does not open the store. If the Keystore key is gone (device reset of the keystore, restored data), a new
 * device key is made and core drops the unreadable account data: the user signs in again.
 */
internal object DeviceKey {
    private const val ALIAS = "photovault-core-device-key"
    private const val FILE = "device.key.wrapped"
    private const val KEY_BYTES = 32
    private const val IV_BYTES = 12
    private const val TAG_BITS = 128

    /** Loads (or creates) the device key; the caller wipes the returned array after use. */
    @Throws(GeneralSecurityException::class, java.io.IOException::class)
    fun load(dir: File): ByteArray {
        val keyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        val wrapped = File(dir, FILE)
        val existing = keyStore.getKey(ALIAS, null) as? SecretKey
        if (existing != null && wrapped.isFile) {
            try {
                val blob = wrapped.readBytes()
                if (blob.size > IV_BYTES) {
                    val cipher = Cipher.getInstance("AES/GCM/NoPadding")
                    cipher.init(Cipher.DECRYPT_MODE, existing, GCMParameterSpec(TAG_BITS, blob, 0, IV_BYTES))
                    val key = cipher.doFinal(blob, IV_BYTES, blob.size - IV_BYTES)
                    if (key.size == KEY_BYTES) return key
                    key.fill(0)
                }
            } catch (e: GeneralSecurityException) {
                // unreadable: replaced below
            }
        }
        val wrapping = existing ?: generate()
        val key = ByteArray(KEY_BYTES).also { SecureRandom().nextBytes(it) }
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, wrapping)
        val blob = cipher.iv + cipher.doFinal(key)
        val tmp = File(dir, "$FILE.tmp")
        tmp.outputStream().use { out ->
            out.write(blob)
            out.fd.sync()
        }
        if (!tmp.renameTo(wrapped)) {
            key.fill(0)
            throw java.io.IOException("cannot save $FILE")
        }
        return key
    }

    private fun generate(): SecretKey {
        val gen = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        gen.init(
            KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build(),
        )
        return gen.generateKey()
    }
}
