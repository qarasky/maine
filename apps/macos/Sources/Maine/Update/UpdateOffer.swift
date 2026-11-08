import Foundation

/// A newer stable release the updater can offer, parsed from the GitHub
/// Releases API. Both assets are required: an update is never installed
/// without a checksum to verify it against.
struct UpdateOffer: Equatable, Sendable {
    let version: AppVersion
    let notes: String
    let dmgURL: URL
    let sha256URL: URL
    let pageURL: URL
}
