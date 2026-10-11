package app.maine.android.ui

import android.app.Application
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import app.maine.android.model.Host
import app.maine.android.session.SessionManager
import app.maine.android.session.SshjConnection
import app.maine.android.session.TerminalSession
import app.maine.android.store.HostStore
import app.maine.android.vault.VaultStore
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File

/**
 * Window-level UI state shared by every screen, mirroring the macOS
 * `AppState`: the vault gate, the host being edited, and the managers.
 * The master password is asked on every launch; there is no "remember me".
 */
class MaineViewModel(application: Application) : AndroidViewModel(application) {

    private val vault = VaultStore(File(application.filesDir, VaultStore.FILE_NAME))
    val store = HostStore(vault)

    /**
     * The app wires the live sshj transport here; [TerminalSession] and
     * [SessionManager] default to the scripted fake so JVM unit tests stay
     * hermetic and never touch the network.
     */
    val sessions = SessionManager(
        store,
        vault,
        viewModelScope,
        makeSession = { host ->
            TerminalSession(host = host, makeConnection = { SshjConnection() })
        },
    )

    var locked by mutableStateOf(true)
        private set
    var vaultExists by mutableStateOf(vault.exists)
        private set
    var unlockError by mutableStateOf<String?>(null)
        private set
    /** PBKDF2 (600,000 iterations) runs off the UI thread. */
    var unlockBusy by mutableStateOf(false)
        private set
    var searchQuery by mutableStateOf("")
    var editingHost by mutableStateOf<Host?>(null)
        private set
    var editorPassword by mutableStateOf("")
    var editorError by mutableStateOf<String?>(null)
        private set

    init {
        store.load()
    }

    fun unlock(masterPassword: String) {
        if (unlockBusy) return
        unlockError = null
        unlockBusy = true
        viewModelScope.launch {
            val result = withContext(Dispatchers.IO) {
                runCatching {
                    vault.unlock(masterPassword)
                    store.load()
                    sessions.restoreSavedSessions()
                }
            }
            result
                .onSuccess {
                    vaultExists = true
                    locked = false
                }
                .onFailure { e ->
                    unlockError = e.message
                }
            unlockBusy = false
        }
    }

    fun lock() {
        viewModelScope.launch {
            runCatching { sessions.checkpointSessions() }
            runCatching { vault.lock() }
            locked = true
            unlockError = null
            store.load()
        }
    }

    /** Best-effort checkpoint when the app goes to the background. */
    fun onBackgrounded() {
        viewModelScope.launch {
            runCatching { sessions.checkpointSessions() }
        }
    }

    fun beginNewHost() {
        editingHost = Host(name = "", hostname = "")
        editorPassword = ""
        editorError = null
    }

    fun beginEditHost(host: Host) {
        editingHost = host
        editorPassword = runCatching { vault.passwordFor(host.id) }.getOrNull() ?: ""
        editorError = null
    }

    fun cancelEdit() {
        editingHost = null
        editorError = null
    }

    /** Saves the host being edited. Returns true when the dialog can close. */
    fun commitEdit(host: Host): Boolean {
        if (editorPassword.isEmpty()) {
            editorError = "A password is required to save a host."
            return false
        }
        return try {
            val isNew = store.host(host.id) == null
            if (isNew) {
                val result = store.add(host)
                if (result.isFailure) {
                    editorError = result.exceptionOrNull()?.message
                    return false
                }
            } else {
                val result = store.update(host)
                if (result.isFailure) {
                    editorError = result.exceptionOrNull()?.message
                    return false
                }
            }
            store.saveHost(host, editorPassword)
            store.clearSaveFailure()
            editingHost = null
            editorError = null
            true
        } catch (e: Exception) {
            editorError = e.message
            false
        }
    }

    fun deleteHost(host: Host) {
        sessions.delete(host)
    }

    override fun onCleared() {
        sessions.stopProcessesForQuit()
        super.onCleared()
    }
}
