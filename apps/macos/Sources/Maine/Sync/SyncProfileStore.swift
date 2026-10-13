import Foundation

/// The remembered sync pairings, in preferences, and their tokens, in the
/// Keychain. Exactly one profile is active at a time; the active profile's
/// vault is the live `passwords.maine`.
@MainActor
public final class SyncProfileStore: ObservableObject {
    @Published public private(set) var profiles: [SyncProfile] = []
    @Published public private(set) var activeProfileID: UUID?

    private let defaults: UserDefaults
    private let tokens: SyncTokenStore
    private static let profilesKey = "MaineSyncProfiles"
    private static let activeKey = "MaineSyncActiveProfile"

    public init(defaults: UserDefaults = .standard, keychainService: String = "Maine Sync") {
        self.defaults = defaults
        tokens = SyncTokenStore(service: keychainService)
        if let data = defaults.data(forKey: Self.profilesKey),
           let decoded = try? JSONDecoder().decode([SyncProfile].self, from: data) {
            profiles = decoded
        }
        if let raw = defaults.string(forKey: Self.activeKey) {
            activeProfileID = UUID(uuidString: raw)
        }
        if let active = activeProfileID, !profiles.contains(where: { $0.id == active }) {
            activeProfileID = nil
        }
    }

    public var activeProfile: SyncProfile? {
        profiles.first { $0.id == activeProfileID }
    }

    public var hasProfiles: Bool { !profiles.isEmpty }

    public func profile(for id: UUID) -> SyncProfile? {
        profiles.first { $0.id == id }
    }

    public func token(for id: UUID) throws -> String? {
        try tokens.token(for: id)
    }

    public func add(_ profile: SyncProfile, token: String) throws {
        try tokens.save(token, for: profile.id)
        profiles.append(profile)
        if activeProfileID == nil { activeProfileID = profile.id }
        persist()
    }

    public func update(id: UUID, _ change: (inout SyncProfile) -> Void) {
        guard let index = profiles.firstIndex(where: { $0.id == id }) else { return }
        change(&profiles[index])
        persist()
    }

    /// Persists a rotated token before anything acknowledges the rotation.
    public func replaceToken(_ token: String, for id: UUID) throws {
        try tokens.save(token, for: id)
    }

    /// Forgets the device's half of a pairing: the profile and its token.
    /// Nothing on the server changes.
    public func remove(_ id: UUID) throws {
        try tokens.delete(for: id)
        profiles.removeAll { $0.id == id }
        if activeProfileID == id { activeProfileID = nil }
        persist()
    }

    public func setActive(_ id: UUID?) {
        activeProfileID = id
        persist()
    }

    private func persist() {
        if let data = try? JSONEncoder().encode(profiles) {
            defaults.set(data, forKey: Self.profilesKey)
        }
        defaults.set(activeProfileID?.uuidString, forKey: Self.activeKey)
    }
}
