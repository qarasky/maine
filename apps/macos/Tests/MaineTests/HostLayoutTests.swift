import Foundation
@testable import Maine
import Testing

/// Groups and host order: the pure rules behind sidebar drag and drop.
@Suite
struct HostLayoutTests {
    private func host(_ name: String, in group: HostGroup? = nil) -> Maine.Host {
        var host = Maine.Host(name: name, hostname: "\(name.lowercased()).example")
        host.groupID = group?.id
        return host
    }

    private func names(_ hosts: [Maine.Host]) -> [String] { hosts.map(\.name) }

    @Test
    func ungroupedHostsAndEachGroupKeepTheirOwnOrder() {
        let prod = HostGroup(name: "Prod"), lab = HostGroup(name: "Lab")
        let layout = HostLayout(groups: [prod, lab],
                                hosts: [host("A"), host("P1", in: prod), host("L1", in: lab), host("B"), host("P2", in: prod)])
        #expect(names(layout.ungrouped) == ["A", "B"])
        #expect(names(layout.hosts(in: prod.id)) == ["P1", "P2"])
        #expect(names(layout.hosts(in: lab.id)) == ["L1"])
    }

    @Test
    func aHostPointingAtAMissingGroupIsUngrouped() {
        let gone = HostGroup(name: "Gone")
        let layout = HostLayout(groups: [], hosts: [host("A", in: gone)])
        #expect(names(layout.ungrouped) == ["A"])
        #expect(layout.hosts.first?.groupID == nil)
    }

    @Test
    func droppingOnAGroupAppendsTheHostToIt() {
        let prod = HostGroup(name: "Prod")
        let a = host("A"), p1 = host("P1", in: prod)
        var layout = HostLayout(groups: [prod], hosts: [a, p1, host("B")])
        layout.moveHost(a.id, toGroup: prod.id, before: nil)
        #expect(names(layout.ungrouped) == ["B"])
        #expect(names(layout.hosts(in: prod.id)) == ["P1", "A"])
    }

    @Test
    func droppingOnAHostTakesItsGroupAndPlace() {
        let prod = HostGroup(name: "Prod")
        let a = host("A"), p1 = host("P1", in: prod), p2 = host("P2", in: prod)
        var layout = HostLayout(groups: [prod], hosts: [a, p1, p2])
        layout.moveHost(a.id, toGroup: prod.id, before: p2.id)
        #expect(names(layout.hosts(in: prod.id)) == ["P1", "A", "P2"])
        #expect(layout.ungrouped.isEmpty)

        // Reorder inside the ungrouped list, and back out of the group.
        let b = host("B"), c = host("C")
        layout = HostLayout(groups: [prod], hosts: [b, c, p1])
        layout.moveHost(c.id, toGroup: nil, before: b.id)
        #expect(names(layout.ungrouped) == ["C", "B"])
        layout.moveHost(p1.id, toGroup: nil, before: nil)
        #expect(names(layout.ungrouped) == ["C", "B", "P1"])
        #expect(layout.hosts(in: prod.id).isEmpty)
    }

    @Test
    func movingAHostOntoItselfChangesNothing() {
        let a = host("A"), b = host("B")
        var layout = HostLayout(groups: [], hosts: [a, b])
        layout.moveHost(a.id, toGroup: nil, before: a.id)
        #expect(names(layout.hosts) == ["A", "B"])
    }

    @Test
    func groupsReorderAndDeletingOneKeepsItsHostsUngrouped() {
        let one = HostGroup(name: "One"), two = HostGroup(name: "Two"), three = HostGroup(name: "Three")
        var layout = HostLayout(groups: [one, two, three],
                                hosts: [host("A"), host("T1", in: two), host("T2", in: two)])
        layout.moveGroup(three.id, before: one.id)
        #expect(layout.groups.map(\.name) == ["Three", "One", "Two"])
        layout.moveGroup(three.id, before: nil)
        #expect(layout.groups.map(\.name) == ["One", "Two", "Three"])

        layout.deleteGroup(two.id)
        #expect(layout.groups.map(\.name) == ["One", "Three"])
        #expect(names(layout.ungrouped) == ["A", "T1", "T2"])
    }

    @Test
    func newGroupsGetUniqueNamesAndRenamesAreTrimmed() throws {
        var layout = HostLayout(groups: [], hosts: [])
        let first = layout.addGroup()
        let second = layout.addGroup()
        #expect(first.name == "New Group")
        #expect(second.name == "New Group 2")
        try layout.renameGroup(first.id, to: "  Production  ")
        #expect(layout.groups.first?.name == "Production")
        #expect(throws: HostStoreError.self) { try layout.renameGroup(first.id, to: "   ") }
        #expect(layout.groups.first?.name == "Production")
    }
}
