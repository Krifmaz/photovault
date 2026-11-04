package app.photovault.android

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import app.photovault.android.ui.nav.AppNav
import app.photovault.android.ui.nav.startDestination
import app.photovault.android.ui.theme.PhotovaultTheme
import app.photovault.android.ui.theme.palette
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** The single activity: Compose all the way down, navigation in [AppNav]. */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        val graph = appGraph
        setContent {
            PhotovaultTheme {
                Surface(Modifier.fillMaxSize().semantics { testTagsAsResourceId = true }, color = MaterialTheme.palette.bg, contentColor = MaterialTheme.palette.text) {
                    // Opening core (native library + local index) happens off the main thread; until it is
                    // ready the window shows the plain background (normally a few frames at most).
                    val start by produceState<Any?>(null, graph) {
                        value = withContext(Dispatchers.IO) { startDestination(graph) }
                    }
                    start?.let { AppNav(graph, it) }
                }
            }
        }
    }
}
