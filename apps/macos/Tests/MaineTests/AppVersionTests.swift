import Maine
import Testing

@Suite
struct AppVersionTests {
    @Test
    func parsesWithAndWithoutV() {
        let prefixed = AppVersion("v1.2.3")
        let plain = AppVersion("1.2.3")

        #expect(prefixed != nil)
        #expect(prefixed == plain)
        #expect(prefixed?.description == "1.2.3")
        #expect(prefixed?.isPrerelease == false)
    }

    @Test
    func rejectsGarbage() {
        for text in ["1.2", "1.2.x", "", "1.2.3.4", "v1.2", "1..2", "1.2.3-"] {
            #expect(AppVersion(text) == nil, "expected \(text) to be rejected")
        }
    }

    @Test
    func comparesNumerically() {
        #expect(AppVersion("1.10.0")! > AppVersion("1.9.0")!)
        #expect(AppVersion("2.0.0")! > AppVersion("1.99.99")!)
    }

    @Test
    func prereleaseSortsBeforeRelease() {
        #expect(AppVersion("1.0.0-beta.1")! < AppVersion("1.0.0")!)
        #expect(AppVersion("1.0.0-beta.2")! > AppVersion("1.0.0-beta.1")!)
        #expect(AppVersion("1.0.0-beta.10")! > AppVersion("1.0.0-beta.2")!)
    }

    @Test
    func devVersionIsDevelopment() {
        #expect(AppVersion("0.0.0-dev")?.isDevelopment == true)
        #expect(AppVersion("1.0.0")?.isDevelopment == false)
    }
}
