package app.maine.android.ui

import androidx.compose.ui.graphics.toArgb
import com.termux.terminal.TerminalColors
import com.termux.terminal.TextStyle

/** Terminal foreground, matching [MaineColors.TerminalText]. */
private val TerminalForeground = MaineColors.TerminalText.toArgb()

/** Terminal background: pure black, matching [MaineColors.Terminal]. */
private val TerminalBackground = MaineColors.Terminal.toArgb()

/** Cursor: the brand yellow, matching [MaineColors.Accent]. */
private val TerminalCursor = MaineColors.Accent.toArgb()

/**
 * The terminal's xterm palette, in Maine's colors.
 *
 * Terminals stay dark on both app themes — that is the convention everywhere
 * (Termux, iTerm, macOS Terminal), and it is what the macOS client does. The
 * sixteen ANSI colors are desaturated so a full-color `ls` or `git diff` does
 * not fight the monochrome UI, with the brand yellow kept as the one loud hue
 * and used for the cursor.
 *
 * Applied once at launch, before any emulator exists. [TerminalColors] reads
 * from a single shared scheme, so this mutates that scheme in place.
 */
fun applyMaineTerminalPalette() {
    val colors = TerminalColors.COLOR_SCHEME.mDefaultColors

    // Indices 0-7 dim, 8-15 bright, per the xterm convention.
    val ansi = intArrayOf(
        0xFF2A2A31.toInt(), // black
        0xFFFF6B6B.toInt(), // red
        0xFF7BD88F.toInt(), // green
        0xFFFCEE0C.toInt(), // yellow (brand)
        0xFF6AA6FF.toInt(), // blue
        0xFFC792EA.toInt(), // magenta
        0xFF56D4DD.toInt(), // cyan
        0xFFC7C7CC.toInt(), // white
        0xFF5A5A66.toInt(), // bright black
        0xFFFF8A8A.toInt(), // bright red
        0xFF9DEBAE.toInt(), // bright green
        0xFFFFF06E.toInt(), // bright yellow
        0xFF93BEFF.toInt(), // bright blue
        0xFFDDB4F5.toInt(), // bright magenta
        0xFF86E7EE.toInt(), // bright cyan
        0xFFFFFFFF.toInt(), // bright white
    )
    ansi.copyInto(colors, destinationOffset = 0)

    colors[TextStyle.COLOR_INDEX_FOREGROUND] = TerminalForeground
    colors[TextStyle.COLOR_INDEX_BACKGROUND] = TerminalBackground
    colors[TextStyle.COLOR_INDEX_CURSOR] = TerminalCursor
}
