package app.maine.android.ssh

import app.maine.android.model.Host
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class SshTargetTest {

    @Test
    fun port22MeansSayNothing() {
        val target = SshTarget.make(Host(name = "W", hostname = "example.com"))
        assertEquals(listOf("example.com"), target.arguments)
        assertEquals("ssh example.com", target.commandLine)
    }

    @Test
    fun customPortAndUserArePassed() {
        val target = SshTarget.make(
            Host(name = "W", hostname = "example.com", port = 2222, username = "deploy"),
        )
        assertEquals(listOf("-p", "2222", "-l", "deploy", "example.com"), target.arguments)
        assertEquals("ssh -p 2222 -l deploy example.com", target.commandLine)
        assertEquals("example.com", target.hostname)
        assertEquals(2222, target.port)
        assertEquals("deploy", target.username)
    }

    @Test
    fun configEntrySuppliesDisplayResolution() {
        val config = SshConfigFile(
            listOf(SshConfigEntry("web", hostname = "10.0.0.5", user = "deploy", port = 2222)),
        )
        val target = SshTarget.make(Host(name = "W", hostname = "web"))
        // Display args stay minimal; resolution explains what happens.
        assertEquals(listOf("web"), target.arguments)
        val resolved = target.resolved(config)
        assertEquals("10.0.0.5", resolved.hostname)
        assertEquals("deploy", resolved.username)
        assertEquals(2222, resolved.port)
        assertEquals("web", resolved.configPattern)
        assertEquals("deploy@10.0.0.5:2222", resolved.target)
    }

    @Test
    fun explicitFlagsWinOverConfig() {
        val config = SshConfigFile(
            listOf(SshConfigEntry("web", hostname = "10.0.0.5", user = "deploy", port = 2222)),
        )
        val target = SshTarget.make(
            Host(name = "W", hostname = "web", port = 2200, username = "root"),
            config,
        )
        val resolved = target.resolved(config)
        assertEquals("root", resolved.username)
        assertEquals(2200, resolved.port)
        assertEquals("10.0.0.5", resolved.hostname)
    }

    @Test
    fun unknownHostResolvesPlainly() {
        val target = SshTarget.make(Host(name = "W", hostname = "example.com"))
        val resolved = target.resolved(SshConfigFile())
        assertEquals("example.com", resolved.hostname)
        assertEquals(22, resolved.port)
        assertTrue(resolved.username.isNotBlank())
        assertNull(resolved.configPattern)
    }
}
