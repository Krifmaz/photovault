package app.photovault.android.ui.map

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.photovault.android.data.CoreRepository
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** The photos on the map, projected once; clusters per integer zoom are computed off the main thread. */
class MapViewModel(
    private val core: CoreRepository,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    private val compute: CoroutineDispatcher = Dispatchers.Default,
) : ViewModel() {

    data class State(
        val loaded: Boolean = false,
        /** Every located photo, newest first. */
        val points: List<WorldPoint> = emptyList(),
        /** Bounding box of [points] in world coordinates (null when there are none). */
        val bounds: DoubleArray? = null,
    )

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    /** Clusters for the zoom the map currently shows ([clustersFor]). */
    private val _clusters = MutableStateFlow<Pair<Int, List<Cluster>>>(-1 to emptyList())
    val clusters: StateFlow<Pair<Int, List<Cluster>>> = _clusters.asStateFlow()

    /** Photos inside the visible part of the map (or the tapped spot), newest first. */
    private val _inView = MutableStateFlow<List<WorldPoint>>(emptyList())
    val inView: StateFlow<List<WorldPoint>> = _inView.asStateFlow()

    private val cache = HashMap<Int, List<Cluster>>()
    private var cellPx = 0f
    private var clusterJob: Job? = null
    private var viewJob: Job? = null

    init {
        viewModelScope.launch { load() }
    }

    suspend fun load() {
        val raw = withContext(io) { core.mapPoints() }
        val (points, bounds) = withContext(compute) {
            val pts = raw.map { WorldPoint(it.fileId, MapMath.worldX(it.longitude), MapMath.worldY(it.latitude), it.takenAtUs) }
            val b = if (pts.isEmpty()) null else doubleArrayOf(
                pts.minOf { it.x }, pts.minOf { it.y }, pts.maxOf { it.x }, pts.maxOf { it.y },
            )
            pts to b
        }
        cache.clear()
        _state.value = State(loaded = true, points = points, bounds = bounds)
        _inView.value = points
    }

    /** Asks for the clusters at integer [zoom] with [cell] pixel cells (the marker size). */
    fun clustersFor(zoom: Int, cell: Float) {
        if (cell != cellPx) { cache.clear(); cellPx = cell }
        if (_clusters.value.first == zoom && cache.containsKey(zoom)) return
        cache[zoom]?.let { _clusters.value = zoom to it; return }
        val points = _state.value.points
        clusterJob?.cancel()
        clusterJob = viewModelScope.launch {
            val list = withContext(compute) { Clustering.cluster(points, zoom, cell) }
            cache[zoom] = list
            _clusters.value = zoom to list
        }
    }

    /** The map settled on a new view: list the photos inside it (world coordinates, x may wrap). */
    fun viewChanged(minX: Double, minY: Double, maxX: Double, maxY: Double) {
        val points = _state.value.points
        viewJob?.cancel()
        viewJob = viewModelScope.launch {
            _inView.value = withContext(compute) {
                if (maxX - minX >= 1.0) Clustering.inside(points, -1.0, minY, 2.0, maxY)
                else {
                    val a = Clustering.inside(points, minX, minY, maxX, maxY)
                    // the view crosses the antimeridian: add the wrapped part
                    val wrapped = when {
                        minX < 0 -> Clustering.inside(points, minX + 1, minY, 1.0, maxY)
                        maxX > 1 -> Clustering.inside(points, 0.0, minY, maxX - 1, maxY)
                        else -> emptyList()
                    }
                    if (wrapped.isEmpty()) a else (a + wrapped).sortedByDescending { it.takenAtUs }
                }
            }
        }
    }

    /** A spot that zooming cannot split was tapped: list just its photos. */
    fun showSpot(c: Cluster) {
        viewJob?.cancel()
        val points = _state.value.points
        viewJob = viewModelScope.launch {
            _inView.value = withContext(compute) {
                Clustering.inside(points, c.minX - 1e-9, c.minY - 1e-9, c.maxX + 1e-9, c.maxY + 1e-9)
            }
        }
    }
}
