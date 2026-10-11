import Foundation
import Maine
import Testing

@Suite
struct SSHConfigFileTests {
    init() {
        try? FileManager.default.createDirectory(
            at: hostConfigDirectory,
            withIntermediateDirectories: true
        )
    }

    private let hostConfigDirectory = FileManager.default.temporaryDirectory
        .appendingPathComponent("maine-config-tests-\(UUID().uuidString)", isDirectory: true)

    private func url(named name: String) -> URL {
        hostConfigDirectory.appendingPathComponent(name)
    }

    private func write(_ name: String, _ contents: String) throws -> URL {
        let url = url(named: name)
        try contents.write(to: url, atomically: true, encoding: .utf8)
        return url
    }

    @Test
    func missingFileIsNotAnError() {
        let config = SSHConfigFile.load(at: URL(fileURLWithPath: "/nonexistent/.ssh/config"))

        #expect(config.entries.isEmpty)
        #expect(config.entry(for: "anything") == nil)
    }

    @Test
    func readsHostNameUserAndPort() throws {
        let main = try write("config", """
        # my hosts
        Host web
            HostName example.com
            User deploy
            Port 2222

        Host db
            HostName db.example.com
        """)

        let config = SSHConfigFile.load(at: main)

        #expect(config.entries.count == 2)
        let web = try #require(config.entry(for: "web"))
        #expect(web.hostname == "example.com")
        #expect(web.user == "deploy")
        #expect(web.port == 2222)
        #expect(config.entry(for: "db")?.hostname == "db.example.com")
    }

    @Test
    func ignoresDirectivesMaineDoesNotModel() throws {
        let main = try write("config", """
        Host withkeys
            IdentityFile ~/.ssh/id_ed25519
            ProxyJump bastion
        """)

        let entry = try #require(SSHConfigFile.load(at: main).entry(for: "withkeys"))

        // Maine records nothing here on purpose: OpenSSH applies these itself.
        #expect(entry.hostname == nil)
        #expect(entry.user == nil)
        #expect(entry.port == nil)
    }

    @Test
    func ignoresWildcardEntriesForImporting() throws {
        let main = try write("config", """
        Host *.internal example.com
            User me

        Host *
            Compression yes
        """)

        let entries = SSHConfigFile.load(at: main).entries

        // Three patterns on two Host lines, but only one names a single host.
        #expect(entries.count == 3)
        #expect(entries.filter { SSHConfigEntry.isConnectable($0.pattern) }.map(\.pattern) == ["example.com"])
    }

    @Test
    func directivesAreCaseInsensitiveAndCommentsAreDropped() throws {
        let main = try write("config", """
        # leading comment
        host build # trailing comment
        HOSTNAME build.example.com
        UsER builder
        port 22
        """)

        let entry = try #require(SSHConfigFile.load(at: main).entry(for: "build"))

        #expect(entry.hostname == "build.example.com")
        #expect(entry.user == "builder")
        #expect(entry.port == 22)
    }

    @Test
    func keywordEqualsValueIsUnderstood() throws {
        let main = try write("config", """
        Host eq
        HostName=eq.example.com
        Port=2022
        """)

        let entry = try #require(SSHConfigFile.load(at: main).entry(for: "eq"))

        #expect(entry.hostname == "eq.example.com")
        #expect(entry.port == 2022)
    }

    @Test
    func firstValueWinsForARepeatedKeyword() throws {
        let main = try write("config", """
        Host repeat
            User first
            User second
        """)

        #expect(SSHConfigFile.load(at: main).entry(for: "repeat")?.user == "first")
    }

    @Test
    func aLaterBlockCanSupplyAValueAnEarlierOneLeftOut() throws {
        let main = try write("config", """
        Host split
            HostName split.example.com

        Host split
            User later
        """)

        let entry = try #require(SSHConfigFile.load(at: main).entry(for: "split"))

        #expect(entry.hostname == "split.example.com")
        #expect(entry.user == "later")
    }

    @Test
    func includeBringsInOtherFiles() throws {
        try write("included.conf", """
        Host extra
            HostName extra.example.com
        """)
        let main = try write("config", """
        Include included.conf
        Host main
            HostName main.example.com
        """)

        let config = SSHConfigFile.load(at: main)

        #expect(config.entry(for: "main")?.hostname == "main.example.com")
        #expect(config.entry(for: "extra")?.hostname == "extra.example.com")
    }

    @Test
    func realConfigurationFileLoadsWithoutCrashing() {
        // Reading the user's own file must always be safe, whatever it holds.
        _ = SSHConfigFile.load()
    }
}
