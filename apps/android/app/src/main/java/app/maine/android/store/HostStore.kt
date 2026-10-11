package app.maine.android.store

import app.maine.android.model.Host
import app.maine.android.model.SavedSession
import app.maine.android.vault.VaultStore
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

class HostStoreError(message: String) : Exception(message)

/**
 * Owns the host list backed by the encrypted vault.
 *
 * There is no backend and no syncing: the vault file is the only copy, and
 * it is rewritten atomically after every change. Mirrors the macOS
 * `HostStore`, including the rule that a host which would not load back is
 * never saved.
 */
class HostStore(val vault: VaultStore) {

    private val _hosts = MutableStateFlow<List<Host>>(emptyList())
    val hosts: StateFlow<List<Host>> = _hosts.asStateFlow()

    private val _loadFailure = MutableStateFlow<LoadFailure?>(null)
    val loadFailure: StateFlow<LoadFailure?> = _loadFailure.asStateFlow()

    private val _saveFailure = MutableStateFlow<String?>(null)
    val saveFailure: StateFlow<String?> = _saveFailure.asStateFlow()

    /** A load problem Maine refuses to guess about. The file is left
     * untouched so the user can look at it and decide. */
    data class LoadFailure(val message: String, val path: String)

    val persistsSessions: Boolean = true

    fun load() {
        try {
            _hosts.value = vault.hosts()
            _loadFailure.value = null
        } catch (e: VaultStore.VaultError.Locked) {
            _hosts.value = emptyList()
            _loadFailure.value = null
        } catch (e: Exception) {
            _hosts.value = emptyList()
            _loadFailure.value = LoadFailure(
                e.message ?: "Maine could not read this host list.",
                vault.file.absolutePath,
            )
        }
    }

    /** Adds a host. Returns failure instead of saving when invalid. */
    fun add(host: Host): Result<Unit> {
        if (!host.isValid) {
            return Result.failure(HostStoreError(host.issues.joinToString(" ")))
        }
        _hosts.value += host
        return save()
    }

    fun update(host: Host): Result<Unit> {
        if (!host.isValid) {
            return Result.failure(HostStoreError(host.issues.joinToString(" ")))
        }
        val index = _hosts.value.indexOfFirst { Host.idsEqual(it.id, host.id) }
        if (index < 0) return Result.failure(HostStoreError("That host no longer exists."))
        _hosts.value = _hosts.value.toMutableList().also { it[index] = host }
        return save()
    }

    fun delete(host: Host) {
        try {
            vault.deleteHost(host.id)
            _hosts.value = _hosts.value.filterNot { Host.idsEqual(it.id, host.id) }
            _saveFailure.value = null
        } catch (e: Exception) {
            _saveFailure.value = e.message
        }
    }

    fun host(id: String?): Host? {
        if (id == null) return null
        return _hosts.value.firstOrNull { Host.idsEqual(it.id, id) }
    }

    @Throws(Exception::class)
    fun saveHost(host: Host, password: String) {
        vault.saveHost(host, password)
        load()
    }

    @Throws(Exception::class)
    fun savedSessions(): List<SavedSession> = vault.savedSessions()

    @Throws(Exception::class)
    fun saveSessions(sessions: List<SavedSession>) = vault.saveSessions(sessions)

    @Throws(Exception::class)
    fun saveSession(session: SavedSession) = vault.saveSession(session)

    @Throws(Exception::class)
    fun deleteSavedSession(id: String) = vault.deleteSavedSession(id)

    /** Writes the whole list, refusing hosts that would not load back. */
    fun save(): Result<Unit> {
        val invalid = _hosts.value.filterNot { it.isValid }
        if (invalid.isNotEmpty()) {
            val names = invalid.joinToString(", ") { it.displayName }
            _saveFailure.value = "Maine did not save: $names need attention."
            return Result.failure(HostStoreError(_saveFailure.value!!))
        }
        return try {
            vault.saveHosts(_hosts.value)
            _saveFailure.value = null
            Result.success(Unit)
        } catch (e: Exception) {
            _saveFailure.value =
                "Maine could not write ${vault.file.absolutePath}: ${e.message}"
            Result.failure(HostStoreError(_saveFailure.value!!))
        }
    }

    fun reportSaveFailure(message: String) {
        _saveFailure.value = message
    }

    fun clearSaveFailure() {
        _saveFailure.value = null
    }
}
