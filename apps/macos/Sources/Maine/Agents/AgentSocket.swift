import Darwin
import Foundation

/// POSIX helpers shared by the agent server (in the app) and the socket
/// client (in `Maine --mcp`).
enum AgentSocket {
    /// A `sockaddr_un` for `path`, or nil when the path does not fit.
    static func address(_ path: String) -> sockaddr_un? {
        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        let bytes = Array(path.utf8)
        let capacity = MemoryLayout.size(ofValue: address.sun_path)
        guard bytes.count < capacity else { return nil }
        withUnsafeMutableBytes(of: &address.sun_path) { buffer in
            buffer.copyBytes(from: bytes)
            buffer[bytes.count] = 0
        }
        address.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
        return address
    }

    /// A connected stream socket, or nil (nothing is listening there).
    static func connect(_ path: String) -> Int32? {
        guard var address = address(path) else { return nil }
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { return nil }
        noSigPipe(fd)
        let result = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard result == 0 else { Darwin.close(fd); return nil }
        return fd
    }

    static func noSigPipe(_ fd: Int32) {
        var on: Int32 = 1
        setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &on, socklen_t(MemoryLayout<Int32>.size))
    }

    /// Writes every byte, retrying short writes. False on error.
    static func writeAll(_ fd: Int32, _ data: Data) -> Bool {
        data.withUnsafeBytes { buffer in
            guard var pointer = buffer.baseAddress else { return true }
            var remaining = buffer.count
            while remaining > 0 {
                let written = Darwin.write(fd, pointer, remaining)
                if written < 0 {
                    if errno == EINTR { continue }
                    return false
                }
                remaining -= written
                pointer += written
            }
            return true
        }
    }

    /// Reads what is available; nil on EOF or error.
    static func readAvailable(_ fd: Int32) -> Data? {
        var buffer = [UInt8](repeating: 0, count: 65_536)
        while true {
            let count = Darwin.read(fd, &buffer, buffer.count)
            if count > 0 { return Data(buffer[0..<count]) }
            if count < 0, errno == EINTR { continue }
            if count < 0, errno == EAGAIN { return Data() }
            return nil
        }
    }
}

/// The bridge's connection to the app: hello first, then calls multiplexed
/// by id so parallel tool calls can share one socket.
final class AgentSocketClient: @unchecked Sendable {
    enum ClientError: Error, Equatable {
        /// Nothing is listening.
        case notRunning
        /// The connection was already closed; the request was never sent.
        case notSent
        /// The connection closed after the request was sent: it may have run.
        case disconnected
    }

    private let fd: Int32
    private let lock = NSLock()
    private var reader = AgentFrame.Reader()
    private var pending: [Int: CheckedContinuation<AgentReply, Error>] = [:]
    private var helloWaiter: CheckedContinuation<AgentHelloReply, Error>?
    private var nextID = 1
    private var closed = false
    private let source: DispatchSourceRead
    /// Writes happen here, never under `lock` and never on the cooperative
    /// pool, so a slow peer can't stall reads or other tasks.
    private let writeQueue = DispatchQueue(label: "app.maine.agent-client.write")

    var isClosed: Bool { lock.withLock { closed } }

    init(socketURL: URL) throws {
        guard let fd = AgentSocket.connect(socketURL.path) else { throw ClientError.notRunning }
        self.fd = fd
        source = DispatchSource.makeReadSource(fileDescriptor: fd, queue: DispatchQueue(label: "app.maine.agent-client"))
        source.setEventHandler { [weak self] in self?.readable() }
        // Close on the write queue, after any write in flight, so the fd
        // number can't be reused while a queued write still targets it.
        let writes = writeQueue
        source.setCancelHandler { writes.async { Darwin.close(fd) } }
        source.resume()
    }

    deinit { close() }

    func hello(_ hello: AgentHello) async throws -> AgentHelloReply {
        let frame = try AgentFrame.encode(hello)
        return try await withCheckedThrowingContinuation { continuation in
            lock.lock()
            guard !closed else { lock.unlock(); continuation.resume(throwing: ClientError.notSent); return }
            helloWaiter = continuation
            lock.unlock()
            write(frame)
        }
    }

    func call(tool: String, arguments: JSONValue) async throws -> AgentReply {
        try await withCheckedThrowingContinuation { continuation in
            lock.lock()
            guard !closed else { lock.unlock(); continuation.resume(throwing: ClientError.notSent); return }
            let id = nextID
            nextID += 1
            guard let frame = try? AgentFrame.encode(AgentCall(id: id, tool: tool, arguments: arguments)) else {
                lock.unlock()
                continuation.resume(throwing: ClientError.notSent)
                return
            }
            pending[id] = continuation
            lock.unlock()
            write(frame)
        }
    }

    func close() {
        lock.lock()
        let wasOpen = !closed
        lock.unlock()
        if wasOpen { fail() }
    }

    private func write(_ frame: Data) {
        let fd = fd
        writeQueue.async { [weak self] in
            guard let self, !self.isClosed else { return }
            if !AgentSocket.writeAll(fd, frame) { self.fail() }
        }
    }

    private func readable() {
        guard let data = AgentSocket.readAvailable(fd) else { fail(); return }
        lock.lock()
        let frames = (try? reader.feed(data)) ?? nil
        lock.unlock()
        guard let frames else { fail(); return }
        for frame in frames { deliver(frame) }
    }

    private func deliver(_ frame: Data) {
        lock.lock()
        if let waiter = helloWaiter, let reply = try? JSONDecoder().decode(AgentHelloReply.self, from: frame) {
            helloWaiter = nil
            lock.unlock()
            waiter.resume(returning: reply)
            return
        }
        guard let reply = try? JSONDecoder().decode(AgentReply.self, from: frame),
              let waiter = pending.removeValue(forKey: reply.id) else { lock.unlock(); return }
        lock.unlock()
        waiter.resume(returning: reply)
    }

    private func fail() {
        lock.lock()
        guard !closed else { lock.unlock(); return }
        closed = true
        let waiters = Array(pending.values)
        pending = [:]
        let hello = helloWaiter
        helloWaiter = nil
        lock.unlock()
        source.cancel()
        hello?.resume(throwing: ClientError.disconnected)
        for waiter in waiters { waiter.resume(throwing: ClientError.disconnected) }
    }
}
