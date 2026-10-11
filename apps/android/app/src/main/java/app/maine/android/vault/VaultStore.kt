package app.maine.android.vault

import app.maine.android.model.Host
import app.maine.android.model.HostConfiguration
import app.maine.android.model.SavedSession
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import java.io.File
import java.security.SecureRandom
import java.util.UUID
import javax.crypto.Cipher
import javax.crypto.SecretKey
import javax.crypto.SecretKeyFactory
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.PBEKeySpec
import javax.crypto.spec.SecretKeySpec

/**
 * Portable AES-256-GCM hosts and credentials protected by a master password.
 *
 * Byte-for-byte compatible with the macOS `PasswordStore` (vault version 2):
 * the same JSON envelope, the same PBKDF2-SHA256 parameters (600,000
 * iterations, 32-byte salt), the same AES-GCM nonce/ciphertext/tag layout,
 * and the same additional authenticated data. A `passwords.maine` file
 * written by either client unlocks on the other with the same master
 * password, in either direction.
 *
 * Differences from macOS, all deliberate:
 * * The derived key is held in memory only and cleared on [lock]. There is
 *   no "remember me": every launch starts locked.
 * * Version-1 vaults are rejected. They predate the Android client; unlock
 *   once on macOS to migrate, then copy the file over.
 * * The file lives in the app's private internal storage
 *   (`filesDir/passwords.maine`), which is already inaccessible to other
 *   apps; `0600` permissions are still applied best-effort.
 *
 * UUID casing: the macOS client writes IDs in uppercase `UUID().uuidString`
 * form and looks passwords up with that exact casing. IDs are normalized to
 * uppercase on read and written uppercase, so lookups succeed on both
 * clients. Comparisons elsewhere use [Host.idsEqual] (case-insensitive).
 */
class VaultStore(val file: File) {

    companion object {
        const val FILE_NAME = "passwords.maine"
        const val ITERATIONS = 600_000
        private const val GCM_TAG_BITS = 128
        private const val NONCE_BYTES = 12
        private const val SALT_BYTES = 32
        private val MARKER = "Maine password vault v1".toByteArray(Charsets.UTF_8)

        private val vaultJson = Json {
            ignoreUnknownKeys = true
            explicitNulls = false
        }
        private val contentsJson = Json {
            ignoreUnknownKeys = true
            explicitNulls = false
        }
    }

    sealed class VaultError(message: String) : Exception(message) {
        data object Locked : VaultError(
            "The password vault is locked. Unlock it with the master password.",
        )
        data object InvalidPassword : VaultError(
            "The master password is incorrect, or the vault has been damaged.",
        )
        data object WeakPassword : VaultError(
            "Use a master password of at least 12 characters.",
        )
        class InvalidFile(
            detail: String = "This password vault is damaged or uses an unsupported format. " +
                "It has not been changed.",
            cause: Throwable? = null,
        ) : VaultError(detail) {
            init {
                if (cause != null) initCause(cause)
            }
        }
    }

    /** On-disk envelope. Field defaults mirror the macOS struct exactly:
     * `version` falls back to 1, `passwords` is the required version-1
     * credential map (must be empty in version 2), and `payload` holds the
     * encrypted contents. */
    @Serializable
    internal data class VaultDto(
        @SerialName("version") val version: Int = 1,
        @SerialName("id") val id: String,
        @SerialName("salt")
        @Serializable(with = Base64Bytes::class)
        val salt: ByteArray,
        @SerialName("iterations") val iterations: Int,
        @SerialName("verification")
        @Serializable(with = Base64Bytes::class)
        val verification: ByteArray,
        @SerialName("passwords")
        val passwords: Map<String, @Serializable(with = Base64Bytes::class) ByteArray>,
        @SerialName("payload")
        @Serializable(with = Base64Bytes::class)
        val payload: ByteArray? = null,
    ) {
        // ByteArray needs value equality for tests.
        override fun equals(other: Any?): Boolean {
            if (this === other) return true
            if (other !is VaultDto) return false
            return version == other.version && id == other.id &&
                salt.contentEquals(other.salt) && iterations == other.iterations &&
                verification.contentEquals(other.verification) &&
                passwords.keys == other.passwords.keys &&
                passwords.all { (k, v) -> other.passwords[k]?.contentEquals(v) == true } &&
                ((payload == null && other.payload == null) ||
                    (payload != null && other.payload != null && payload.contentEquals(other.payload)))
        }

        override fun hashCode(): Int {
            var result = version
            result = 31 * result + id.hashCode()
            result = 31 * result + salt.contentHashCode()
            result = 31 * result + iterations
            result = 31 * result + verification.contentHashCode()
            result = 31 * result + passwords.hashCode()
            result = 31 * result + (payload?.contentHashCode() ?: 0)
            return result
        }
    }

    @Serializable
    internal data class ContentsDto(
        @SerialName("hosts") val hosts: List<Host> = emptyList(),
        @SerialName("passwords") val passwords: Map<String, String> = emptyMap(),
        @SerialName("sessions") val sessions: List<SavedSession>? = null,
    )

    @Volatile
    private var unlockedKey: SecretKey? = null

    val exists: Boolean get() = file.exists()
    val isUnlocked: Boolean get() = unlockedKey != null

    /** Creates a vault on first use, or verifies and unlocks an existing copy. */
    @Throws(VaultError::class)
    fun unlock(masterPassword: String) {
        val vault = read()
        if (vault == null) {
            if (masterPassword.length < 12) throw VaultError.WeakPassword
            val salt = randomBytes(SALT_BYTES)
            val fresh = VaultDto(
                version = 2,
                id = UUID.randomUUID().toString().uppercase(),
                salt = salt,
                iterations = ITERATIONS,
                verification = ByteArray(0),
                passwords = emptyMap(),
            )
            val key = derive(masterPassword, fresh.salt, fresh.iterations)
            val sealed = fresh.copy(
                verification = seal(MARKER, key, aad(fresh)),
                payload = seal(encodeContents(ContentsDto()), key, aad(fresh)),
            )
            write(sealed)
            unlockedKey = key
            return
        }
        if (vault.version == 1) {
            throw VaultError.InvalidFile(
                "This vault was created by an older version of Maine. " +
                    "Unlock it once on macOS to migrate it, then copy it here. " +
                    "It has not been changed.",
            )
        }
        val key = derive(masterPassword, vault.salt, vault.iterations)
        try {
            if (!open(vault.verification, key, aad(vault)).contentEquals(MARKER)) {
                throw VaultError.InvalidPassword
            }
            decodeContents(vault, key)
        } catch (e: VaultError) {
            throw VaultError.InvalidPassword
        }
        unlockedKey = key
    }

    @Throws(VaultError::class)
    fun lock() {
        // Validates the file exactly like the macOS client, then forgets the key.
        if (exists) read()
        unlockedKey = null
    }

    @Throws(VaultError::class)
    fun passwordFor(hostId: String): String? {
        val vault = read() ?: return null
        return decodeContents(vault, unlockedKey(vault)).passwords[canonicalId(hostId)]
    }

    @Throws(VaultError::class)
    fun savePassword(password: String, hostId: String) {
        if (password.isEmpty()) {
            deletePassword(hostId)
            return
        }
        updateContents { it.copy(passwords = it.passwords + (canonicalId(hostId) to password)) }
    }

    @Throws(VaultError::class)
    fun deletePassword(hostId: String) {
        if (!exists) return
        updateContents { it.copy(passwords = it.passwords - canonicalId(hostId)) }
    }

    @Throws(VaultError::class)
    fun hosts(): List<Host> {
        val vault = read() ?: return emptyList()
        return decodeContents(vault, unlockedKey(vault)).hosts
    }

    @Throws(VaultError::class)
    fun saveHosts(hosts: List<Host>) {
        hosts.forEach { requireStorable(it) }
        updateContents { it.copy(hosts = hosts.map { h -> h.copy(id = canonicalId(h.id)) }) }
    }

    /** Commits metadata and its credential together in one atomic replacement. */
    @Throws(VaultError::class)
    fun saveHost(host: Host, password: String) {
        if (!host.isValid || password.isEmpty()) throw VaultError.InvalidFile()
        requireStorable(host)
        val id = canonicalId(host.id)
        updateContents { contents ->
            val updated = if (contents.hosts.any { Host.idsEqual(it.id, id) }) {
                contents.hosts.map { if (Host.idsEqual(it.id, id)) host.copy(id = id) else it }
            } else {
                contents.hosts + host.copy(id = id)
            }
            contents.copy(hosts = updated, passwords = contents.passwords + (id to password))
        }
    }

    @Throws(VaultError::class)
    fun deleteHost(hostId: String) {
        val key = canonicalIdOrNull(hostId)
        updateContents { contents ->
            contents.copy(
                hosts = contents.hosts.filterNot { Host.idsEqual(it.id, hostId) },
                passwords = if (key == null) contents.passwords else contents.passwords - key,
                sessions = contents.sessions?.filterNot { Host.idsEqual(it.hostID, hostId) },
            )
        }
    }

    @Throws(VaultError::class)
    fun savedSessions(): List<SavedSession> {
        val vault = read() ?: return emptyList()
        return decodeContents(vault, unlockedKey(vault)).sessions ?: emptyList()
    }

    @Throws(VaultError::class)
    fun saveSession(session: SavedSession) {
        if (session.name.trim().isEmpty()) throw VaultError.InvalidFile()
        updateContents { contents ->
            val saved = (contents.sessions ?: emptyList()).toMutableList()
            val index = saved.indexOfFirst { Host.idsEqual(it.id, session.id) }
            if (index >= 0) saved[index] = session else saved += session
            contents.copy(sessions = saved)
        }
    }

    @Throws(VaultError::class)
    fun saveSessions(sessions: List<SavedSession>) {
        updateContents { contents ->
            val saved = (contents.sessions ?: emptyList()).toMutableList()
            for (session in sessions) {
                val index = saved.indexOfFirst { Host.idsEqual(it.id, session.id) }
                if (index >= 0) saved[index] = session else saved += session
            }
            contents.copy(sessions = saved)
        }
    }

    @Throws(VaultError::class)
    fun deleteSavedSession(id: String) {
        updateContents { contents ->
            contents.copy(sessions = contents.sessions?.filterNot { Host.idsEqual(it.id, id) })
        }
    }

    // MARK: - Internals

    private fun updateContents(update: (ContentsDto) -> ContentsDto) {
        val vault = read() ?: throw VaultError.Locked
        val key = unlockedKey(vault)
        val contents = decodeContents(vault, key)
        val updated = update(contents)
        // Re-validate: never write hosts that would not load back.
        HostConfiguration(hosts = updated.hosts).let {
            if (!it.isSupportedByThisVersion) throw VaultError.InvalidFile()
        }
        write(vault.copy(payload = seal(encodeContents(updated), key, aad(vault))))
    }

    private fun decodeContents(vault: VaultDto, key: SecretKey): ContentsDto {
        val payload = vault.payload
        if (vault.version != 2 || payload == null) throw VaultError.InvalidFile()
        val plain = try {
            open(payload, key, aad(vault))
        } catch (e: VaultError) {
            throw e
        } catch (e: Exception) {
            throw VaultError.InvalidFile(cause = e)
        }
        val contents = try {
            contentsJson.decodeFromString(
                ContentsDto.serializer(),
                plain.toString(Charsets.UTF_8),
            )
        } catch (e: Exception) {
            throw VaultError.InvalidFile(cause = e)
        }
        // Hosts must round-trip through the shared configuration format.
        try {
            HostConfiguration.decode(
                HostConfiguration(hosts = contents.hosts).encoded(),
            )
        } catch (e: HostConfiguration.DecodeError) {
            throw VaultError.InvalidFile()
        }
        if (!contents.passwords.keys.all { isUuid(it) }) throw VaultError.InvalidFile()
        if (contents.sessions != null &&
            !contents.sessions.all { isUuid(it.id) && isUuid(it.hostID) }
        ) {
            throw VaultError.InvalidFile()
        }
        return ContentsDto(
            hosts = contents.hosts.map { it.copy(id = canonicalId(it.id)) },
            passwords = contents.passwords.mapKeys { canonicalId(it.key) },
            sessions = contents.sessions?.map {
                it.copy(id = canonicalId(it.id), hostID = canonicalId(it.hostID))
            },
        )
    }

    private fun unlockedKey(vault: VaultDto): SecretKey {
        val key = unlockedKey ?: throw VaultError.Locked
        if (!open(vault.verification, key, aad(vault)).contentEquals(MARKER)) {
            throw VaultError.InvalidFile()
        }
        return key
    }

    private fun read(): VaultDto? {
        if (!exists) return null
        try {
            val vault = vaultJson.decodeFromString(
                VaultDto.serializer(),
                file.readText(Charsets.UTF_8),
            )
            uuidOrThrow(vault.id)
            if (vault.version !in 1..2) throw VaultError.InvalidFile()
            if (vault.salt.size != SALT_BYTES) throw VaultError.InvalidFile()
            if (vault.iterations != ITERATIONS) throw VaultError.InvalidFile()
            if (vault.version == 1) return vault
            if (vault.payload == null || vault.passwords.isNotEmpty()) {
                throw VaultError.InvalidFile()
            }
            if (!vault.passwords.keys.all { isUuid(it) }) throw VaultError.InvalidFile()
            return vault
        } catch (e: VaultError) {
            throw e
        } catch (e: Exception) {
            throw VaultError.InvalidFile(cause = e)
        }
    }

    private fun write(vault: VaultDto) {
        file.parentFile?.mkdirs()
        val temp = File(file.parentFile, "${file.name}.tmp")
        temp.writeText(vaultJson.encodeToString(VaultDto.serializer(), vault), Charsets.UTF_8)
        if (!temp.renameTo(file)) {
            // renameTo fails across volumes; fall back to a copy.
            temp.copyTo(file, overwrite = true)
            temp.delete()
        }
        // Internal storage is already private; still apply 0600 best-effort.
        runCatching {
            file.setReadable(false, false)
            file.setWritable(false, false)
            file.setExecutable(false, false)
            file.setReadable(true, true)
            file.setWritable(true, true)
        }
    }

    private fun encodeContents(contents: ContentsDto): ByteArray =
        contentsJson.encodeToString(ContentsDto.serializer(), contents)
            .toByteArray(Charsets.UTF_8)

    /** Additional authenticated data. Must stay identical to the macOS
     * `"maine-vault:\(version):\(id.uuidString):\(salt):\(iterations)"`. */
    internal fun aad(vault: VaultDto): ByteArray =
        ("maine-vault:${vault.version}:${uuidOrThrow(vault.id)}:" +
            "${encodeBase64(vault.salt)}:${vault.iterations}").toByteArray(Charsets.UTF_8)

    internal fun derive(password: String, salt: ByteArray, iterations: Int): SecretKey {
        val factory = SecretKeyFactory.getInstance("PBKDF2WithHmacSHA256")
        val spec = PBEKeySpec(password.toCharArray(), salt, iterations, 256)
        return SecretKeySpec(factory.generateSecret(spec).encoded, "AES").also {
            spec.clearPassword()
        }
    }

    /** Sealed layout matches CryptoKit's `combined`: nonce || ciphertext || tag. */
    internal fun seal(data: ByteArray, key: SecretKey, context: ByteArray): ByteArray {
        val nonce = randomBytes(NONCE_BYTES)
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, key, GCMParameterSpec(GCM_TAG_BITS, nonce))
        cipher.updateAAD(context)
        return nonce + cipher.doFinal(data)
    }

    internal fun open(data: ByteArray, key: SecretKey, context: ByteArray): ByteArray {
        if (data.size < NONCE_BYTES + GCM_TAG_BITS / 8) throw VaultError.InvalidFile()
        try {
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(
                Cipher.DECRYPT_MODE,
                key,
                GCMParameterSpec(GCM_TAG_BITS, data.copyOfRange(0, NONCE_BYTES)),
            )
            cipher.updateAAD(context)
            return cipher.doFinal(data, NONCE_BYTES, data.size - NONCE_BYTES)
        } catch (e: java.security.GeneralSecurityException) {
            // Wrong password, tampered file, or wrong AAD: the callers map
            // this to InvalidPassword (unlock) or InvalidFile (reads).
            throw VaultError.InvalidFile(cause = e)
        }
    }

    private fun randomBytes(count: Int): ByteArray =
        ByteArray(count).also { SecureRandom().nextBytes(it) }

    private fun requireStorable(host: Host) {
        try {
            HostConfiguration.decode(HostConfiguration(hosts = listOf(host)).encoded())
        } catch (e: HostConfiguration.DecodeError) {
            throw VaultError.InvalidFile()
        }
    }

    private fun canonicalId(raw: String): String = uuidOrThrow(raw)

    private fun canonicalIdOrNull(raw: String): String? =
        runCatching { uuidOrThrow(raw) }.getOrNull()

    private fun isUuid(raw: String): Boolean = runCatching {
        UUID.fromString(raw.trim())
        true
    }.getOrDefault(false)

    private fun uuidOrThrow(raw: String): String = try {
        UUID.fromString(raw.trim()).toString().uppercase()
    } catch (e: IllegalArgumentException) {
        throw VaultError.InvalidFile()
    }

    internal fun encodeBase64(data: ByteArray): String =
        java.util.Base64.getEncoder().encodeToString(data)
}
