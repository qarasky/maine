import Foundation
@testable import Maine
import Testing

@Suite
struct CredentialTextTests {
    @Test
    func everyFieldBecomesOneLabelledLine() {
        let host = Host(name: "Web", hostname: "10.0.0.4", port: 2222, username: "root")
        #expect(CredentialText.make(host: host, password: "abcdef123") == """
        IP: 10.0.0.4
        port: 2222
        username: root
        password: abcdef123
        """)
    }

    @Test
    func theDefaultPortAndMissingFieldsAreOmitted() {
        let host = Host(name: "Web", hostname: "example.com")
        #expect(CredentialText.make(host: host, password: nil) == "IP: example.com")
        #expect(CredentialText.make(host: host, password: "") == "IP: example.com")
    }

    @Test
    func theUsernameSurvivesWithoutASavedPassword() {
        let host = Host(name: "Web", hostname: "example.com", username: "deploy")
        #expect(CredentialText.make(host: host, password: nil) == """
        IP: example.com
        username: deploy
        """)
    }
}
