package app.maine.android.ui

import android.view.KeyEvent
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.termux.terminal.KeyHandler

/** A key the row sends directly, with its Android key code. */
enum class ExtraKey(val label: String, val keyCode: Int) {
    Escape("ESC", KeyEvent.KEYCODE_ESCAPE),
    Tab("TAB", KeyEvent.KEYCODE_TAB),
    Left("←", KeyEvent.KEYCODE_DPAD_LEFT),
    Up("↑", KeyEvent.KEYCODE_DPAD_UP),
    Down("↓", KeyEvent.KEYCODE_DPAD_DOWN),
    Right("→", KeyEvent.KEYCODE_DPAD_RIGHT),
}

/**
 * Modifier state for the keys row, shared between the row and the terminal view.
 *
 * It has to live above the terminal because the modifier is resolved *at key
 * time*: `TerminalView` asks its client for Ctrl and Alt while handling input,
 * and only then turns them into a control code. That is what makes Ctrl+C work
 * when the `c` comes from the on-screen keyboard rather than from this row.
 *
 * Modifiers are one-shot: they clear as soon as a key has used them, so an
 * armed Ctrl cannot quietly control-code the rest of a typed command.
 */
class ExtraKeysState {
    var ctrl by mutableStateOf(false)
        private set

    var alt by mutableStateOf(false)
        private set

    fun toggleCtrl() {
        ctrl = !ctrl
    }

    fun toggleAlt() {
        alt = !alt
    }

    /** The Ctrl/Alt bits to merge into a special key's modifier mask. */
    fun modifierMask(): Int {
        var mask = 0
        if (ctrl) mask = mask or KeyHandler.KEYMOD_CTRL
        if (alt) mask = mask or KeyHandler.KEYMOD_ALT
        return mask
    }

    fun clear() {
        ctrl = false
        alt = false
    }
}

/**
 * The extra-keys row, sitting between the terminal and the soft keyboard.
 *
 * It exists because soft keyboards have no Escape, Tab or arrow keys: without
 * them there is no tab completion, no `Ctrl+C`, and no way out of `vim`'s
 * insert mode. Modifier keys toggle and stay lit while armed; everything else
 * sends once and clears them.
 */
@Composable
fun ExtraKeysRow(
    state: ExtraKeysState,
    onKey: (ExtraKey) -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier = modifier
            .fillMaxWidth()
            .background(MaineColors.Terminal),
    ) {
        HorizontalDivider(
            thickness = 1.dp,
            color = MaterialTheme.colorScheme.outlineVariant,
        )
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = TerminalInset, vertical = 6.dp),
            horizontalArrangement = Arrangement.spacedBy(6.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            ExtraKeysButton("ESC", active = false, modifier = Modifier.weight(1f), onClick = { onKey(ExtraKey.Escape) })
            ExtraKeysButton("CTRL", active = state.ctrl, modifier = Modifier.weight(1f), onClick = state::toggleCtrl)
            ExtraKeysButton("ALT", active = state.alt, modifier = Modifier.weight(1f), onClick = state::toggleAlt)
            ExtraKeysButton("TAB", active = false, modifier = Modifier.weight(1f), onClick = { onKey(ExtraKey.Tab) })
            ExtraKeysButton("←", active = false, modifier = Modifier.weight(1f), onClick = { onKey(ExtraKey.Left) })
            ExtraKeysButton("↑", active = false, modifier = Modifier.weight(1f), onClick = { onKey(ExtraKey.Up) })
            ExtraKeysButton("↓", active = false, modifier = Modifier.weight(1f), onClick = { onKey(ExtraKey.Down) })
            ExtraKeysButton("→", active = false, modifier = Modifier.weight(1f), onClick = { onKey(ExtraKey.Right) })
        }
    }
}

/**
 * One key. Armed modifiers fill with the brand yellow — the same signal the
 * rest of the app uses for "this is active" — and everything else stays in the
 * neutral surface tone so the row reads as chrome, not as content.
 */
@Composable
private fun ExtraKeysButton(
    label: String,
    active: Boolean,
    modifier: Modifier = Modifier,
    onClick: () -> Unit,
) {
    Box(
        modifier = modifier
            .height(38.dp)
            .clip(RoundedCornerShape(12.dp))
            .background(
                if (active) MaineColors.Accent else MaterialTheme.colorScheme.surfaceVariant,
            )
            .clickable(onClick = onClick),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            label,
            fontFamily = MaineMono,
            fontSize = 12.sp,
            fontWeight = FontWeight.SemiBold,
            color = if (active) MaineColors.AccentOn else MaterialTheme.colorScheme.onSurface,
        )
    }
}
