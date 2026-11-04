@file:OptIn(ExperimentalMaterial3Api::class)

package app.photovault.android.ui.people

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.photovault.android.R
import app.photovault.android.ui.components.PrimaryButton
import app.photovault.android.ui.components.ProblemText
import app.photovault.android.ui.components.SecondaryButton
import app.photovault.android.ui.theme.EncoreRadius
import app.photovault.android.ui.theme.Spacing
import app.photovault.android.ui.theme.palette

/** "Review suggestions": is this group of faces the same person? Yes joins it, no stops suggesting it. */
@Composable
fun SuggestionsScreen(vm: SuggestionsViewModel, onBack: () -> Unit) {
    val state by vm.state.collectAsStateWithLifecycle()
    val p = MaterialTheme.palette
    val name = personLabel(state.person)

    Scaffold(
        containerColor = p.bg,
        topBar = {
            TopAppBar(
                colors = TopAppBarDefaults.topAppBarColors(containerColor = p.bg, titleContentColor = p.text),
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(painterResource(R.drawable.ic_arrow_back), contentDescription = stringResource(R.string.back), tint = p.text)
                    }
                },
                title = { Text(stringResource(R.string.person_review), style = MaterialTheme.typography.titleLarge) },
            )
        },
    ) { padding ->
        if (!state.loaded) {
            Box(Modifier.fillMaxSize().padding(padding), contentAlignment = Alignment.Center) { CircularProgressIndicator(color = p.accent2) }
            return@Scaffold
        }
        LazyColumn(
            modifier = Modifier.fillMaxSize(),
            contentPadding = PaddingValues(
                start = Spacing.xl,
                end = Spacing.xl,
                top = padding.calculateTopPadding() + Spacing.md,
                bottom = padding.calculateBottomPadding() + Spacing.section,
            ),
            verticalArrangement = Arrangement.spacedBy(Spacing.lg),
        ) {
            state.error?.let { error -> item(key = "error") { ProblemText(error) } }
            if (state.cards.isEmpty()) {
                item(key = "done") {
                    Text(
                        stringResource(R.string.suggestions_done),
                        style = MaterialTheme.typography.bodyMedium,
                        color = p.muted,
                        textAlign = TextAlign.Center,
                        modifier = Modifier.fillMaxWidth().padding(Spacing.section),
                    )
                }
            }
            items(state.cards, key = { it.cluster.id }) { card ->
                SuggestionCardView(
                    card = card,
                    name = name,
                    answering = card.cluster.id in state.answering,
                    onYes = { vm.answer(card, accept = true) },
                    onNo = { vm.answer(card, accept = false) },
                )
            }
        }
    }
}

@Composable
private fun SuggestionCardView(card: SuggestionCard, name: String, answering: Boolean, onYes: () -> Unit, onNo: () -> Unit) {
    val p = MaterialTheme.palette
    val shape = RoundedCornerShape(EncoreRadius.card)
    Column(
        Modifier
            .fillMaxWidth()
            .clip(shape)
            .background(p.panel, shape)
            .border(1.dp, p.border, shape)
            .padding(Spacing.lg),
        verticalArrangement = Arrangement.spacedBy(Spacing.lg),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Spacing.lg)) {
            FaceAvatar(card.cluster.cover?.faceId, 56.dp)
            Column(Modifier.weight(1f)) {
                Text(stringResource(R.string.suggestions_question, name), style = MaterialTheme.typography.titleMedium, color = p.text)
                Text(
                    pluralStringResource(R.plurals.search_photos, card.cluster.fileCount.toInt(), card.cluster.fileCount.toInt()),
                    style = MaterialTheme.typography.bodySmall,
                    color = p.muted,
                )
            }
        }
        if (card.sampleFileIds.isNotEmpty()) {
            Row(horizontalArrangement = Arrangement.spacedBy(Spacing.sm)) {
                card.sampleFileIds.forEach { id -> SmallThumb(id, 64.dp) }
            }
        }
        Row(horizontalArrangement = Arrangement.spacedBy(Spacing.md)) {
            SecondaryButton(stringResource(R.string.suggestions_no), onClick = onNo, modifier = Modifier.weight(1f), enabled = !answering)
            PrimaryButton(stringResource(R.string.suggestions_yes), onClick = onYes, modifier = Modifier.weight(1f), enabled = !answering, loading = answering)
        }
    }
}
