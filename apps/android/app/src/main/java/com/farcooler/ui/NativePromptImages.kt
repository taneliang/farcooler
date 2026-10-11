package com.farcooler.ui

import android.graphics.Bitmap
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.rememberTransformableState
import androidx.compose.foundation.gestures.transformable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.outlined.Image
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import com.farcooler.model.AgentRow
import com.farcooler.net.PromptImageStore
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** The pane's prompt images, where its runner serves them (ov-458); null, and no thumbnails are drawn. */
val LocalPromptImages = staticCompositionLocalOf<PromptImageStore?> { null }

/** The side of a thumbnail. */
private val ThumbnailSide = 72.dp

/** The longest edge a full-size view decodes to: sharp on a phone's screen without holding a camera roll's megapixels. */
private const val FULL_EDGE = 2048

/**
 * A turn's images above its message, as thumbnails that open at full size
 * (ov-458); the Mac's `PromptImageStrip`. They wrap onto another row where the
 * screen is too narrow for them all, each row flush with the message's trailing
 * edge.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun PromptImageStrip(row: String, images: List<AgentRow.PromptImage>, store: PromptImageStore, startOpen: Int? = null) {
    var opened by remember { mutableStateOf(startOpen) }
    FlowRow(
        Modifier.fillMaxWidth().testTag("native-prompt-images"),
        horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.End),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        images.indices.forEach { i ->
            LaunchedEffect(row, i) { store.load(row, i) }
            Thumbnail(i, images.size, store.state(row, i)) { opened = i }
        }
    }
    opened?.let { start ->
        val shown = images.indices.mapNotNull { i -> (store.state(row, i) as? PromptImageStore.State.Shown)?.let { i to it.bytes } }
        val at = shown.indexOfFirst { it.first == start }
        if (at >= 0) FullSizeImages(shown, at, images.size) { opened = null } else opened = null
    }
}

@Composable
private fun Thumbnail(i: Int, count: Int, state: PromptImageStore.State, open: () -> Unit) {
    val label = if (count == 1) "Image" else "Image ${i + 1} of $count"
    val shape = RoundedCornerShape(Radius.small)
    when (state) {
        is PromptImageStore.State.Shown -> {
            // Decoded small, once, off the main thread, never at full size on a redraw.
            val picture by produceState<Bitmap?>(null, state) {
                value = withContext(Dispatchers.Default) { chipPicture(state.bytes, side = 256) }
            }
            val bitmap = picture
            Box(
                Modifier
                    .size(ThumbnailSide)
                    .clip(shape)
                    .background(MaterialTheme.colorScheme.surfaceContainerHigh)
                    .clickable(onClickLabel = "Open image", role = Role.Button, onClick = open)
                    .semantics { contentDescription = label }
                    .testTag("native-prompt-image"),
            ) {
                if (bitmap != null) {
                    Image(bitmap.asImageBitmap(), contentDescription = null, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize())
                }
            }
        }
        PromptImageStore.State.Loading -> Placeholder("$label, loading", shape) {
            CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
        }
        PromptImageStore.State.Failed -> Placeholder("$label, couldn’t be loaded", shape) {
            Icon(Icons.Outlined.Image, null, tint = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

@Composable
private fun Placeholder(description: String, shape: RoundedCornerShape, content: @Composable () -> Unit) {
    Box(
        Modifier
            .size(ThumbnailSide)
            .clip(shape)
            .background(MaterialTheme.colorScheme.surfaceContainerHigh)
            .semantics { contentDescription = description },
        contentAlignment = Alignment.Center,
    ) { content() }
}

/**
 * The prompt's images at full size, over the conversation: swipe between them,
 * pinch to zoom, Back or the close button to leave.
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun FullSizeImages(images: List<Pair<Int, ByteArray>>, start: Int, count: Int, close: () -> Unit) {
    Dialog(onDismissRequest = close, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = 0.92f)).testTag("native-prompt-image-full")) {
            val pager = rememberPagerState(initialPage = start) { images.size }
            HorizontalPager(pager, Modifier.fillMaxSize(), userScrollEnabled = images.size > 1) { page ->
                val (index, bytes) = images[page]
                ZoomableImage(bytes, if (count == 1) "Image" else "Image ${index + 1} of $count", close)
            }
            IconButton(
                onClick = close,
                modifier = Modifier.align(Alignment.TopEnd).padding(8.dp).testTag("native-prompt-image-close"),
                colors = IconButtonDefaults.iconButtonColors(containerColor = Color.Black.copy(alpha = 0.5f), contentColor = Color.White),
            ) { Icon(Icons.Filled.Close, contentDescription = "Close") }
        }
    }
}

@Composable
private fun ZoomableImage(bytes: ByteArray, description: String, close: () -> Unit) {
    val picture by produceState<Bitmap?>(null, bytes) { value = withContext(Dispatchers.Default) { chipPicture(bytes, FULL_EDGE) } }
    var scale by remember { mutableFloatStateOf(1f) }
    val state = rememberTransformableState { zoom, _, _ -> scale = (scale * zoom).coerceIn(1f, 5f) }
    Box(Modifier.fillMaxSize().clickable(onClickLabel = "Close", onClick = close), contentAlignment = Alignment.Center) {
        val bitmap = picture
        if (bitmap == null) {
            CircularProgressIndicator(color = Color.White)
        } else {
            Image(
                bitmap.asImageBitmap(),
                contentDescription = description,
                contentScale = ContentScale.Fit,
                modifier = Modifier
                    .fillMaxSize()
                    .padding(16.dp)
                    .transformable(state)
                    .graphicsLayer(scaleX = scale, scaleY = scale),
            )
        }
    }
}
