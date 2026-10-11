package app.maine.android.ssh

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class SshConfigFileTest {

    @get:Rule
    val temp = TemporaryFolder()

    @Test
    fun parsesBlocksWithFirstValueWinning() {
        val file = temp.newFile("config").apply {
            writeText(
                """
                Host web
                  HostName 10.0.0.5
                  User deploy
                  Port 2222

                Host web
                  User ignored
                  Port 2200
                """.trimIndent(),
            )
        }
        val config = SshConfigFile.load(file)
        val entry = config.entryFor("web")!!
        assertEquals("10.0.0.5", entry.hostname)
        assertEquals("deploy", entry.user)
        assertEquals(2222, entry.port)
    }

    @Test
    fun handlesCommentsEqualsFormAndQuotes() {
        val file = temp.newFile("config").apply {
            writeText(
                """
                # whole line comment
                Host quoted # trailing comment
                  HostName="10.0.0.9" # after value
                  User='deploy'
                  Port=2200
                Host hash#inside user#name
                """.trimIndent(),
            )
        }
        val config = SshConfigFile.load(file)
        val entry = config.entryFor("quoted")!!
        assertEquals("10.0.0.9", entry.hostname)
        assertEquals("deploy", entry.user)
        assertEquals(2200, entry.port)
        // '#' without preceding whitespace is part of the value.
        assertTrue(config.entries.any { it.pattern == "hash#inside" })
        assertTrue(config.entries.any { it.pattern == "user#name" })
    }

    @Test
    fun followsIncludesWithCycleProtection() {
        val dir = temp.newFolder("ssh")
        val extra = java.io.File(dir, "extra").apply {
            writeText("Host extra\n  HostName 10.0.0.7\n")
        }
        val main = java.io.File(dir, "config").apply {
            writeText("Include extra\nHost main\n  HostName 10.0.0.1\nInclude extra\n")
        }
        // Self-include must terminate.
        extra.appendText("Include extra\n")
        val config = SshConfigFile.load(main)
        assertEquals("10.0.0.1", config.entryFor("main")?.hostname)
        assertEquals("10.0.0.7", config.entryFor("extra")?.hostname)
    }

    @Test
    fun missingFileIsEmptyNotAnError() {
        assertTrue(SshConfigFile.load(java.io.File(temp.root, "nope")).entries.isEmpty())
    }

    @Test
    fun entryLookupIsCaseInsensitive() {
        val config = SshConfigFile(listOf(SshConfigEntry("Web", hostname = "x")))
        assertEquals("x", config.entryFor("web")?.hostname)
        assertEquals("x", config.entryFor("WEB")?.hostname)
    }

    @Test
    fun wildcardsAreNotConnectable() {
        assertFalse(SshConfigEntry.isConnectable("*.example.com"))
        assertFalse(SshConfigEntry.isConnectable("web?"))
        assertFalse(SshConfigEntry.isConnectable(""))
        assertFalse(SshConfigEntry.isConnectable("has space"))
        assertTrue(SshConfigEntry.isConnectable("web"))
        assertNull(SshConfigEntry("*.example.com").toHostOrNull())
        val host = SshConfigEntry("web", user = "deploy", port = 2222).toHostOrNull()!!
        assertEquals("web", host.name)
        assertEquals("deploy", host.username)
        assertEquals(2222, host.port)
    }
}
