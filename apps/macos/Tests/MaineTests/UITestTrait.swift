import Foundation
import Testing

extension Trait where Self == ConditionTrait {
    /// Tests that create, show, activate or resize real windows. They take over
    /// the screen while someone is working, so plain `swift test` skips them;
    /// run them deliberately with `MAINE_UI_TESTS=1 swift test`.
    static var uiTest: Self {
        .enabled(if: ProcessInfo.processInfo.environment["MAINE_UI_TESTS"] == "1",
                 "Opens real windows; set MAINE_UI_TESTS=1 to run")
    }
}
