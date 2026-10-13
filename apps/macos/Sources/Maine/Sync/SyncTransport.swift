import Foundation

/// One slot's server-side state, as reported by response headers or JSON.
public struct SyncSlotState: Equatable, Sendable {
    public var version: Int
    public var size: Int
    public var sha256: String?
    public var updatedAt: Date?

    public init(version: Int, size: Int = 0, sha256: String? = nil, updatedAt: Date? = nil) {
        self.version = version
        self.size = size
        self.sha256 = sha256
        self.updatedAt = updatedAt
    }
}

public enum SyncFetchResult: Equatable, Sendable {
    /// `304`: the caller's last version is still current.
    case notModified(SyncSlotState)
    /// `204`: the slot holds nothing yet.
    case empty(SyncSlotState)
    /// `200` to a `HEAD`: metadata only.
    case current(SyncSlotState)
    /// `200`: the current blob.
    case blob(Data, SyncSlotState)
}

public enum SyncTransportError: Error, LocalizedError, Equatable, Sendable {
    case unauthorized
    case forbidden(String)
    case notFound(String)
    /// `409` or `412`: the slot is not at the version the client expected.
    case conflict(currentVersion: Int)
    /// `428`: a write to a non-empty slot without a precondition.
    case preconditionRequired(currentVersion: Int)
    case tooLarge
    case offline(String)
    case server(status: Int, message: String)
    case malformed(String)

    public var errorDescription: String? {
        switch self {
        case .unauthorized: return "The server did not accept this sync token."
        case .forbidden(let message): return message
        case .notFound(let message): return message
        case .conflict: return "Another device changed the vault first."
        case .preconditionRequired: return "The sync client sent an incomplete write request."
        case .tooLarge: return "The vault is larger than this server accepts."
        case .offline(let message): return message
        case .server(_, let message): return message
        case .malformed(let message): return message
        }
    }
}

/// The wire contract from `docs/sync-protocol.md`, behind a protocol so the
/// engine can be tested without a network.
public protocol SyncTransport: Sendable {
    func createVault(at server: URL, createSecret: String?, body: Data) async throws -> (token: String, state: SyncSlotState)
    func fetchVault(at server: URL, token: String, ifNoneMatch: Int?) async throws -> SyncFetchResult
    func headVault(at server: URL, token: String, ifNoneMatch: Int?) async throws -> SyncFetchResult
    func putVault(at server: URL, token: String, body: Data, expectedVersion: Int?) async throws -> SyncSlotState
    func rotateToken(at server: URL, token: String) async throws -> String
    func deleteVault(at server: URL, token: String) async throws
}

public struct URLSessionSyncTransport: SyncTransport {
    private let session: URLSession

    public init(requestTimeout: TimeInterval = 10) {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = requestTimeout
        configuration.timeoutIntervalForResource = requestTimeout * 3
        configuration.httpCookieStorage = nil
        configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
        session = URLSession(configuration: configuration)
    }

    // MARK: - Endpoints

    public func createVault(at server: URL, createSecret: String?, body: Data) async throws -> (token: String, state: SyncSlotState) {
        var request = makeRequest("POST", server, path: "v1/vaults", token: nil)
        request.setValue("application/octet-stream", forHTTPHeaderField: "Content-Type")
        if let createSecret, !createSecret.isEmpty {
            request.setValue(createSecret, forHTTPHeaderField: "X-Maine-Create-Secret")
        }
        request.httpBody = body
        let (data, response) = try await send(request)
        try expect(response, data, statuses: [201])
        let created: CreatedResponse = try decode(data)
        return (created.token, SyncSlotState(version: created.version, size: created.size, sha256: created.sha256))
    }

    public func fetchVault(at server: URL, token: String, ifNoneMatch: Int?) async throws -> SyncFetchResult {
        try await fetch("GET", at: server, token: token, ifNoneMatch: ifNoneMatch)
    }

    public func headVault(at server: URL, token: String, ifNoneMatch: Int?) async throws -> SyncFetchResult {
        try await fetch("HEAD", at: server, token: token, ifNoneMatch: ifNoneMatch)
    }

    public func putVault(at server: URL, token: String, body: Data, expectedVersion: Int?) async throws -> SyncSlotState {
        var request = makeRequest("PUT", server, path: "v1/vault", token: token)
        request.setValue("application/octet-stream", forHTTPHeaderField: "Content-Type")
        if let expectedVersion {
            request.setValue("\"v\(expectedVersion)\"", forHTTPHeaderField: "If-Match")
        } else {
            request.setValue("*", forHTTPHeaderField: "If-None-Match")
        }
        request.httpBody = body
        let (data, response) = try await send(request)
        if response.statusCode == 200 {
            let stored: StoredResponse = try decode(data)
            return SyncSlotState(version: stored.version, size: stored.size, sha256: stored.sha256)
        }
        try expect(response, data, statuses: [])
        throw SyncTransportError.malformed("unreachable")
    }

    public func rotateToken(at server: URL, token: String) async throws -> String {
        let (data, response) = try await send(makeRequest("POST", server, path: "v1/vault/rotate", token: token))
        try expect(response, data, statuses: [200])
        return try decode(data, as: TokenResponse.self).token
    }

    public func deleteVault(at server: URL, token: String) async throws {
        let (data, response) = try await send(makeRequest("DELETE", server, path: "v1/vault", token: token))
        try expect(response, data, statuses: [204, 200])
    }

    // MARK: - Plumbing

    private func fetch(_ method: String, at server: URL, token: String, ifNoneMatch: Int?) async throws -> SyncFetchResult {
        var request = makeRequest(method, server, path: "v1/vault", token: token)
        if let ifNoneMatch, ifNoneMatch > 0 {
            request.setValue("\"v\(ifNoneMatch)\"", forHTTPHeaderField: "If-None-Match")
        }
        let (data, response) = try await send(request)
        switch response.statusCode {
        case 304:
            return .notModified(state(from: response))
        case 204:
            return .empty(state(from: response))
        case 200:
            let state = state(from: response)
            return method == "HEAD" ? .current(state) : .blob(data, state)
        default:
            try expect(response, data, statuses: [])
            throw SyncTransportError.malformed("unreachable")
        }
    }

    private func makeRequest(_ method: String, _ server: URL, path: String, token: String?) -> URLRequest {
        var url = server
        for piece in path.split(separator: "/") {
            url = url.appendingPathComponent(String(piece))
        }
        var request = URLRequest(url: url)
        request.httpMethod = method
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        if let token, !token.isEmpty {
            request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        }
        return request
    }

    private func send(_ request: URLRequest) async throws -> (Data, HTTPURLResponse) {
        do {
            let (data, response) = try await session.data(for: request)
            guard let http = response as? HTTPURLResponse else {
                throw SyncTransportError.malformed("The server sent no HTTP response.")
            }
            return (data, http)
        } catch let error as URLError {
            if error.code == .cancelled { throw CancellationError() }
            throw SyncTransportError.offline(Self.offlineMessage(error))
        }
    }

    private func expect(_ response: HTTPURLResponse, _ data: Data, statuses: Set<Int>) throws {
        if statuses.contains(response.statusCode) { return }
        let message = (try? JSONDecoder().decode(ErrorBody.self, from: data))?.error
            ?? "The server answered with status \(response.statusCode)."
        switch response.statusCode {
        case 401: throw SyncTransportError.unauthorized
        case 403: throw SyncTransportError.forbidden(message)
        case 404: throw SyncTransportError.notFound(message)
        case 409, 412: throw SyncTransportError.conflict(currentVersion: version(from: response) ?? 0)
        case 413: throw SyncTransportError.tooLarge
        case 428: throw SyncTransportError.preconditionRequired(currentVersion: version(from: response) ?? 0)
        default: throw SyncTransportError.server(status: response.statusCode, message: message)
        }
    }

    private func version(from response: HTTPURLResponse) -> Int? {
        response.value(forHTTPHeaderField: "X-Maine-Version").flatMap(Int.init)
    }

    private func state(from response: HTTPURLResponse) -> SyncSlotState {
        let updated = response.value(forHTTPHeaderField: "X-Maine-Updated-At").flatMap(Double.init)
        return SyncSlotState(
            version: version(from: response) ?? 0,
            size: response.value(forHTTPHeaderField: "X-Maine-Size").flatMap(Int.init) ?? 0,
            sha256: response.value(forHTTPHeaderField: "X-Maine-SHA256"),
            updatedAt: updated.map { Date(timeIntervalSince1970: $0) }
        )
    }

    private func decode<T: Decodable>(_ data: Data, as type: T.Type = T.self) throws -> T {
        do {
            return try JSONDecoder().decode(T.self, from: data)
        } catch {
            throw SyncTransportError.malformed("The server sent an unexpected response.")
        }
    }

    private static func offlineMessage(_ error: URLError) -> String {
        switch error.code {
        case .notConnectedToInternet, .networkConnectionLost:
            return "No network connection."
        case .cannotConnectToHost, .cannotFindHost, .dnsLookupFailed:
            return "The sync server could not be reached."
        case .timedOut:
            return "The sync server took too long to answer."
        default:
            return error.localizedDescription
        }
    }
}

private struct CreatedResponse: Decodable {
    let token: String
    let version: Int
    let size: Int
    let sha256: String?
}

private struct StoredResponse: Decodable {
    let version: Int
    let size: Int
    let sha256: String?
}

private struct TokenResponse: Decodable {
    let token: String
}

private struct ErrorBody: Decodable {
    let error: String
}
