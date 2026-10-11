package app.maine.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.unit.dp

/** Quiet connection-status indicator shared by host and session rows. */
@Composable
fun StatusDot(connected: Boolean) {
    Box(
        modifier = Modifier
            .size(10.dp)
            .clip(CircleShape)
            .background(
                if (connected) MaterialTheme.colorScheme.primary
                else MaterialTheme.colorScheme.outline,
            ),
    )
}
