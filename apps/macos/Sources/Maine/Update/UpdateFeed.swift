import Foundation

/// Parses the GitHub "latest release" response into an offer the updater can
/// act on. Pure: no networking here, so it is safe to test offline.
enum UpdateFeed {
    /// The only endpoint the updater reads.
    static let latestReleaseURL = URL(string: "https://api.github.com/repos/qarasky/maine/releases/latest")!

    private struct Release: Decodable {
        let tagName: String
        let body: String?
        let pageURL: String
        let draft: Bool
        let prerelease: Bool
        let assets: [Asset]

        enum CodingKeys: String, CodingKey {
            case tagName = "tag_name"
            case body
            case pageURL = "html_url"
            case draft
            case prerelease
            case assets
        }
    }

    private struct Asset: Decodable {
        let name: String
        let url: String

        enum CodingKeys: String, CodingKey {
            case name
            case url = "browser_download_url"
        }
    }

    /// Returns an offer only for a newer stable release that carries both the
    /// DMG and its checksum on GitHub over HTTPS. Anything else yields `nil`;
    /// unreadable JSON throws.
    static func offer(from json: Data, current: AppVersion) throws -> UpdateOffer? {
        let release = try JSONDecoder().decode(Release.self, from: json)

        guard !release.draft,
              !release.prerelease,
              let version = AppVersion(release.tagName),
              !version.isPrerelease,
              version > current,
              let pageURL = URL(string: release.pageURL),
              pageURL.scheme == "https",
              let dmgURL = assetURL(named: "Maine-\(version).dmg", in: release),
              let sha256URL = assetURL(named: "Maine-\(version).dmg.sha256", in: release) else {
            return nil
        }

        return UpdateOffer(
            version: version,
            notes: release.body ?? "",
            dmgURL: dmgURL,
            sha256URL: sha256URL,
            pageURL: pageURL
        )
    }

    /// The download URL of `name`, only when it points at GitHub over HTTPS.
    private static func assetURL(named name: String, in release: Release) -> URL? {
        guard let asset = release.assets.first(where: { $0.name == name }),
              let url = URL(string: asset.url),
              url.scheme == "https",
              url.host == "github.com" else {
            return nil
        }
        return url
    }

    /// The expected SHA-256 hex digest for `fileName` in a `shasum -a 256`
    /// checksum file, or `nil` when the file has no hex entry for it.
    static func expectedSHA256(from checksumFile: String, fileName: String) -> String? {
        for line in checksumFile.split(whereSeparator: \.isNewline) {
            let fields = line.split(whereSeparator: \.isWhitespace)
            guard fields.count >= 2 else { continue }
            let digest = fields[0]
            let isHex = !digest.isEmpty && digest.allSatisfy { $0.isASCII && $0.isHexDigit }
            guard isHex else { continue }
            let name = fields[1].hasPrefix("*") ? fields[1].dropFirst() : fields[1]
            guard name == fileName else { continue }
            return String(digest)
        }
        return nil
    }
}

/// Remembers the version the user chose to skip. Preferences only: never the
/// vault, never synced.
struct SkippedVersionStore {
    static let key = "app.maine.update.skippedVersion"

    private let defaults: UserDefaults

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
    }

    var skipped: AppVersion? {
        get {
            guard let text = defaults.string(forKey: Self.key) else { return nil }
            return AppVersion(text)
        }
        nonmutating set {
            guard let version = newValue else {
                defaults.removeObject(forKey: Self.key)
                return
            }
            defaults.set(version.description, forKey: Self.key)
        }
    }
}
