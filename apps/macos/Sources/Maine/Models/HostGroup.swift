import Foundation

/// A named, user-ordered section of the sidebar. Hosts point at a group by ID
/// (`Host.groupID`); a group may be empty. Groups travel in the vault, so every
/// paired device sees the same ones. Whether a group is collapsed is
/// device-local state, not part of this type.
public struct HostGroup: Identifiable, Codable, Hashable, Sendable {
    public static let maxNameLength = 128

    public var id: UUID
    public var name: String

    public init(id: UUID = UUID(), name: String) {
        self.id = id
        self.name = name
    }
}
