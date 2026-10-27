import Foundation

/// Groups plus the ordered host list, and the rules for rearranging them.
///
/// Display order is the order of `hosts`: ungrouped hosts first in the
/// sidebar, then each group with its hosts in array order. A host whose
/// `groupID` names no existing group is treated as ungrouped (and normalized
/// to `nil`), so a dangling reference can never hide a host.
public struct HostLayout: Equatable, Sendable {
    public private(set) var groups: [HostGroup]
    public private(set) var hosts: [Host]

    public init(groups: [HostGroup], hosts: [Host]) {
        self.groups = groups
        self.hosts = hosts
        normalize()
    }

    public var ungrouped: [Host] { hosts.filter { $0.groupID == nil } }

    public func hosts(in groupID: HostGroup.ID) -> [Host] {
        hosts.filter { $0.groupID == groupID }
    }

    public func group(id: HostGroup.ID?) -> HostGroup? {
        guard let id else { return nil }
        return groups.first { $0.id == id }
    }

    // MARK: - Groups

    @discardableResult
    public mutating func addGroup(named base: String = "New Group") -> HostGroup {
        var name = base
        var suffix = 2
        while groups.contains(where: { $0.name == name }) {
            name = "\(base) \(suffix)"
            suffix += 1
        }
        let group = HostGroup(name: name)
        groups.append(group)
        return group
    }

    public mutating func renameGroup(_ id: HostGroup.ID, to name: String) throws {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { throw HostStoreError("A group name is required.") }
        guard trimmed.count <= HostGroup.maxNameLength else {
            throw HostStoreError("Group names can be at most \(HostGroup.maxNameLength) characters.")
        }
        guard let index = groups.firstIndex(where: { $0.id == id }) else { return }
        groups[index].name = trimmed
    }

    /// Removes the group; its hosts become ungrouped and keep their order.
    public mutating func deleteGroup(_ id: HostGroup.ID) {
        groups.removeAll { $0.id == id }
        normalize()
    }

    /// Moves a group before `target`, or to the end when `target` is nil.
    public mutating func moveGroup(_ id: HostGroup.ID, before target: HostGroup.ID?) {
        guard id != target, let index = groups.firstIndex(where: { $0.id == id }) else { return }
        let group = groups.remove(at: index)
        if let target, let targetIndex = groups.firstIndex(where: { $0.id == target }) {
            groups.insert(group, at: targetIndex)
        } else {
            groups.append(group)
        }
    }

    // MARK: - Hosts

    /// Puts a host in `groupID` (nil = ungrouped). With `target`, it lands
    /// just before that host; otherwise after the last host of that section.
    public mutating func moveHost(_ id: Host.ID, toGroup groupID: HostGroup.ID?, before target: Host.ID?) {
        guard id != target, let index = hosts.firstIndex(where: { $0.id == id }) else { return }
        let groupID = groupID.flatMap { group(id: $0)?.id }
        var host = hosts.remove(at: index)
        host.groupID = groupID
        if let target, let targetIndex = hosts.firstIndex(where: { $0.id == target }),
           hosts[targetIndex].groupID == groupID {
            hosts.insert(host, at: targetIndex)
        } else if let last = hosts.lastIndex(where: { $0.groupID == groupID }) {
            hosts.insert(host, at: last + 1)
        } else {
            hosts.append(host)
        }
    }

    private mutating func normalize() {
        let known = Set(groups.map(\.id))
        for index in hosts.indices where hosts[index].groupID.map({ !known.contains($0) }) ?? false {
            hosts[index].groupID = nil
        }
    }
}
