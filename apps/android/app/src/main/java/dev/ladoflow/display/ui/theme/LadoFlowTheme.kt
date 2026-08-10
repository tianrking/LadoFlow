package dev.ladoflow.display.ui.theme

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color

val LadoNavy = Color(0xFF07131F)
val LadoSurface = Color(0xFF0C1B29)
val LadoSurfaceRaised = Color(0xFF132536)
val LadoCyan = Color(0xFF30D5C8)
val LadoCoral = Color(0xFFFF7B63)
val LadoBlue = Color(0xFF78A9FF)
val LadoText = Color(0xFFE9F2F8)
val LadoMuted = Color(0xFF9FB2C1)

private val DarkColors = darkColorScheme(
    primary = LadoCyan,
    onPrimary = LadoNavy,
    secondary = LadoCoral,
    onSecondary = LadoNavy,
    tertiary = LadoBlue,
    background = LadoNavy,
    onBackground = LadoText,
    surface = LadoSurface,
    onSurface = LadoText,
    surfaceVariant = LadoSurfaceRaised,
    onSurfaceVariant = LadoMuted,
    error = Color(0xFFFFB4AB),
    onError = Color(0xFF690005),
)

private val LightColors = lightColorScheme(
    primary = Color(0xFF006A64),
    onPrimary = Color.White,
    secondary = Color(0xFF9B3F2F),
    onSecondary = Color.White,
    background = Color(0xFFF5FAFC),
    onBackground = Color(0xFF102027),
    surface = Color.White,
    onSurface = Color(0xFF102027),
    surfaceVariant = Color(0xFFDCE8ED),
    onSurfaceVariant = Color(0xFF3F525A),
)

@Composable
fun LadoFlowTheme(
    darkTheme: Boolean = isSystemInDarkTheme(),
    content: @Composable () -> Unit,
) {
    MaterialTheme(
        colorScheme = if (darkTheme) DarkColors else LightColors,
        typography = MaterialTheme.typography,
        content = content,
    )
}
