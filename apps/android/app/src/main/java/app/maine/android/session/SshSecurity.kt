package app.maine.android.session

import org.bouncycastle.jce.provider.BouncyCastleProvider
import java.security.Security

/**
 * Installs the full BouncyCastle provider in place of Android's.
 *
 * Android ships a stripped-down BouncyCastle registered under the same name
 * ("BC") that sshj looks up by default, and it lacks the modern curves servers
 * advertise — the office Pi offers `ssh-ed25519`, which Android's provider
 * cannot do. sshj already depends on the full `bcprov-jdk18on`, so this only
 * swaps which provider answers to "BC".
 *
 * Must run before the first SSH connection; safe to call more than once.
 */
fun installSshSecurityProvider() {
    if (Security.getProvider("BC") is BouncyCastleProvider) return
    Security.removeProvider("BC")
    Security.addProvider(BouncyCastleProvider())
}
