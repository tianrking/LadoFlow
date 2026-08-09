package dev.ladoflow.display.media

import android.content.Context
import android.view.SurfaceHolder
import android.view.SurfaceView
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.viewinterop.AndroidView
import dev.ladoflow.display.input.AndroidInputController

/** Compose-owned SurfaceView that forwards only Surface lifecycle to [decoder]. */
@Composable
fun MediaCodecSurface(
    decoder: VideoDecoder,
    modifier: Modifier = Modifier,
    inputController: AndroidInputController? = null,
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
            DecoderSurfaceView(context).also { surfaceView ->
                surfaceView.holder.addCallback(callback)
                surfaceView.installInputController(inputController)
            }
        },
        update = { surfaceView -> surfaceView.installInputController(inputController) },
        modifier = modifier,
    )

    DisposableEffect(decoder, callback) {
        onDispose { decoder.setOutputSurface(null) }
    }
}

private fun DecoderSurfaceView.installInputController(controller: AndroidInputController?) {
    isFocusable = controller != null
    isFocusableInTouchMode = controller != null
    setOnTouchListener(
        controller?.let { input ->
            { view, event ->
                if (event.actionMasked == android.view.MotionEvent.ACTION_DOWN) view.requestFocus()
                val handled = input.onTouchEvent(view.width, view.height, event)
                if (handled && event.actionMasked == android.view.MotionEvent.ACTION_UP) {
                    view.performClick()
                }
                handled
            }
        },
    )
    setOnGenericMotionListener(
        controller?.let { input ->
            { view, event -> input.onGenericMotionEvent(view.width, view.height, event) }
        },
    )
    setOnKeyListener(
        controller?.let { input ->
            { _, _, event -> input.onKeyEvent(event) }
        },
    )
    onFocusChangeListener = controller?.let { input ->
        android.view.View.OnFocusChangeListener { _, focused -> input.onFocusChanged(focused) }
    }
}

private class DecoderSurfaceView(context: Context) : SurfaceView(context) {
    override fun performClick(): Boolean {
        super.performClick()
        return true
    }
}
