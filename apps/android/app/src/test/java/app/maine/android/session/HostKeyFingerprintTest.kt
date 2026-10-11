package app.maine.android.session

import net.schmizz.sshj.common.Buffer
import net.schmizz.sshj.common.KeyType
import org.junit.Assert.assertEquals
import org.junit.Test
import java.util.Base64

/**
 * Host key fingerprints must match what OpenSSH prints, because that is the
 * value a user compares against their server's when a key change is reported.
 *
 * The vector is a throwaway ed25519 key whose fingerprint was produced by
 * `ssh-keygen -lf`, an independent implementation of the format.
 */
class HostKeyFingerprintTest {

    /** `ssh-ed25519 AAAAC3...` — the base64 blob only. */
    private val publicKeyBlob =
        "AAAAC3NzaC1lZDI1NTE5AAAAIEpgDscHDceFf+5bIyv20BCfWr+mc3hjGpG1LwphJuc1"

    /** What `ssh-keygen -lf` reports for the key above. */
    private val openSshFingerprint =
        "SHA256:26TgOQKT+mCaYKQ8zxDuyUWSmnjJnwCzCtqKQmtovrY"

    @Test
    fun matchesOpenSshFingerprintForEd25519() {
        val buffer = Buffer.PlainBuffer(Base64.getDecoder().decode(publicKeyBlob))
        // A wire-encoded public key is `string name` followed by the key fields.
        assertEquals("ssh-ed25519", buffer.readString())
        val key = KeyType.ED25519.readPubKeyFromBuffer(buffer)

        assertEquals("ssh-ed25519", HostKeyFingerprint.typeName(key))
        assertEquals(openSshFingerprint, HostKeyFingerprint.of(key))
    }
}
