package app.photovault.android.ui.people

import androidx.annotation.DrawableRes
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import app.photovault.android.R
import app.photovault.android.data.ml.Person
import app.photovault.android.ui.image.FaceThumb
import app.photovault.android.ui.image.Thumb
import app.photovault.android.ui.theme.EncoreRadius
import app.photovault.android.ui.theme.Spacing
import app.photovault.android.ui.theme.palette
import coil3.compose.AsyncImage

/**
 * A round face crop (core's `face_thumbnail` through Coil, see FaceThumbnails.kt). Without a face it shows a
 * person glyph on the panel colour. Flat: no shadow, no blur.
 */
@Composable
fun FaceAvatar(faceId: String?, size: Dp, modifier: Modifier = Modifier) {
    val p = MaterialTheme.palette
    Box(
        modifier
            .size(size)
            .clip(CircleShape)
            .background(p.panel2),
        contentAlignment = Alignment.Center,
    ) {
        if (faceId != null) {
            AsyncImage(
                model = FaceThumb(faceId),
                contentDescription = null,
                contentScale = ContentScale.Crop,
                modifier = Modifier.fillMaxSize(),
            )
        } else {
            Icon(painterResource(R.drawable.ic_account), contentDescription = null, tint = p.faint, modifier = Modifier.size(size / 2))
        }
    }
}

/** A square file thumbnail with rounded corners (album covers, suggestion samples, suggestion rows). */
@Composable
fun SmallThumb(fileId: Long?, size: Dp, modifier: Modifier = Modifier, @DrawableRes fallback: Int = R.drawable.ic_image) {
    val p = MaterialTheme.palette
    Box(
        modifier
            .size(size)
            .clip(RoundedCornerShape(EncoreRadius.small))
            .background(p.panel2),
        contentAlignment = Alignment.Center,
    ) {
        if (fileId != null) {
            AsyncImage(model = Thumb(fileId), contentDescription = null, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize())
        } else {
            Icon(painterResource(fallback), contentDescription = null, tint = p.accent2, modifier = Modifier.size(size / 2))
        }
    }
}

/** A person's display name: the name, or "Add a name" for an unnamed cluster. */
@Composable
fun personLabel(person: Person?): String =
    person?.name ?: stringResource(R.string.people_unnamed)

/** A pill-shaped choice (places, file types, magic prompts). */
@Composable
fun Pill(text: String, onClick: () -> Unit, modifier: Modifier = Modifier, @DrawableRes icon: Int? = null) {
    val p = MaterialTheme.palette
    val shape = RoundedCornerShape(EncoreRadius.pill)
    Row(
        modifier
            .heightIn(min = 36.dp)
            .clip(shape)
            .background(p.panel, shape)
            .border(1.dp, p.border, shape)
            .clickable(onClick = onClick)
            .padding(horizontal = Spacing.lg, vertical = Spacing.sm),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(Spacing.sm),
    ) {
        if (icon != null) Icon(painterResource(icon), contentDescription = null, tint = p.accent2, modifier = Modifier.size(16.dp))
        Text(text, style = MaterialTheme.typography.labelLarge, color = p.text, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}
