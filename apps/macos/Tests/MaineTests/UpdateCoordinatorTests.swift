import Foundation
@testable import Maine
import Testing

/// Stands in for the GitHub request: counts calls and can fail on demand.
private actor FakeFeed {
    private var data: Data
    private var failure: (any Error)?
    private(set) var fetches = 0

    init(data: Data) {
        self.data = data
    }

    func fail() {
        failure = TestError()
    }

    func fetch() throws -> Data {
        fetches += 1
        if let failure { throw failure }
        return data
    }

    private struct TestError: Error {}
}

/// Scheduling, opt-outs and preferences for the updater. No network: every
/// test injects the release JSON.
@MainActor
@Suite
struct UpdateCoordinatorTests {
    @MainActor
    private final class Fixture {
        let suite = "app.maine.tests.update.\(UUID().uuidString)"
        let defaults: UserDefaults
        let feed: FakeFeed
        let coordinator: UpdateCoordinator

        init(current: String = "1.0.0", tag: String = "v1.1.0") {
            defaults = UserDefaults(suiteName: suite)!
            feed = FakeFeed(data: Self.release(tag: tag))
            coordinator = UpdateCoordinator(
                current: AppVersion(current)!,
                fetch: { [feed] in try await feed.fetch() },
                defaults: defaults
            )
        }

        func tearDown() {
            coordinator.vaultDidLock()
            defaults.removePersistentDomain(forName: suite)
        }

        /// Builds the shape GitHub's releases API returns, with both assets.
        private static func release(tag: String, notes: String = "Fixes and polish.") -> Data {
            let name = String(tag.dropFirst())
            let base = "https://github.com/qarasky/maine/releases/download/\(tag)/Maine-\(name)"
            let json = """
            {
              "tag_name": "\(tag)",
              "body": "\(notes)",
              "html_url": "https://github.com/qarasky/maine/releases/tag/\(tag)",
              "draft": false,
              "prerelease": false,
              "assets": [
                {"name": "Maine-\(name).dmg", "browser_download_url": "\(base).dmg"},
                {"name": "Maine-\(name).dmg.sha256", "browser_download_url": "\(base).dmg.sha256"}
              ]
            }
            """
            return Data(json.utf8)
        }
    }

    @Test
    func automaticCheckReturnsAnOfferForANewerRelease() async {
        let fixture = Fixture(current: "1.0.0", tag: "v1.1.0")
        defer { fixture.tearDown() }

        let offer = await fixture.coordinator.check(manual: false)

        #expect(offer?.version == AppVersion("1.1.0"))
        #expect(fixture.coordinator.lastChecked != nil)
        #expect(fixture.coordinator.phase == .idle)
        let fetches = await fixture.feed.fetches
        #expect(fetches == 1)
    }

    @Test
    func skippedVersionIsHiddenFromAutomaticChecksButNotManualOnes() async throws {
        let fixture = Fixture(current: "1.0.0", tag: "v1.1.0")
        defer { fixture.tearDown() }

        let offer = try #require(await fixture.coordinator.check(manual: true))
        fixture.coordinator.skip(offer)
        #expect(fixture.defaults.string(forKey: "app.maine.update.skippedVersion") == "1.1.0")

        let automatic = await fixture.coordinator.check(manual: false)
        #expect(automatic == nil)
        let manual = await fixture.coordinator.check(manual: true)
        #expect(manual?.version == AppVersion("1.1.0"))
        let fetches = await fixture.feed.fetches
        #expect(fetches == 3)
    }

    @Test
    func disabledToggleNeverFetches() async {
        let fixture = Fixture()
        defer { fixture.tearDown() }

        fixture.coordinator.automaticChecks = false
        #expect(fixture.defaults.object(forKey: "app.maine.update.automatic") as? Bool == false)

        let offer = await fixture.coordinator.check(manual: false)
        #expect(offer == nil)
        #expect(fixture.coordinator.phase == .idle)
        let fetches = await fixture.feed.fetches
        #expect(fetches == 0)

        // The choice outlives the session.
        let again = UpdateCoordinator(current: AppVersion("1.0.0")!, fetch: { Data() },
                                      defaults: fixture.defaults)
        #expect(!again.automaticChecks)
    }

    @Test
    func developmentBuildNeverAutoChecks() async {
        let fixture = Fixture(current: "0.0.0-dev", tag: "v1.1.0")
        defer { fixture.tearDown() }

        let automatic = await fixture.coordinator.check(manual: false)
        #expect(automatic == nil)
        let fetchesAfterAutomatic = await fixture.feed.fetches
        #expect(fetchesAfterAutomatic == 0)

        // A person asking explicitly still gets an answer.
        let manual = await fixture.coordinator.check(manual: true)
        #expect(manual?.version == AppVersion("1.1.0"))
        let fetchesAfterManual = await fixture.feed.fetches
        #expect(fetchesAfterManual == 1)
    }

    @Test
    func automaticFailureIsSilent() async {
        let fixture = Fixture()
        defer { fixture.tearDown() }
        await fixture.feed.fail()

        let offer = await fixture.coordinator.check(manual: false)

        #expect(offer == nil)
        #expect(fixture.coordinator.phase == .idle)
        #expect(fixture.coordinator.lastChecked == nil)
    }

    @Test
    func manualFailureSetsFailedPhase() async {
        let fixture = Fixture()
        defer { fixture.tearDown() }
        await fixture.feed.fail()

        let offer = await fixture.coordinator.check(manual: true)

        #expect(offer == nil)
        guard case .failed(let message) = fixture.coordinator.phase else {
            Issue.record("expected a failed phase, got \(fixture.coordinator.phase)")
            return
        }
        #expect(!message.isEmpty)
    }

    @Test
    func lockStopsTheTimer() {
        let fixture = Fixture()
        defer { fixture.tearDown() }

        #expect(!fixture.coordinator.isScheduling)
        fixture.coordinator.vaultDidUnlock()
        #expect(fixture.coordinator.isScheduling)
        fixture.coordinator.vaultDidLock()
        #expect(!fixture.coordinator.isScheduling)
    }

    @Test
    func manualCheckThatFindsNothingShowsUpToDate() async {
        let fixture = Fixture(current: "1.0.0", tag: "v1.0.0")
        defer { fixture.tearDown() }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let vault = PasswordStore(fileURL: directory.appendingPathComponent("vault.maine"),
                                  keychainService: "app.maine.tests.\(UUID())")
        let appState = AppState(passwords: vault)
        appState.vaultDidUnlock()
        fixture.coordinator.configure(appState: appState,
                                      sessions: SessionManager(store: HostStore(vault: vault)))

        let offer = await fixture.coordinator.check(manual: true)

        #expect(offer == nil)
        #expect(appState.alert?.title == "Maine is up to date (1.0.0)")
    }

    @Test
    func onlyAFullSHA256DigestIsAccepted() {
        #expect(UpdateCoordinator.isValidSHA256(String(repeating: "a", count: 64)))
        #expect(UpdateCoordinator.isValidSHA256(String(repeating: "0", count: 64)))
        #expect(!UpdateCoordinator.isValidSHA256(String(repeating: "a", count: 63)))
        #expect(!UpdateCoordinator.isValidSHA256(String(repeating: "a", count: 65)))
        #expect(!UpdateCoordinator.isValidSHA256(String(repeating: "g", count: 64)))
        #expect(!UpdateCoordinator.isValidSHA256(""))
    }
}
