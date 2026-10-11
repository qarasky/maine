package app.maine.android.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class HostTest {

    private fun valid() = Host(name = "Web", hostname = "203.0.113.10")

    @Test
    fun defaultsMatchSchema() {
        val host = Host(name = "Web", hostname = "example.com")
        assertEquals(22, host.port)
        assertEquals("", host.username)
        assertTrue(host.isValid)
    }

    @Test
    fun displayNameFallsBackToHostname() {
        assertEquals("example.com", Host(name = "  ", hostname = "example.com").displayName)
        assertEquals("Web", Host(name = "  Web ", hostname = "example.com").displayName)
    }

    @Test
    fun targetAndSubtitle() {
        assertEquals("203.0.113.10", valid().target)
        assertEquals("203.0.113.10", valid().subtitle)
        assertEquals("deploy@203.0.113.10:2222", valid().copy(username = "deploy", port = 2222).subtitle)
        assertNull(valid().portDescription)
        assertEquals(":2222", valid().copy(port = 2222).portDescription)
    }

    @Test
    fun nameIsRequired() {
        assertEquals(listOf("A name is required."), valid().copy(name = "   ").issues)
    }

    @Test
    fun hostnameRulesMirrorMacOS() {
        assertTrue(valid().copy(hostname = "").issues.contains("A hostname or IP address is required."))
        assertTrue(
            valid().copy(hostname = "has space").issues.contains("The hostname cannot contain spaces."),
        )
        assertTrue(
            valid().copy(hostname = "-x").issues.first()
                .contains("cannot start with"),
        )
        assertTrue(
            valid().copy(hostname = "a".repeat(254)).issues.contains(
                "The hostname cannot be longer than 253 characters.",
            ),
        )
        assertTrue(valid().copy(hostname = "a".repeat(253)).isValid)
    }

    @Test
    fun portRange() {
        assertFalse(valid().copy(port = 0).isValid)
        assertFalse(valid().copy(port = 65536).isValid)
        assertTrue(valid().copy(port = 1).isValid)
        assertTrue(valid().copy(port = 65535).isValid)
        assertTrue(
            valid().copy(port = 0).issues.contains("The port must be between 1 and 65535."),
        )
    }

    @Test
    fun usernameRules() {
        assertTrue(
            valid().copy(username = "has space").issues.contains("The username cannot contain spaces."),
        )
        assertTrue(
            valid().copy(username = "u".repeat(129)).issues.contains(
                "The username cannot be longer than 128 characters.",
            ),
        )
    }

    @Test
    fun idsCompareCaseInsensitively() {
        val lower = "0f8fad5b-d9cb-469f-a165-70867728950e"
        val upper = lower.uppercase()
        assertTrue(Host.idsEqual(lower, upper))
        assertEquals(upper, Host.normalizeId(lower))
        assertFalse(Host.idsEqual(lower, "11111111-1111-1111-1111-111111111111"))
    }
}
