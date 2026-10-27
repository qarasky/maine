import Foundation

/// Which hosts MCP agents may use on this device. A host is allowed when it
/// is listed itself or its group is listed; allowed means fully usable. There
/// are no modes. Stored per vault in the encrypted local state file, never in
/// the vault (so it never syncs) and never in preferences.
public struct AgentAccess: Codable, Equatable, Sendable {
    public var groups: Set<UUID>
    public var hosts: Set<UUID>

    public init(groups: Set<UUID> = [], hosts: Set<UUID> = []) {
        self.groups = groups
        self.hosts = hosts
    }
}

public extension AgentAccess {
    func allows(_ host: Host) -> Bool {
        hosts.contains(host.id) || host.groupID.map(groups.contains) == true
    }

    /// Allowed hosts in sidebar order.
    func allowedHosts(in layout: HostLayout) -> [Host] {
        layout.ungrouped.filter(allows) + layout.groups.flatMap { layout.hosts(in: $0.id).filter(allows) }
    }

    /// Drops rules naming groups or hosts the layout no longer has.
    func pruned(to layout: HostLayout) -> AgentAccess {
        AgentAccess(groups: groups.intersection(layout.groups.map(\.id)),
                    hosts: hosts.intersection(layout.hosts.map(\.id)))
    }
}
