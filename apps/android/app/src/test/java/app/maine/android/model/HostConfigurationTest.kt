package app.maine.android.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

class HostConfigurationTest {

    @Test
    fun decodesValidFile() {
        val json = """
            {"schemaVersion": 1, "hosts": [
              {"id": "0f8fad5b-d9cb-469f-a165-70867728950e", "name": "Web",
               "hostname": "203.0.113.10", "port": 22, "username": ""}
            ]}
        """.trimIndent()
        val config = HostConfiguration.decode(json)
        assertEquals(1, config.hosts.size)
        assertEquals("Web", config.hosts[0].name)
        assertTrue(config.isSupportedByThisVersion)
    }

    @Test
    fun missingAndNullFieldsFallBackToDefaults() {
        val missing = """{"schemaVersion": 1, "hosts": [
          {"id": "0f8fad5b-d9cb-469f-a165-70867728950e", "name": "W", "hostname": "h"}
        ]}"""
        val nulled = """{"schemaVersion": 1, "hosts": [
          {"id": "0f8fad5b-d9cb-469f-a165-70867728950e", "name": "W", "hostname": "h",
           "port": null, "username": null}
        ]}"""
        for (json in listOf(missing, nulled)) {
            val host = HostConfiguration.decode(json).hosts.single()
            assertEquals(22, host.port)
            assertEquals("", host.username)
        }
    }

    @Test
    fun refusesNewerVersionWithoutTouchingIt() {
        val json = """{"schemaVersion": 2, "hosts": []}"""
        try {
            HostConfiguration.decode(json)
            fail("expected UnsupportedVersion")
        } catch (e: HostConfiguration.DecodeError.UnsupportedVersion) {
            assertTrue(e.message!!.contains("version 2"))
        }
    }

    @Test
    fun wrongTypeIsMalformed() {
        val json = """{"schemaVersion": 1, "hosts": [
          {"id": "0f8fad5b-d9cb-469f-a165-70867728950e", "name": "W",
           "hostname": "h", "port": "not-a-port", "username": ""}
        ]}"""
        try {
            HostConfiguration.decode(json)
            fail("expected Malformed")
        } catch (e: HostConfiguration.DecodeError.Malformed) {
            assertTrue(e.message!!.startsWith("Maine could not read this host list:"))
        }
    }

    @Test
    fun nonUuidIdIsMalformed() {
        val json = """{"schemaVersion": 1, "hosts": [
          {"id": "not-a-uuid", "name": "W", "hostname": "h", "port": 22, "username": ""}
        ]}"""
        try {
            HostConfiguration.decode(json)
            fail("expected Malformed")
        } catch (e: HostConfiguration.DecodeError.Malformed) {
            // Expected.
        }
    }

    @Test
    fun roundTrip() {
        val config = HostConfiguration(
            hosts = listOf(
                Host(name = "Web", hostname = "203.0.113.10"),
                Host(name = "DB", hostname = "db.internal", port = 2222, username = "deploy"),
            ),
        )
        val decoded = HostConfiguration.decode(config.encoded())
        assertEquals(config.hosts.map { it.name }, decoded.hosts.map { it.name })
        assertEquals(2222, decoded.hosts[1].port)
        assertEquals("deploy", decoded.hosts[1].username)
    }
}
