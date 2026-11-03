package app.photovault.android.data.fake

import java.io.ByteArrayOutputStream
import java.util.zip.CRC32
import java.util.zip.Deflater
import kotlin.math.abs
import kotlin.math.max
import kotlin.math.min
import kotlin.math.sqrt

/**
 * Placeholder pictures for [FakeCore], drawn in code and encoded as PNG with nothing but java.util.zip,
 * so they go through the same decode path (Coil) as real thumbnails and work in JVM unit tests.
 *
 * Each picture is a soft two-colour sky/ground gradient with a "sun" disc, seeded by the file id, so
 * neighbouring cells look different and the same id always looks the same.
 */
object PlaceholderPng {
    fun render(seed: Long, width: Int, height: Int): ByteArray {
        require(width in 1..4096 && height in 1..4096)
        val rnd = Lcg(seed * 2654435761L + 17)
        val hue = rnd.nextFloat()
        val top = hsv(hue, 0.35f + rnd.nextFloat() * 0.3f, 0.85f)
        val bottom = hsv((hue + 0.08f + rnd.nextFloat() * 0.2f) % 1f, 0.55f, 0.35f + rnd.nextFloat() * 0.25f)
        val ground = hsv((hue + 0.45f) % 1f, 0.45f, 0.25f + rnd.nextFloat() * 0.2f)
        val sun = hsv((hue + 0.12f) % 1f, 0.25f, 1f)
        val horizon = height * (0.55f + rnd.nextFloat() * 0.25f)
        val sunX = width * (0.2f + rnd.nextFloat() * 0.6f)
        val sunY = horizon * (0.3f + rnd.nextFloat() * 0.4f)
        val sunR = min(width, height) * (0.08f + rnd.nextFloat() * 0.1f)
        val waveAmp = height * 0.04f
        val waveLen = width / (1.5f + rnd.nextFloat() * 2f)

        val stride = width * 3 + 1
        val raw = ByteArray(stride * height)
        for (y in 0 until height) {
            val row = y * stride
            raw[row] = 0 // filter: none
            for (x in 0 until width) {
                val hill = horizon + waveAmp * kotlin.math.sin(x / waveLen * 6.2831855f).toFloat()
                var c: Int
                if (y >= hill) {
                    val t = ((y - hill) / max(1f, height - hill)).coerceIn(0f, 1f)
                    c = mix(ground, 0x000000, t * 0.5f)
                } else {
                    c = mix(top, bottom, (y / max(1f, hill)).coerceIn(0f, 1f))
                    val dx = x - sunX
                    val dy = y - sunY
                    val d = sqrt(dx * dx + dy * dy)
                    if (d < sunR) c = mix(c, sun, 0.9f)
                    else if (d < sunR * 2.2f) c = mix(c, sun, 0.25f * (1f - (d - sunR) / (sunR * 1.2f)))
                }
                val i = row + 1 + x * 3
                raw[i] = (c shr 16).toByte()
                raw[i + 1] = (c shr 8).toByte()
                raw[i + 2] = c.toByte()
            }
        }
        return encode(width, height, raw)
    }

    private fun encode(width: Int, height: Int, raw: ByteArray): ByteArray {
        val out = ByteArrayOutputStream(raw.size / 4 + 64)
        out.write(byteArrayOf(0x89.toByte(), 'P'.code.toByte(), 'N'.code.toByte(), 'G'.code.toByte(), 13, 10, 26, 10))
        val ihdr = ByteArrayOutputStream()
        ihdr.writeInt(width); ihdr.writeInt(height)
        ihdr.write(8) // bit depth
        ihdr.write(2) // colour type: RGB
        ihdr.write(0); ihdr.write(0); ihdr.write(0)
        chunk(out, "IHDR", ihdr.toByteArray())
        val deflater = Deflater(Deflater.BEST_SPEED)
        deflater.setInput(raw)
        deflater.finish()
        val z = ByteArrayOutputStream()
        val buf = ByteArray(16 * 1024)
        while (!deflater.finished()) {
            val n = deflater.deflate(buf)
            z.write(buf, 0, n)
        }
        deflater.end()
        chunk(out, "IDAT", z.toByteArray())
        chunk(out, "IEND", ByteArray(0))
        return out.toByteArray()
    }

    private fun chunk(out: ByteArrayOutputStream, type: String, data: ByteArray) {
        out.writeInt(data.size)
        val typeBytes = type.toByteArray(Charsets.US_ASCII)
        out.write(typeBytes)
        out.write(data)
        val crc = CRC32()
        crc.update(typeBytes)
        crc.update(data)
        out.writeInt(crc.value.toInt())
    }

    private fun ByteArrayOutputStream.writeInt(v: Int) {
        write(v ushr 24 and 0xFF); write(v ushr 16 and 0xFF); write(v ushr 8 and 0xFF); write(v and 0xFF)
    }

    private fun mix(a: Int, b: Int, t: Float): Int {
        val u = t.coerceIn(0f, 1f)
        fun ch(shift: Int): Int {
            val ca = a shr shift and 0xFF
            val cb = b shr shift and 0xFF
            return (ca + (cb - ca) * u).toInt().coerceIn(0, 255)
        }
        return (ch(16) shl 16) or (ch(8) shl 8) or ch(0)
    }

    private fun hsv(h: Float, s: Float, v: Float): Int {
        val c = v * s
        val hp = (h % 1f) * 6f
        val x = c * (1 - abs(hp % 2 - 1))
        val (r, g, b) = when (hp.toInt()) {
            0 -> Triple(c, x, 0f)
            1 -> Triple(x, c, 0f)
            2 -> Triple(0f, c, x)
            3 -> Triple(0f, x, c)
            4 -> Triple(x, 0f, c)
            else -> Triple(c, 0f, x)
        }
        val m = v - c
        fun to8(f: Float) = ((f + m) * 255).toInt().coerceIn(0, 255)
        return (to8(r) shl 16) or (to8(g) shl 8) or to8(b)
    }

    /** Tiny deterministic generator (java.util.Random would do, but this keeps the output stable forever). */
    internal class Lcg(seed: Long) {
        private var state = seed xor 0x5DEECE66DL
        fun nextInt(): Int {
            state = state * 6364136223846793005L + 1442695040888963407L
            return (state ushr 33).toInt()
        }
        /** Uniform in [0, 1): the top 24 of the 31 bits [nextInt] yields. */
        fun nextFloat(): Float = (nextInt() ushr 7) / 16777216f
    }
}
