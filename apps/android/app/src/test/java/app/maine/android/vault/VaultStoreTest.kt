package app.maine.android.vault

import app.maine.android.model.Host
import app.maine.android.model.SavedSession
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class VaultStoreTest {

    @get:Rule
    val temp = TemporaryFolder()

    private fun store(name: String = "passwords.maine") =
        VaultStore(temp.root.resolve(name))

    private val password = "correct horse battery staple"

    @Test
    fun creatingRequiresTwelveCharacters() {
        val vault = store()
        try {
            vault.unlock("short")
            fail("expected WeakPassword")
        } catch (e: VaultStore.VaultError.WeakPassword) {
            // Expected.
        }
        assertFalse(vault.exists)
    }

    @Test
    fun roundTripAcrossInstances() {
        val file = temp.root.resolve("passwords.maine")
        val host = Host(name = "Web", hostname = "203.0.113.10", port = 2222, username = "deploy")
        VaultStore(file).apply {
            unlock(password)
            saveHost(host, "s3cret")
            saveSession(
                SavedSession(hostID = host.id, name = "Session 1", history = "hello"),
            )
            lock()
        }
        VaultStore(file).apply {
            unlock(password)
            val hosts = hosts()
            assertEquals(1, hosts.size)
            assertEquals("Web", hosts[0].name)
            assertEquals(2222, hosts[0].port)
            assertEquals("s3cret", passwordFor(host.id))
            val sessions = savedSessions()
            assertEquals(1, sessions.size)
            assertEquals("hello", sessions[0].history)
        }
    }

    @Test
    fun wrongPasswordFails() {
        val file = temp.root.resolve("passwords.maine")
        VaultStore(file).apply {
            unlock(password)
            saveHosts(listOf(Host(name = "W", hostname = "h")))
        }
        try {
            VaultStore(file).unlock("wrong password!!")
            fail("expected InvalidPassword")
        } catch (e: VaultStore.VaultError.InvalidPassword) {
            // Expected.
        }
    }

    @Test
    fun lockedReadsAreEmptyAndWritesRefuse() {
        val vault = store()
        assertEquals(emptyList<Host>(), vault.hosts())
        assertNull(vault.passwordFor(Host.newId()))
        assertEquals(emptyList<SavedSession>(), vault.savedSessions())
        try {
            vault.saveHosts(emptyList())
            fail("expected Locked")
        } catch (e: VaultStore.VaultError.Locked) {
            // Expected.
        }
    }

    @Test
    fun lockForgetsTheKey() {
        val file = temp.root.resolve("passwords.maine")
        val vault = VaultStore(file)
        vault.unlock(password)
        vault.saveHosts(listOf(Host(name = "W", hostname = "h")))
        vault.lock()
        try {
            vault.hosts()
            fail("expected Locked")
        } catch (e: VaultStore.VaultError.Locked) {
            // Expected.
        }
        // And it unlocks again with the same password.
        vault.unlock(password)
        assertEquals(1, vault.hosts().size)
    }

    @Test
    fun tamperedVaultFailsUnlock() {
        val file = temp.root.resolve("passwords.maine")
        VaultStore(file).apply { unlock(password) }
        // Flip one base64 character inside the verification blob: the JSON
        // stays well-formed, so only GCM authentication can catch it.
        val text = file.readText()
        val marker = "\"verification\":\""
        val at = text.indexOf(marker)
        assertTrue("vault JSON must carry verification", at >= 0)
        val pos = at + marker.length
        val flipped = text.toCharArray().also { it[pos] = if (it[pos] == 'A') 'B' else 'A' }
        file.writeText(String(flipped))
        try {
            VaultStore(file).unlock(password)
            fail("expected InvalidPassword")
        } catch (e: VaultStore.VaultError.InvalidPassword) {
            // Expected.
        }
    }

    @Test
    fun everyWriteUsesFreshRandomness() {
        val file = temp.root.resolve("passwords.maine")
        val vault = VaultStore(file)
        vault.unlock(password)
        vault.saveHosts(emptyList())
        val first = file.readText()
        vault.saveHosts(emptyList())
        assertNotEquals(first, file.readText())
    }

    @Test
    fun version1IsRejectedWithoutChanges() {
        val file = temp.root.resolve("passwords.maine")
        val salt = java.util.Base64.getEncoder().encodeToString(ByteArray(32))
        file.writeText(
            """{"id": "0F8FAD5B-D9CB-469F-A165-70867728950E", "iterations": 600000,
               "passwords": {}, "salt": "$salt", "verification": "eA==", "version": 1}""",
        )
        val before = file.readText()
        try {
            VaultStore(file).unlock(password)
            fail("expected InvalidFile")
        } catch (e: VaultStore.VaultError.InvalidFile) {
            // Expected.
        }
        assertEquals(before, file.readText())
    }

    @Test
    fun saveHostRequiresValidHostAndPassword() {
        val vault = store()
        vault.unlock(password)
        try {
            vault.saveHost(Host(name = "", hostname = "h"), "x")
            fail("expected InvalidFile")
        } catch (e: VaultStore.VaultError.InvalidFile) {
            // Expected.
        }
        try {
            vault.saveHost(Host(name = "W", hostname = "h"), "")
            fail("expected InvalidFile")
        } catch (e: VaultStore.VaultError.InvalidFile) {
            // Expected.
        }
    }

    @Test
    fun deleteHostRemovesCredentialsAndSnapshots() {
        val vault = store()
        vault.unlock(password)
        val host = Host(name = "W", hostname = "h")
        vault.saveHost(host, "pw")
        vault.saveSession(SavedSession(hostID = host.id, name = "Session 1"))
        vault.deleteHost(host.id)
        assertTrue(vault.hosts().isEmpty())
        assertNull(vault.passwordFor(host.id))
        assertTrue(vault.savedSessions().isEmpty())
    }

    @Test
    fun emptyPasswordDeletesTheCredential() {
        val vault = store()
        vault.unlock(password)
        val host = Host(name = "W", hostname = "h")
        vault.saveHost(host, "pw")
        vault.savePassword("", host.id)
        assertNull(vault.passwordFor(host.id))
    }

    @Test
    fun idsAreCanonicalizedToUppercase() {
        val vault = store()
        vault.unlock(password)
        val lower = Host(name = "W", hostname = "h", id = "0f8fad5b-d9cb-469f-a165-70867728950e")
        vault.saveHost(lower, "pw")
        val stored = vault.hosts().single()
        assertEquals(lower.id.uppercase(), stored.id)
        // Lookup works with either casing, like the macOS client.
        assertEquals("pw", vault.passwordFor(lower.id.uppercase()))
        assertEquals("pw", vault.passwordFor(lower.id.lowercase()))
    }

    @Test
    fun sealOpenRoundTripAndAadMismatch() {
        val vault = store()
        val key = vault.derive(password, ByteArray(32) { it.toByte() }, 600_000)
        // AAD layout must stay identical to the macOS context string.
        val aad = "maine-vault:2:0F8FAD5B-D9CB-469F-A165-70867728950E:" +
            "${vault.encodeBase64(ByteArray(32))}:600000"
        val sealed = vault.seal("hello".toByteArray(), key, aad.toByteArray())
        // CryptoKit combined layout: 12-byte nonce || ciphertext || 16-byte tag.
        assertEquals(12 + 5 + 16, sealed.size)
        assertEquals("hello", vault.open(sealed, key, aad.toByteArray()).toString(Charsets.UTF_8))
        try {
            vault.open(sealed, key, "maine-vault:2:WRONG:AAAA:600000".toByteArray())
            fail("expected authentication failure")
        } catch (e: Exception) {
            // Expected: wrong AAD must never decrypt.
        }
    }
}
