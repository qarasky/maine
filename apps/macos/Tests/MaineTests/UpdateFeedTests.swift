import Foundation
import Testing

@testable import Maine

@Suite
struct UpdateFeedTests {
    /// Builds the shape GitHub's releases API returns, with both assets.
    private func release(
        tag: String,
        notes: String = "Fixes and polish.",
        draft: Bool = false,
        prerelease: Bool = false,
        includeDMG: Bool = true,
        includeChecksum: Bool = true,
        dmgURL: String? = nil,
        sha256URL: String? = nil
    ) -> Data {
        let name = String(tag.dropFirst())
        let base = "https://github.com/qarasky/maine/releases/download/\(tag)/Maine-\(name)"
        var assets: [String] = []
        if includeDMG {
            assets.append(#"{"name": "Maine-\#(name).dmg", "browser_download_url": "\#(dmgURL ?? base + ".dmg")"}"#)
        }
        if includeChecksum {
            assets.append(#"{"name": "Maine-\#(name).dmg.sha256", "browser_download_url": "\#(sha256URL ?? base + ".dmg.sha256")"}"#)
        }
        let json = """
        {
          "tag_name": "\(tag)",
          "body": "\(notes)",
          "html_url": "https://github.com/qarasky/maine/releases/tag/\(tag)",
          "draft": \(draft),
          "prerelease": \(prerelease),
          "assets": [\(assets.joined(separator: ","))]
        }
        """
        return Data(json.utf8)
    }

    private let current = AppVersion("1.0.0")!

    @Test
    func offersANewerStableRelease() throws {
        let data = release(tag: "v1.1.0", notes: "Faster sessions.")
        let offer = try UpdateFeed.offer(from: data, current: current)

        #expect(offer?.version == AppVersion("1.1.0"))
        #expect(offer?.notes == "Faster sessions.")
        #expect(offer?.dmgURL.absoluteString == "https://github.com/qarasky/maine/releases/download/v1.1.0/Maine-1.1.0.dmg")
        #expect(offer?.sha256URL.absoluteString == "https://github.com/qarasky/maine/releases/download/v1.1.0/Maine-1.1.0.dmg.sha256")
        #expect(offer?.pageURL.absoluteString == "https://github.com/qarasky/maine/releases/tag/v1.1.0")
    }

    @Test
    func ignoresEqualOrOlder() throws {
        #expect(try UpdateFeed.offer(from: release(tag: "v1.0.0"), current: current) == nil)
        #expect(try UpdateFeed.offer(from: release(tag: "v0.9.0"), current: current) == nil)
    }

    @Test
    func ignoresDraftAndPrerelease() throws {
        #expect(try UpdateFeed.offer(from: release(tag: "v1.1.0", draft: true), current: current) == nil)
        #expect(try UpdateFeed.offer(from: release(tag: "v1.1.0", prerelease: true), current: current) == nil)
        #expect(try UpdateFeed.offer(from: release(tag: "v1.1.0-beta.1"), current: current) == nil)
    }

    @Test
    func ignoresReleaseMissingDMGOrChecksum() throws {
        let onlyDMG = release(tag: "v1.1.0", includeChecksum: false)
        let onlyChecksum = release(tag: "v1.1.0", includeDMG: false)
        #expect(try UpdateFeed.offer(from: onlyDMG, current: current) == nil)
        #expect(try UpdateFeed.offer(from: onlyChecksum, current: current) == nil)

        // A checksum asset named for a different version does not count.
        let wrongName = Data("""
        {
          "tag_name": "v1.1.0",
          "body": "",
          "html_url": "https://github.com/qarasky/maine/releases/tag/v1.1.0",
          "draft": false,
          "prerelease": false,
          "assets": [
            {"name": "Maine-1.1.0.dmg", "browser_download_url": "https://github.com/qarasky/maine/releases/download/v1.1.0/Maine-1.1.0.dmg"},
            {"name": "Maine-1.0.0.dmg.sha256", "browser_download_url": "https://github.com/qarasky/maine/releases/download/v1.1.0/Maine-1.0.0.dmg.sha256"}
          ]
        }
        """.utf8)
        #expect(try UpdateFeed.offer(from: wrongName, current: current) == nil)
    }

    @Test
    func ignoresNonGithubOrNonHTTPSAssetURLs() throws {
        let httpDMG = release(tag: "v1.1.0", dmgURL: "http://github.com/qarasky/maine/releases/download/v1.1.0/Maine-1.1.0.dmg")
        let otherHost = release(tag: "v1.1.0", dmgURL: "https://example.com/Maine-1.1.0.dmg")
        let httpChecksum = release(tag: "v1.1.0", sha256URL: "http://github.com/qarasky/maine/releases/download/v1.1.0/Maine-1.1.0.dmg.sha256")
        let otherChecksumHost = release(tag: "v1.1.0", sha256URL: "https://example.com/Maine-1.1.0.dmg.sha256")

        #expect(try UpdateFeed.offer(from: httpDMG, current: current) == nil)
        #expect(try UpdateFeed.offer(from: otherHost, current: current) == nil)
        #expect(try UpdateFeed.offer(from: httpChecksum, current: current) == nil)
        #expect(try UpdateFeed.offer(from: otherChecksumHost, current: current) == nil)
    }

    @Test
    func compares110Above19() throws {
        let offer = try UpdateFeed.offer(from: release(tag: "v1.10.0"), current: AppVersion("1.9.0")!)
        #expect(offer?.version == AppVersion("1.10.0"))
    }

    @Test
    func malformedJSONThrows() {
        #expect(throws: (any Error).self) {
            try UpdateFeed.offer(from: Data("not json".utf8), current: current)
        }
        #expect(throws: (any Error).self) {
            try UpdateFeed.offer(from: Data(#"{"tag_name": "v1.1.0"}"#.utf8), current: current)
        }
    }

    @Test
    func parsesChecksumLine() {
        #expect(UpdateFeed.expectedSHA256(from: "abc123  Maine-1.2.3.dmg\n", fileName: "Maine-1.2.3.dmg") == "abc123")
        #expect(UpdateFeed.expectedSHA256(from: "abc123  Maine-1.2.4.dmg\n", fileName: "Maine-1.2.3.dmg") == nil)
        #expect(UpdateFeed.expectedSHA256(from: "zzzz  Maine-1.2.3.dmg\n", fileName: "Maine-1.2.3.dmg") == nil)
        #expect(UpdateFeed.expectedSHA256(from: "", fileName: "Maine-1.2.3.dmg") == nil)

        // shasum writes a blank line first when the file list is multi-line.
        let multiLine = "aaaa  Maine-1.0.0.dmg\nabc123  Maine-1.2.3.dmg\n"
        #expect(UpdateFeed.expectedSHA256(from: multiLine, fileName: "Maine-1.2.3.dmg") == "abc123")
    }

    @Test
    func skippedVersionRoundTrips() {
        let suite = "UpdateFeedTests-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }

        let store = SkippedVersionStore(defaults: defaults)
        #expect(store.skipped == nil)

        store.skipped = AppVersion("1.2.3")
        #expect(store.skipped == AppVersion("1.2.3"))
        #expect(defaults.string(forKey: "app.maine.update.skippedVersion") == "1.2.3")

        store.skipped = nil
        #expect(store.skipped == nil)
    }
}
