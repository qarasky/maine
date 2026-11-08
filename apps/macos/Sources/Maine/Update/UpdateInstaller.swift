import CryptoKit
import Foundation

/// Why an update could not be installed. The installed app is untouched in
/// every case; the caller decides between retrying, explaining, or opening the
/// release page.
enum UpdateInstallError: LocalizedError {
    case notWritable
    case checksumMismatch
    case badDownload
    case mountFailed
    case wrongApp

    var errorDescription: String? {
        switch self {
        case .notWritable:
            "Maine cannot update itself because its folder is not writable. Download the new version from the release page instead."
        case .checksumMismatch:
            "The downloaded update did not match its checksum. It was deleted and the installed app was not changed."
        case .badDownload:
            "The update could not be downloaded."
        case .mountFailed:
            "The update disk image could not be opened."
        case .wrongApp:
            "The update disk image does not contain the expected Maine app."
        }
    }
}

/// Downloads a release disk image, verifies it and stages the new bundle next
/// to the running one. A detached shell helper performs the swap once this
/// process has quit, because a running bundle cannot be replaced in place.
enum UpdateInstaller {
    static let bundleIdentifier = "app.maine.Maine"

    /// Hex SHA-256 of a file, streamed so a large disk image stays out of memory.
    static func sha256(ofFileAt url: URL) throws -> String {
        let handle = try FileHandle(forReadingFrom: url)
        defer { try? handle.close() }
        var hasher = SHA256()
        while let chunk = try handle.read(upToCount: 1 << 20), !chunk.isEmpty {
            hasher.update(data: chunk)
        }
        return hasher.finalize().map { String(format: "%02x", $0) }.joined()
    }

    /// True when the bundle can be replaced in place: a `.app` outside
    /// `/Volumes/` and outside App Translocation, with a writable parent folder.
    static func canInstall(over app: URL) -> Bool {
        let path = app.path
        guard app.pathExtension.lowercased() == "app",
              !path.hasPrefix("/Volumes/"),
              !path.contains("AppTranslocation") else {
            return false
        }
        return FileManager.default.isWritableFile(atPath: app.deletingLastPathComponent().path)
    }

    /// The helper script. Paths reach it as arguments (`$1`...`$4`) and are
    /// never spliced into the script text, so quotes and spaces stay data.
    /// Arguments: parent pid, staged app, target app, relaunch command.
    static func swapScript(parentPID: Int32, staged: URL, target: URL,
                           relaunch: String = "/usr/bin/open") -> String {
        guard parentPID > 0,
              target.pathExtension.lowercased() == "app",
              staged.pathExtension.lowercased() == "app",
              staged != target,
              relaunch.hasPrefix("/") else {
            return """
            #!/bin/bash
            echo 'Maine update helper: invalid arguments' >&2
            exit 1
            """
        }
        return swapScriptBody
    }

    /// Waits for the app to quit, swaps the bundles and relaunches. On failure
    /// the old bundle is moved back and the script exits nonzero.
    private static let swapScriptBody = """
    #!/bin/bash
    # Maine update helper. Arguments: parent pid, staged app, target app,
    # relaunch command. Every path arrives as an argument, never as script text.
    set -u

    parent="${1:?missing parent pid}"
    staged="${2:?missing staged app}"
    target="${3:?missing target app}"
    relaunch="${4:-/usr/bin/open}"

    # Wait for this app to exit, for at most 60 seconds.
    tries=0
    while kill -0 "$parent" 2>/dev/null; do
        if [ "$tries" -ge 240 ]; then
            break
        fi
        sleep 0.25
        tries=$((tries + 1))
    done

    if [ ! -d "$target" ]; then
        exit 1
    fi

    backup="${target}.old"
    rm -rf "$backup"
    if ! mv "$target" "$backup"; then
        exit 1
    fi
    if ! mv "$staged" "$target"; then
        mv "$backup" "$target"
        # The app already quit: bring the old version back up.
        "$relaunch" "$target"
        exit 1
    fi
    rm -rf "$backup"

    # Remove the staging folder only when it is one of ours.
    staging="${staged%/*}"
    case "${staging##*/}" in
        .Maine-update-*) rm -rf "$staging" ;;
    esac

    "$relaunch" "$target"
    rm -f "$0"
    """

    /// Downloads, verifies and copies the new bundle into
    /// `<target parent>/.Maine-update-<uuid>/Maine.app`. Returns that URL. The
    /// caller must check `canInstall(over:)` before asking for this.
    static func stage(_ offer: UpdateOffer, expectedSHA256: String,
                      over target: URL) async throws -> URL {
        let fileManager = FileManager.default
        let work = fileManager.temporaryDirectory
            .appendingPathComponent("Maine-update-\(UUID().uuidString)", isDirectory: true)
        do {
            try fileManager.createDirectory(at: work, withIntermediateDirectories: true)
        } catch {
            throw UpdateInstallError.badDownload
        }
        defer { try? fileManager.removeItem(at: work) }

        let download: URL
        let response: URLResponse
        do {
            (download, response) = try await URLSession.shared.download(from: offer.dmgURL)
        } catch {
            throw UpdateInstallError.badDownload
        }
        guard let http = response as? HTTPURLResponse, http.statusCode == 200 else {
            try? fileManager.removeItem(at: download)
            throw UpdateInstallError.badDownload
        }

        let dmg = work.appendingPathComponent("Maine-\(offer.version).dmg")
        do {
            try fileManager.moveItem(at: download, to: dmg)
        } catch {
            try? fileManager.removeItem(at: download)
            throw UpdateInstallError.badDownload
        }

        // Verify before anything is mounted or copied.
        let digest: String
        do {
            digest = try sha256(ofFileAt: dmg)
        } catch {
            throw UpdateInstallError.badDownload
        }
        guard digest.caseInsensitiveCompare(expectedSHA256) == .orderedSame else {
            try? fileManager.removeItem(at: dmg)
            throw UpdateInstallError.checksumMismatch
        }

        let mount = work.appendingPathComponent("volume", isDirectory: true)
        do {
            try fileManager.createDirectory(at: mount, withIntermediateDirectories: true)
        } catch {
            throw UpdateInstallError.mountFailed
        }
        var mounted = false
        defer {
            if mounted {
                _ = try? run("/usr/bin/hdiutil", ["detach", mount.path, "-force"])
            }
        }
        guard (try? run("/usr/bin/hdiutil",
                        ["attach", dmg.path, "-nobrowse", "-readonly", "-mountpoint", mount.path])) == 0 else {
            throw UpdateInstallError.mountFailed
        }
        mounted = true

        let source = mount.appendingPathComponent("Maine.app", isDirectory: true)
        guard fileManager.fileExists(atPath: source.path) else {
            throw UpdateInstallError.wrongApp
        }

        let staging = target.deletingLastPathComponent()
            .appendingPathComponent(".Maine-update-\(UUID().uuidString)", isDirectory: true)
        do {
            try fileManager.createDirectory(at: staging, withIntermediateDirectories: false)
        } catch {
            throw UpdateInstallError.notWritable
        }
        var keepStaging = false
        defer {
            if !keepStaging {
                try? fileManager.removeItem(at: staging)
            }
        }

        let staged = staging.appendingPathComponent("Maine.app", isDirectory: true)
        guard (try? run("/usr/bin/ditto", [source.path, staged.path])) == 0 else {
            throw UpdateInstallError.badDownload
        }
        guard let bundle = Bundle(url: staged),
              bundle.bundleIdentifier == bundleIdentifier,
              let version = bundle.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String,
              AppVersion(version) == offer.version else {
            throw UpdateInstallError.wrongApp
        }

        keepStaging = true
        return staged
    }

    /// Writes the swap script and starts it detached, so it survives this
    /// process quitting. Paths are passed as arguments, never as script text.
    static func launchSwap(parentPID: Int32, staged: URL, target: URL) throws {
        let scriptURL = FileManager.default.temporaryDirectory
            .appendingPathComponent("Maine-update-\(UUID().uuidString).sh")
        try swapScript(parentPID: parentPID, staged: staged, target: target)
            .write(to: scriptURL, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o700],
                                              ofItemAtPath: scriptURL.path)

        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/bin/bash")
        // Static command text; the paths follow as `bash -c` arguments.
        process.arguments = [
            "-c",
            #"/usr/bin/nohup /bin/bash "$0" "$@" >/dev/null 2>&1 &"#,
            scriptURL.path,
            String(parentPID),
            staged.path,
            target.path,
            "/usr/bin/open",
        ]
        process.standardInput = FileHandle.nullDevice
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        try process.run()
    }

    // MARK: - Commands

    @discardableResult
    private static func run(_ executable: String, _ arguments: [String]) throws -> Int32 {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: executable)
        process.arguments = arguments
        process.standardInput = FileHandle.nullDevice
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        try process.run()
        process.waitUntilExit()
        return process.terminationStatus
    }
}
