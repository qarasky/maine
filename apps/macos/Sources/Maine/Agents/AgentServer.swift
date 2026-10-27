import Darwin
import Foundation

/// Answers tool calls for an authenticated client. Implemented by
/// `AgentToolHandler` in the app; tests use small fakes.
protocol AgentCallHandling: Sendable {
    func handle(_ call: AgentCall, client: AgentClient) async -> AgentReply
}

/// The app side of the agent socket: a Unix stream socket that only this
/// user can open (directory 0700, socket 0600, peer uid checked). Every
/// connection must present a paired client's token in its first frame.
final class AgentServer: @unchecked Sendable {
    static let notPaired = "This MCP client is not paired with Maine."

    static var defaultSocketURL: URL {
        PasswordStore.defaultFileURL.deletingLastPathComponent().appendingPathComponent("agent.sock")
    }

    private let socketURL: URL
    private let clients: AgentClientStore
    private let handler: AgentCallHandling
    private let queue = DispatchQueue(label: "app.maine.agent-server")
    private let lock = NSLock()
    private var listener: DispatchSourceRead?
    private var connections: [ObjectIdentifier: Connection] = [:]

    init(socketURL: URL = AgentServer.defaultSocketURL, clients: AgentClientStore, handler: AgentCallHandling) {
        self.socketURL = socketURL
        self.clients = clients
        self.handler = handler
    }

    var isRunning: Bool {
        lock.lock(); defer { lock.unlock() }
        return listener != nil
    }

    func start() throws {
        guard !isRunning else { return }
        let path = socketURL.path
        let directory = socketURL.deletingLastPathComponent()
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: directory.path)
        if FileManager.default.fileExists(atPath: path) {
            if let live = AgentSocket.connect(path) {
                Darwin.close(live)
                throw HostStoreError("Another copy of Maine is already serving agents.")
            }
            try FileManager.default.removeItem(atPath: path)
        }
        guard var address = AgentSocket.address(path) else { throw HostStoreError("The agent socket path is too long.") }
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { throw HostStoreError("Could not create the agent socket.") }
        let bound = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                bind(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard bound == 0, chmod(path, 0o600) == 0, listen(fd, 16) == 0 else {
            Darwin.close(fd)
            throw HostStoreError("Could not listen on the agent socket.")
        }
        _ = fcntl(fd, F_SETFL, fcntl(fd, F_GETFL) | O_NONBLOCK)
        let source = DispatchSource.makeReadSource(fileDescriptor: fd, queue: queue)
        source.setEventHandler { [weak self] in self?.accept(from: fd) }
        source.setCancelHandler { Darwin.close(fd) }
        lock.lock()
        listener = source
        lock.unlock()
        source.resume()
    }

    func stop() {
        lock.lock()
        let source = listener
        listener = nil
        let open = Array(connections.values)
        connections = [:]
        lock.unlock()
        guard let source else { return }
        source.cancel()
        for connection in open { connection.close() }
        unlink(socketURL.path)
    }

    /// Ends every live connection of a client (revoked, or the master switch).
    func disconnect(clientID: UUID) {
        lock.lock()
        let matching = connections.values.filter { $0.client?.id == clientID }
        lock.unlock()
        for connection in matching { connection.close() }
    }

    // MARK: - Connections

    private func accept(from listenFD: Int32) {
        while true {
            let fd = Darwin.accept(listenFD, nil, nil)
            guard fd >= 0 else { return }
            var uid: uid_t = 0, gid: gid_t = 0
            guard getpeereid(fd, &uid, &gid) == 0, uid == getuid() else { Darwin.close(fd); continue }
            AgentSocket.noSigPipe(fd)
            // Accepted sockets inherit O_NONBLOCK; replies are written blocking.
            _ = fcntl(fd, F_SETFL, fcntl(fd, F_GETFL) & ~O_NONBLOCK)
            let connection = Connection(fd: fd, server: self)
            lock.lock()
            connections[ObjectIdentifier(connection)] = connection
            lock.unlock()
            connection.start(queue: queue)
        }
    }

    fileprivate func removed(_ connection: Connection) {
        lock.lock()
        connections[ObjectIdentifier(connection)] = nil
        lock.unlock()
    }

    fileprivate func authenticate(_ token: String) -> AgentClient? {
        try? clients.client(forToken: token)
    }

    fileprivate func handle(_ call: AgentCall, client: AgentClient) async -> AgentReply {
        await handler.handle(call, client: client)
    }

    fileprivate final class Connection: @unchecked Sendable {
        let fd: Int32
        weak var server: AgentServer?
        private let lock = NSLock()
        private var reader = AgentFrame.Reader()
        private var source: DispatchSourceRead?
        /// Replies are written here, outside `lock` and off the cooperative
        /// pool: a client that stops reading stalls only its own connection.
        private let writeQueue = DispatchQueue(label: "app.maine.agent-server.write")
        private var closed = false
        private(set) var client: AgentClient? {
            get { lock.lock(); defer { lock.unlock() }; return storedClient }
            set { lock.lock(); storedClient = newValue; lock.unlock() }
        }
        private var storedClient: AgentClient?

        init(fd: Int32, server: AgentServer) {
            self.fd = fd
            self.server = server
        }

        func start(queue: DispatchQueue) {
            let source = DispatchSource.makeReadSource(fileDescriptor: fd, queue: queue)
            let fd = fd
            let writes = writeQueue
            source.setEventHandler { [weak self] in self?.readable() }
            // After any write in flight, so the fd can't be reused under it.
            source.setCancelHandler { writes.async { Darwin.close(fd) } }
            self.source = source
            source.resume()
        }

        func close() {
            lock.lock()
            guard !closed else { lock.unlock(); return }
            closed = true
            let source = source
            lock.unlock()
            source?.cancel()
            server?.removed(self)
        }

        private func readable() {
            guard let data = AgentSocket.readAvailable(fd) else { close(); return }
            guard let frames = try? reader.feed(data) else { close(); return }
            for frame in frames { received(frame) }
        }

        private func received(_ frame: Data) {
            guard let client else {
                guard let hello = try? JSONDecoder().decode(AgentHello.self, from: frame),
                      let client = server?.authenticate(hello.token) else {
                    send(AgentHelloReply(ok: false, error: AgentServer.notPaired), thenClose: true)
                    return
                }
                self.client = client
                send(AgentHelloReply(ok: true, error: nil))
                return
            }
            guard let call = try? JSONDecoder().decode(AgentCall.self, from: frame) else { close(); return }
            Task { [weak self] in
                guard let server = self?.server else { return }
                let reply = await server.handle(call, client: client)
                self?.send(reply)
            }
        }

        private var isClosed: Bool { lock.withLock { closed } }

        private func send<T: Encodable>(_ value: T, thenClose: Bool = false) {
            guard let frame = try? AgentFrame.encode(value) else { return }
            let fd = fd
            writeQueue.async { [weak self] in
                guard let self, !self.isClosed else { return }
                if !AgentSocket.writeAll(fd, frame) || thenClose { self.close() }
            }
        }
    }
}
