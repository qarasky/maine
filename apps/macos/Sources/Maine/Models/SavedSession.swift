import Foundation

/// A portable snapshot, never a running shell or a command replay.
public struct SavedSession: Codable, Identifiable, Equatable, Sendable {
    public var id: UUID
    public var hostID: UUID
    public var name: String
    public var history: String
    public var savedAt: Date
    public var usesDefaultName: Bool?
    public var number: Int?

    public init(id: UUID = UUID(), hostID: UUID, name: String, history: String, savedAt: Date = Date(),
                usesDefaultName: Bool? = nil, number: Int? = nil) {
        self.id = id
        self.hostID = hostID
        self.name = name
        self.history = history
        self.savedAt = savedAt
        self.usesDefaultName = usesDefaultName
        self.number = number
    }
}
