package app.maine.android.ui

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp

/**
 * Maine visual language: monochrome-first surfaces in the spirit of
 * dot-island (plain black, white glyphs, one neutral outline, system
 * monospace for technical values, readable sans for names), with a single
 * restrained brand-yellow accent reserved for primary actions (the logo's
 * main color): yellow carries near-black text on both themes, so buttons
 * stay monochrome-adjacent instead of going purple.
 * Functional status colors (connected/connecting/failed) are the only
 * other hues, never decoration.
 */
object MaineColors {
    /** Brand yellow, the logo's main color. Near-black text on top, both themes. */
    val Accent = Color(0xFFFCEE0C)
    val AccentOn = Color(0xFF101014)

    /** Supporting text, dot-island's exact value. */
    val MutedDark = Color(0xFF8E8E93)
    val MutedLight = Color(0xFF5E5E66)

    /** Terminal surface is pure black on both themes. */
    val Terminal = Color.Black
    val TerminalText = Color(0xFFE8E8EA)

    val ConnectedDark = Color(0xFF3DDC84)
    val ConnectedLight = Color(0xFF178A4C)
    val Connecting = Color(0xFFFF9F0A)
    val FailedDark = Color(0xFFFF453A)
    val FailedLight = Color(0xFFC81E1E)
}

private val MaineDarkColors = darkColorScheme(
    primary = MaineColors.Accent,
    onPrimary = MaineColors.AccentOn,
    primaryContainer = Color(0xFF3A3404),
    onPrimaryContainer = Color(0xFFFFF6BF),
    background = Color(0xFF0B0B0E),
    onBackground = Color(0xFFF4F4F6),
    surface = Color(0xFF15151A),
    onSurface = Color(0xFFF4F4F6),
    surfaceVariant = Color(0xFF1E1E24),
    onSurfaceVariant = MaineColors.MutedDark,
    surfaceContainerHigh = Color(0xFF232329),
    outline = Color(0xFF2C2C33),
    outlineVariant = Color(0xFF1F1F25),
    error = MaineColors.FailedDark,
    onError = Color.White,
    errorContainer = Color(0xFF3A1518),
    onErrorContainer = Color(0xFFFFDAD6),
)

private val MaineLightColors = lightColorScheme(
    primary = MaineColors.Accent,
    onPrimary = MaineColors.AccentOn,
    primaryContainer = Color(0xFFFFF0A8),
    onPrimaryContainer = Color(0xFF3A3400),
    background = Color(0xFFF6F6F8),
    onBackground = Color(0xFF101014),
    surface = Color.White,
    onSurface = Color(0xFF101014),
    surfaceVariant = Color(0xFFECECF0),
    onSurfaceVariant = MaineColors.MutedLight,
    surfaceContainerHigh = Color(0xFFE4E4E9),
    outline = Color(0xFFD9D9DE),
    outlineVariant = Color(0xFFE8E8EC),
    error = MaineColors.FailedLight,
    onError = Color.White,
    errorContainer = Color(0xFFFDECEA),
    onErrorContainer = Color(0xFF5C1111),
)

/** System monospace for endpoints, states, command lines and terminal text. */
val MaineMono = FontFamily.Monospace

/**
 * Inset between the terminal sheet's edges and its contents — matching the gap
 * between the header card and the terminal, so the terminal does not look
 * crammed against the sides.
 */
val TerminalInset = 8.dp

/**
 * Selected-row text: brand yellow on dark, dark olive on light (yellow
 * body text on white is unreadable, so the hue deepens instead).
 */
@Composable
fun maineSelectionColor(): Color =
    if (isSystemInDarkTheme()) MaineColors.Accent else Color(0xFF6E5F00)

/** Focus rings stay on-brand without going invisible on white. */
@Composable
fun maineFieldColors() = androidx.compose.material3.OutlinedTextFieldDefaults.colors(
    focusedBorderColor = if (isSystemInDarkTheme()) MaineColors.Accent else Color(0xFF8A7500),
    focusedLabelColor = if (isSystemInDarkTheme()) MaineColors.Accent else Color(0xFF8A7500),
    cursorColor = if (isSystemInDarkTheme()) MaineColors.Accent else Color(0xFF8A7500),
)

private val MaineTypography = Typography()

private val MaineShapes = androidx.compose.material3.Shapes(
    extraSmall = androidx.compose.foundation.shape.RoundedCornerShape(8.dp),
    small = androidx.compose.foundation.shape.RoundedCornerShape(12.dp),
    medium = androidx.compose.foundation.shape.RoundedCornerShape(20.dp),
    large = androidx.compose.foundation.shape.RoundedCornerShape(24.dp),
    extraLarge = androidx.compose.foundation.shape.RoundedCornerShape(28.dp),
)

@Composable
fun MaineTheme(
    dark: Boolean = isSystemInDarkTheme(),
    content: @Composable () -> Unit,
) {
    MaterialTheme(
        colorScheme = if (dark) MaineDarkColors else MaineLightColors,
        typography = MaineTypography,
        shapes = MaineShapes,
        content = content,
    )
}

/** Accent-colored pill button: the single primary action per screen. */
@Composable
fun MainePrimaryButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
) {
    androidx.compose.material3.Button(
        onClick = onClick,
        enabled = enabled,
        shape = androidx.compose.foundation.shape.CircleShape,
        modifier = modifier.then(
            Modifier.height(56.dp).fillMaxWidth(),
        ),
    ) {
        Text(
            text,
            style = MaterialTheme.typography.titleMedium,
        )
    }
}
