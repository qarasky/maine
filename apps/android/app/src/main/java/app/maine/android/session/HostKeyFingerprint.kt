package app.maine.android.session

import net.schmizz.sshj.common.Buffer
import net.schmizz.sshj.common.KeyType
import java.security.MessageDigest
import java.security.PublicKey
import java.util.Base64

/**
 * Host key fingerprints in the format the SSH tooling prints.
 *
 * The digest is taken over the key's SSH wire encoding — `string name` followed
 * by the key fields, per RFC 4253 section 6.6 — which is what `ssh-keygen -lf`
 * and `ssh-keyscan` hash. That matters: a fingerprint shown to the user is only
 * useful if it can be compared against the one their server reports.
 */
object HostKeyFingerprint {

    /** `SHA256:base64`, the format `ssh-keygen -l` prints. */
    fun of(key: PublicKey): String? {
        val blob = wireEncoding(key) ?: return null
        val digest = MessageDigest.getInstance("SHA-256").digest(blob)
        return "SHA256:" + Base64.getEncoder().withoutPadding().encodeToString(digest)
    }

    /** The protocol name, e.g. `ssh-ed25519`. */
    fun typeName(key: PublicKey): String = protocolName(KeyType.fromKey(key))

    /**
     * The SSH wire encoding of a public key, which sshj writes whole: the type
     * name as a length-prefixed string followed by the key fields. Returns null
     * for a key sshj cannot encode, which callers treat as untrustworthy rather
     * than as a match.
     */
    private fun wireEncoding(key: PublicKey): ByteArray? = try {
        val buffer = Buffer.PlainBuffer()
        KeyType.fromKey(key).putPubKeyIntoBuffer(key, buffer)
        buffer.compactData
    } catch (e: Exception) {
        null
    }

    private fun protocolName(type: KeyType): String = when (type) {
        KeyType.RSA -> "ssh-rsa"
        KeyType.DSA -> "ssh-dss"
        KeyType.ED25519 -> "ssh-ed25519"
        KeyType.ECDSA256 -> "ecdsa-sha2-nistp256"
        KeyType.ECDSA384 -> "ecdsa-sha2-nistp384"
        KeyType.ECDSA521 -> "ecdsa-sha2-nistp521"
        KeyType.SK_ED25519 -> "sk-ssh-ed25519@openssh.com"
        KeyType.SK_ECDSA -> "sk-ecdsa-sha2-nistp256@openssh.com"
        else -> type.name.lowercase()
    }
}
