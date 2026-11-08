import Foundation

/// A semantic version (`X.Y.Z` or `X.Y.Z-prerelease`) that parses from tags
/// like `v1.2.3` and orders by semver 2.0 precedence.
public struct AppVersion: Comparable, Hashable, Sendable, CustomStringConvertible {
    let major: Int
    let minor: Int
    let patch: Int
    /// Dot-separated identifiers after the hyphen; empty for a release.
    let prerelease: [String]

    /// The version local builds report when no version was injected.
    static let development = AppVersion("0.0.0-dev")!

    /// Accepts an optional leading `v`, then `X.Y.Z` with an optional
    /// `-pre.release` suffix.
    public init?(_ string: String) {
        var text = string.trimmingCharacters(in: .whitespacesAndNewlines)
        if text.hasPrefix("v") { text.removeFirst() }

        let halves = text.split(separator: "-", maxSplits: 1, omittingEmptySubsequences: false)
        guard halves.count <= 2 else { return nil }

        let core = halves[0].split(separator: ".", omittingEmptySubsequences: false)
        guard core.count == 3 else { return nil }
        let numbers = core.compactMap { Int($0) }
        guard numbers.count == 3, core.allSatisfy({ $0.allSatisfy(\.isNumber) }) else { return nil }

        var identifiers: [String] = []
        if halves.count == 2 {
            identifiers = halves[1].split(separator: ".", omittingEmptySubsequences: false).map(String.init)
            guard !identifiers.isEmpty else { return nil }
            for identifier in identifiers {
                guard !identifier.isEmpty else { return nil }
                let valid = identifier.allSatisfy { character in
                    character.isASCII && (character.isLetter || character.isNumber || character == "-")
                }
                guard valid else { return nil }
            }
        }

        self.major = numbers[0]
        self.minor = numbers[1]
        self.patch = numbers[2]
        self.prerelease = identifiers
    }

    /// True for versions like `1.0.0-beta.1`; the updater ignores these.
    public var isPrerelease: Bool { !prerelease.isEmpty }

    /// The version the running bundle reports, or `0.0.0-dev` outside a bundle.
    public static var current: AppVersion {
        guard let text = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String,
              let version = AppVersion(text) else {
            return development
        }
        return version
    }

    /// True for `0.0.0-dev`, which never auto-checks for updates.
    public var isDevelopment: Bool { self == Self.development }

    public var description: String {
        let core = "\(major).\(minor).\(patch)"
        guard isPrerelease else { return core }
        return core + "-" + prerelease.joined(separator: ".")
    }

    public static func < (lhs: AppVersion, rhs: AppVersion) -> Bool {
        if lhs.major != rhs.major { return lhs.major < rhs.major }
        if lhs.minor != rhs.minor { return lhs.minor < rhs.minor }
        if lhs.patch != rhs.patch { return lhs.patch < rhs.patch }

        switch (lhs.isPrerelease, rhs.isPrerelease) {
        case (false, false):
            return false
        case (false, true):
            // A release outranks any of its prereleases.
            return false
        case (true, false):
            return true
        case (true, true):
            return Self.prereleaseIsOrderedBefore(lhs.prerelease, rhs.prerelease)
        }
    }

    private static func prereleaseIsOrderedBefore(_ lhs: [String], _ rhs: [String]) -> Bool {
        for (left, right) in zip(lhs, rhs) where left != right {
            let leftNumber = Int(left)
            let rightNumber = Int(right)
            switch (leftNumber, rightNumber) {
            case let (leftNumber?, rightNumber?):
                return leftNumber < rightNumber
            case (_?, nil):
                // Numeric identifiers sort before alphanumeric ones.
                return true
            case (nil, _?):
                return false
            case (nil, nil):
                return left < right
            }
        }
        // All shared identifiers are equal: the shorter set is lower.
        return lhs.count < rhs.count
    }
}
