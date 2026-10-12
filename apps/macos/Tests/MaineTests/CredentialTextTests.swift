import Foundation
@testable import Maine
import Testing

@Suite
struct CredentialTextTests {
    @Test
    func everyFieldBecomesOneBracketedLine() {
        let host = Host(name: "Web", hostname: "10.0.0.4", port: 2222, username: "root")
        #expect(CredentialText.make(host: host, password: "abcdef123") == """
        [ip] 10.0.0.4
        [port] 2222
        [username] root
        [password] abcdef123
        """)
    }

    @Test
    func theDefaultPortAndMissingFieldsAreOmitted() {
        let host = Host(name: "Web", hostname: "example.com")
        #expect(CredentialText.make(host: host, password: nil) == "[ip] example.com")
        #expect(CredentialText.make(host: host, password: "") == "[ip] example.com")
    }

    @Test
    func theUsernameSurvivesWithoutASavedPassword() {
        let host = Host(name: "Web", hostname: "example.com", username: "deploy")
        #expect(CredentialText.make(host: host, password: nil) == """
        [ip] example.com
        [username] deploy
        """)
    }
}
