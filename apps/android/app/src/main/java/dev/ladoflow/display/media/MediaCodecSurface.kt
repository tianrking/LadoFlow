package dev.ladoflow.display.media

import android.view.SurfaceHolder
import android.view.SurfaceView
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.viewinterop.AndroidView

/** Compose-owned SurfaceView that forwards only Surface lifecycle to [decoder]. */
@Composable
fun MediaCodecSurface(
    decoder: VideoDecoder,
    modifier: Modifier = Modifier,
) {
    val callback = remember(decoder) {
        object : SurfaceHolder.Callback {
            override fun surfaceCreated(holder: SurfaceHolder) {
                decoder.setOutputSurface(holder.surface)
            }

            override fun surfaceChanged(
                holder: SurfaceHolder,
                format: Int,
                width: Int,
                height: Int,
            ) = Unit

            override fun surfaceDestroyed(holder: SurfaceHolder) {
                decoder.setOutputSurface(null)
            }
        }
    }

    AndroidView(
        factory = { context ->
            SurfaceView(context).also { surfaceView ->
                surfaceView.holder.addCallback(callback)
            }
        },
        modifier = modifier,
    )

    DisposableEffect(decoder, callback) {
        onDispose { decoder.setOutputSurface(null) }
    }
}
