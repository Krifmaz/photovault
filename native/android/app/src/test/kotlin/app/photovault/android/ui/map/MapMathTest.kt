package app.photovault.android.ui.map

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class MapMathTest {
    @Test
    fun projectsAndInvertsWebMercator() {
        assertEquals(0.5, MapMath.worldX(0.0), 1e-12)
        assertEquals(0.5, MapMath.worldY(0.0), 1e-12)
        assertEquals(0.0, MapMath.worldX(-180.0), 1e-12)
        // OSM tile of Paris at zoom 10 is x=518, y=352
        val z = 1 shl 10
        assertEquals(518, (MapMath.worldX(2.35) * z).toInt())
        assertEquals(352, (MapMath.worldY(48.86) * z).toInt())
        for (lat in listOf(-60.0, -10.5, 0.0, 38.72, 64.15)) assertEquals(lat, MapMath.lat(MapMath.worldY(lat)), 1e-9)
        for (lon in listOf(-179.0, -9.14, 0.0, 135.77)) assertEquals(lon, MapMath.lon(MapMath.worldX(lon)), 1e-9)
        // the poles clamp instead of going to infinity
        assertEquals(0.0, MapMath.worldY(90.0), 1e-9)
        assertEquals(1.0, MapMath.worldY(-90.0), 1e-9)
    }

    @Test
    fun fitShowsTheWholeBoxAndCapsTheZoom() {
        val c = MapMath.fit(0.40, 0.30, 0.60, 0.40, 1080f, 1920f, 100f)
        assertEquals(0.5, c.x, 1e-12)
        assertEquals(0.35, c.y, 1e-12)
        val ws = MapMath.worldSize(c.zoom)
        assertTrue(0.2 * ws <= 1080 - 200 + 1e-6)
        assertTrue(0.1 * ws <= 1920 - 200 + 1e-6)
        // a single point does not zoom to the max
        assertEquals(16f, MapMath.fit(0.5, 0.5, 0.5, 0.5, 1080f, 1920f, 100f).zoom)
    }

    @Test
    fun tileZoomRoundsAndClamps() {
        assertEquals(3, MapMath.tileZoom(3.4f))
        assertEquals(4, MapMath.tileZoom(3.5f))
        assertEquals(19, MapMath.tileZoom(19f))
        assertEquals(0, MapMath.tileZoom(0.2f))
    }

    @Test
    fun cameraWrapsXAndClampsY() {
        val c = Camera(1.25, 1.3, 30f).normalized()
        assertEquals(0.25, c.x, 1e-12)
        assertEquals(1.0, c.y, 1e-12)
        assertEquals(MapMath.MAX_ZOOM, c.zoom)
    }

    @Test
    fun clustersGroupNearbyPointsWithTheNewestAsCover() {
        // newest first
        val pts = listOf(
            WorldPoint(1, 0.500000, 0.500000, 300),
            WorldPoint(2, 0.500001, 0.500001, 200),
            WorldPoint(3, 0.700000, 0.200000, 100),
        )
        val far = Clustering.cluster(pts, zoom = 2, cellPx = 64f)
        assertEquals(2, far.size)
        assertEquals(1L, far.first().coverId)
        assertEquals(2, far.first().count)
        val near = Clustering.cluster(pts, zoom = 22, cellPx = 64f)
        assertEquals(3, near.size)
        assertEquals(3, near.sumOf { it.count })
        assertTrue(Clustering.cluster(emptyList(), 5, 64f).isEmpty())
        assertTrue(Cluster(1, 0.5, 0.5, 4, 0.5, 0.5, 0.5, 0.5).isSpot)
    }

    @Test
    fun insideKeepsOrder() {
        val pts = (0 until 10).map { WorldPoint(it.toLong(), it / 10.0, 0.5, 100L - it) }
        assertEquals(listOf(2L, 3L, 4L), Clustering.inside(pts, 0.2, 0.0, 0.45, 1.0).map { it.fileId })
    }
}
