package app.maine.android.ui

import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.unit.dp
import app.maine.android.session.TerminalSession

/** Connection status. Only state colors — never decoration. */
enum class StatusKind { Live, Connecting, Failed, Idle }

fun TerminalSession.State.statusKind(): StatusKind = when (this) {
    is TerminalSession.State.Connected -> StatusKind.Live
    is TerminalSession.State.Connecting -> StatusKind.Connecting
    is TerminalSession.State.Failed -> StatusKind.Failed
    else -> StatusKind.Idle
}

/**
 * Quiet status indicator with a soft glow when live and a slow pulse while
 * connecting. Failed is a flat red dot; idle is the theme outline.
 */
@Composable
fun StatusDot(kind: StatusKind, modifier: Modifier = Modifier) {
    val dark = isSystemInDarkTheme()
    val solid = when (kind) {
        StatusKind.Live -> if (dark) MaineColors.ConnectedDark else MaineColors.ConnectedLight
        StatusKind.Connecting -> MaineColors.Connecting
        StatusKind.Failed -> if (dark) MaineColors.FailedDark else MaineColors.FailedLight
        StatusKind.Idle -> MaterialTheme.colorScheme.outline
    }
    if (kind == StatusKind.Connecting) {
        val pulse by rememberInfiniteTransition(label = "connecting")
            .animateFloat(
                initialValue = 0.35f,
                targetValue = 1f,
                animationSpec = infiniteRepeatable(
                    animation = tween(900),
                    repeatMode = RepeatMode.Reverse,
                ),
                label = "alpha",
            )
        Box(
            contentAlignment = Alignment.Center,
            modifier = modifier.size(22.dp),
        ) {
            Box(
                modifier = Modifier
                    .size(22.dp)
                    .clip(CircleShape)
                    .background(solid.copy(alpha = 0.18f)),
            )
            Box(
                modifier = Modifier
                    .size(10.dp)
                    .clip(CircleShape)
                    .background(solid)
                    .alpha(pulse),
            )
        }
        return
    }
    Box(
        contentAlignment = Alignment.Center,
        modifier = modifier.size(22.dp),
    ) {
        if (kind == StatusKind.Live) {
            Box(
                modifier = Modifier
                    .size(22.dp)
                    .clip(CircleShape)
                    .background(solid.copy(alpha = 0.16f)),
            )
        }
        Box(
            modifier = Modifier
                .size(10.dp)
                .clip(CircleShape)
                .background(solid),
        )
    }
}

/** Backwards-compatible overload for rows that only know live vs not. */
@Composable
fun StatusDot(connected: Boolean, modifier: Modifier = Modifier) {
    StatusDot(
        kind = if (connected) StatusKind.Live else StatusKind.Idle,
        modifier = modifier,
    )
}
