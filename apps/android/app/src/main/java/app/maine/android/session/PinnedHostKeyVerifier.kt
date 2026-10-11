package app.maine.android.session

import net.schmizz.sshj.transport.verification.HostKeyVerifier
import java.security.PublicKey

/**
 * Trust on first use for a host's key.
 *
 * The first key seen for a host is remembered; every later key must match it.
 * A key that does not match is refused rather than accepted silently, and both
 * fingerprints are recorded so the change can be reported and checked against
 * the server — the one case where a silent "just reconnect" would be exactly
 * the attack this is meant to catch.
 */
class PinnedHostKeyVerifier(
    private val hostId: String,
    private val knownHosts: KnownHosts,
) : HostKeyVerifier {

    /**
     * Set when a key did not match the pinned one. sshj reports a refusal with
     * its own generic message, so the specific reason is kept here and used in
     * preference to it.
     */
    @Volatile
    var mismatch: String? = null
        private set

    override fun verify(hostname: String, port: Int, key: PublicKey): Boolean {
        val fingerprint = HostKeyFingerprint.of(key) ?: return false
        val keyType = HostKeyFingerprint.typeName(key)

        val pinned = knownHosts.entry(hostId)
        if (pinned == null) {
            knownHosts.pin(hostId, KnownHosts.Entry(keyType, fingerprint))
            return true
        }
        if (pinned.fingerprint == fingerprint) return true

        mismatch = "Host key changed: the server presents $keyType $fingerprint, but was last " +
            "seen with ${pinned.keyType} ${pinned.fingerprint}. If the server was rebuilt or " +
            "reinstalled, delete this host and add it again to trust the new key."
        return false
    }

    /** sshj uses this only to report which algorithms a known host offers. */
    override fun findExistingAlgorithms(hostname: String, port: Int): List<String> = emptyList()
}
