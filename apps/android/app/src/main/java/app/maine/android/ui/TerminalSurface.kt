package app.maine.android.ui

import android.content.Context
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.inputmethod.InputMethodManager
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import app.maine.android.session.TerminalSession
import com.termux.terminal.TerminalSession as TermuxSession
import com.termux.view.TerminalView
import com.termux.view.TerminalViewClient

private const val DEFAULT_TEXT_SIZE_SP = 14f
private const val MIN_TEXT_SIZE_SP = 8f
private const val MAX_TEXT_SIZE_SP = 24f

/**
 * The terminal itself, as the input surface: a real VT/xterm emulator view.
 *
 * Tapping focuses it and raises the keyboard, typing goes straight to the
 * remote shell, and the screen scrolls through the emulator's own scrollback —
 * there is no separate "send input" field, like Termux.
 *
 * The emulator lives in the [TerminalSession], not the view, so it survives
 * navigating away and back: only the view is rebuilt, and it re-attaches to the
 * same terminal state.
 */
@Composable
fun TerminalSurface(
    session: TerminalSession,
    extraKeys: ExtraKeysState,
    modifier: Modifier = Modifier,
) {
    val density = LocalDensity.current
    AndroidView(
        modifier = modifier,
        factory = { viewContext ->
            val terminal = session.emulator(viewContext)
            val view = TerminalView(viewContext, null).apply {
                setBackgroundColor(0xFF000000.toInt())
                isFocusable = true
                isFocusableInTouchMode = true
            }
            val client = MaineTerminalViewClient(viewContext, view, density, extraKeys)
            view.setTerminalViewClient(client)
            client.applyTextSize(DEFAULT_TEXT_SIZE_SP)
            terminal.onScreenUpdated = view::onScreenUpdated
            view.attachSession(terminal.session)
            view
        },
        onRelease = {
            // The emulator outlives the view; drop the hook so a destroyed view
            // cannot be kept alive by the session.
            session.emulator(it.context).onScreenUpdated = null
        },
    )
}

/**
 * Terminal UI policy for Maine.
 *
 * Hardware keys are passed through to the emulator untouched, and the system
 * back gesture keeps navigating the app rather than being mapped to ESC (the
 * session screen's header has an explicit back button, and escape is available
 * from the keyboard).
 *
 * All `log*` methods are intentionally empty: this interface is how the
 * vendored view reports key and IME activity, and the diagnostics invariant
 * forbids writing terminal contents — including keystrokes — anywhere.
 * [TerminalView.setIsTerminalViewKeyLoggingEnabled] stays off for the same
 * reason.
 */
private class MaineTerminalViewClient(
    private val appContext: Context,
    private val view: TerminalView,
    private val density: Density,
    private val extraKeys: ExtraKeysState,
) : TerminalViewClient {

    private var textSizeSp = DEFAULT_TEXT_SIZE_SP

    /** Applies a monospace text size, in sp, and reflows the terminal. */
    fun applyTextSize(sp: Float) {
        textSizeSp = sp
        view.setTextSize(with(density) { sp.sp.toPx() }.toInt())
    }

    /**
     * Pinch to zoom text. The recognizer passes the factor accumulated since
     * the gesture began and expects 1f back to reset the accumulator.
     */
    override fun onScale(scale: Float): Float {
        when {
            scale > 1.1f -> applyTextSize((textSizeSp + 1f).coerceAtMost(MAX_TEXT_SIZE_SP))
            scale < 0.9f -> applyTextSize((textSizeSp - 1f).coerceAtLeast(MIN_TEXT_SIZE_SP))
        }
        return 1f
    }

    override fun onSingleTapUp(e: MotionEvent) {
        val imm = appContext.getSystemService(InputMethodManager::class.java)
        imm?.showSoftInput(view, InputMethodManager.SHOW_IMPLICIT)
    }

    override fun shouldBackButtonBeMappedToEscape(): Boolean = false

    /** Avoids IME autocorrect and suggestions hijacking terminal input. */
    override fun shouldEnforceCharBasedInput(): Boolean = true

    override fun shouldUseCtrlSpaceWorkaround(): Boolean = false

    override fun isTerminalViewSelected(): Boolean = true

    override fun copyModeChanged(copyMode: Boolean) = Unit

    override fun onKeyDown(keyCode: Int, e: KeyEvent, session: TermuxSession): Boolean = false

    override fun onKeyUp(keyCode: Int, e: KeyEvent): Boolean = false

    override fun onLongPress(event: MotionEvent): Boolean = false

    // The keys row drives these; `TerminalView` reads them while turning a key
    // into a control code, which is how Ctrl+C works with a soft-keyboard `c`.
    override fun readControlKey(): Boolean = extraKeys.ctrl

    override fun readAltKey(): Boolean = extraKeys.alt

    override fun readShiftKey(): Boolean = false

    override fun readFnKey(): Boolean = false

    override fun onCodePoint(codePoint: Int, ctrlDown: Boolean, session: TermuxSession): Boolean {
        // The view has already resolved Ctrl/Alt for this code point, so this is
        // where a one-shot modifier is spent.
        extraKeys.clear()
        return false
    }

    override fun onEmulatorSet() = Unit

    override fun logError(tag: String, message: String) = Unit
    override fun logWarn(tag: String, message: String) = Unit
    override fun logInfo(tag: String, message: String) = Unit
    override fun logDebug(tag: String, message: String) = Unit
    override fun logVerbose(tag: String, message: String) = Unit
    override fun logStackTraceWithMessage(tag: String, message: String, e: Exception) = Unit
    override fun logStackTrace(tag: String, e: Exception) = Unit
}
