import Foundation
import Maine
import Testing

/// The versioned JSON format every Maine client shares.
@Suite
struct HostConfigurationTests {
    private let host = Host(
        id: UUID(uuidString: "0F8FAD5B-D9CB-469F-A165-70867728950E")!,
        name: "Web server",
        hostname: "203.0.113.10",
        port: 22,
        username: ""
    )

    @Test
    func roundTripKeepsEveryField() throws {
        let configuration = HostConfiguration(hosts: [host])
        let decoded = try HostConfiguration(data: configuration.encoded())

        #expect(decoded.schemaVersion == HostConfiguration.currentSchemaVersion)
        #expect(decoded.hosts == [host])
    }

    @Test
    func encodedFileHasTheFieldsTheSchemaRequires() throws {
        let object = try #require(
            try JSONSerialization.jsonObject(with: HostConfiguration(hosts: [host]).encoded())
                as? [String: Any]
        )

        #expect(object["schemaVersion"] as? Int == 2)

        let entry = try #require((object["hosts"] as? [[String: Any]])?.first)
        for key in ["id", "name", "hostname", "port", "username"] {
            #expect(entry[key] != nil, "\(key) is required by the schema")
        }
        #expect(entry["port"] as? Int == 22)
        // An ungrouped host keeps exactly the version 1 shape.
        #expect(entry["group"] == nil)
    }

    @Test
    func groupsRoundTripAndHostsReferenceThemByID() throws {
        let group = HostGroup(name: "Production")
        var grouped = host
        grouped.groupID = group.id
        let configuration = HostConfiguration(groups: [group], hosts: [grouped])
        let data = try configuration.encoded()
        let object = try #require(try JSONSerialization.jsonObject(with: data) as? [String: Any])
        let entry = try #require((object["hosts"] as? [[String: Any]])?.first)
        #expect(entry["group"] as? String == group.id.uuidString)
        #expect((object["groups"] as? [[String: Any]])?.first?["name"] as? String == "Production")

        let decoded = try HostConfiguration(data: data)
        #expect(decoded.groups == [group])
        #expect(decoded.hosts == [grouped])
    }

    @Test
    func versionOneFilesStillLoadWithoutGroups() throws {
        let data = """
        {"schemaVersion":1,"hosts":[{"id":"\(host.id.uuidString)","name":"Web","hostname":"example.com","port":22,"username":""}]}
        """.data(using: .utf8)!
        let decoded = try HostConfiguration(data: data)
        #expect(decoded.groups.isEmpty)
        #expect(decoded.hosts.first?.groupID == nil)
    }

    @Test
    func missingOptionalsFallBackToTheirDefaults() throws {
        let data = """
        {"schemaVersion":1,"hosts":[{"id":"\(host.id.uuidString)","name":"Web","hostname":"example.com"}]}
        """.data(using: .utf8)!

        let decoded = try HostConfiguration(data: data)

        #expect(decoded.hosts.count == 1)
        #expect(decoded.hosts[0].port == Host.defaultPort)
        #expect(decoded.hosts[0].username == "")
    }

    @Test
    func emptyListIsValid() throws {
        let decoded = try HostConfiguration(data: HostConfiguration(hosts: []).encoded())
        #expect(decoded.hosts.isEmpty)
    }

    @Test
    func futureSchemaVersionIsRefused() throws {
        let data = #"{"schemaVersion":3,"hosts":[]}"#.data(using: .utf8)!

        #expect(throws: HostConfiguration.DecodeError.unsupportedVersion(3)) {
            try HostConfiguration(data: data)
        }
    }

    @Test
    func wrongFieldTypeIsRefusedWithAReadableMessage() throws {
        // A valid UUID here, so the field that fails is the port.
        let id = UUID(uuidString: "0F8FAD5B-D9CB-469F-A165-70867728950E")!
        let data = """
        {"schemaVersion":1,"hosts":[{"id":"\(id.uuidString)","name":"Web","hostname":"example.com","port":"22"}]}
        """.data(using: .utf8)!

        do {
            _ = try HostConfiguration(data: data)
            Issue.record("Expected the wrong type to be refused")
        } catch let error as HostConfiguration.DecodeError {
            guard case .malformed(let detail) = error else {
                Issue.record("Expected a malformed error, got \(error)")
                return
            }
            #expect(detail.contains("port"))
        } catch {
            Issue.record("Unexpected error: \(error)")
        }
    }

    @Test
    func invalidJSONIsRefused() {
        #expect(throws: (any Error).self) {
            try HostConfiguration(data: Data("not json".utf8))
        }
    }
}
