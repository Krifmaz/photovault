package app.photovault.android

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import app.photovault.android.ui.nav.AppNav
import app.photovault.android.ui.nav.startDestination
import app.photovault.android.ui.theme.PhotovaultTheme
import app.photovault.android.ui.theme.palette

/** The single activity: Compose all the way down, navigation in [AppNav]. */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        val graph = appGraph
        setContent {
            PhotovaultTheme {
                Surface(Modifier.fillMaxSize(), color = MaterialTheme.palette.bg, contentColor = MaterialTheme.palette.text) {
                    val start = remember { startDestination(graph) }
                    AppNav(graph, start)
                }
            }
        }
    }
}
