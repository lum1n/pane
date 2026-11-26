import CPane
import Foundation

/// Swift face of the Rust core (crates/pane-ffi).
@MainActor
final class PaneCore {
    static let shared = PaneCore()

    /// Latest snapshot, delivered on the main thread (coalesced to 50 ms).
    var onSnapshot: ((Snapshot) -> Void)?
    /// `UiRequest` objects from the core, on the main thread.
    var onRequest: (([String: Any]) -> Void)?
    private(set) var snapshot: Snapshot?

    private var pendingSnapshot: Data?
    private var snapshotScheduled = false

    func start(supportDir: String? = nil) -> String? {
        let ctx = Unmanaged.passUnretained(self).toOpaque()
        let snap: PaneCallback = { ctx, json in
            guard let ctx, let json else { return }
            let data = Data(bytes: json, count: strlen(json))
            let me = Unmanaged<PaneCore>.fromOpaque(ctx).takeUnretainedValue()
            DispatchQueue.main.async { MainActor.assumeIsolated { me.receiveSnapshot(data) } }
        }
        let req: PaneCallback = { ctx, json in
            guard let ctx, let json else { return }
            let data = Data(bytes: json, count: strlen(json))
            let me = Unmanaged<PaneCore>.fromOpaque(ctx).takeUnretainedValue()
            DispatchQueue.main.async { MainActor.assumeIsolated { me.receiveRequest(data) } }
        }
        let ok: Bool
        if let dir = supportDir {
            ok = dir.withCString { pane_start($0, snap, req, ctx) }
        } else {
            ok = pane_start(nil, snap, req, ctx)
        }
        if ok { return nil }
        guard let e = pane_last_error() else { return "Pane failed to start" }
        defer { pane_free(e) }
        return String(cString: e)
    }

    private func receiveSnapshot(_ data: Data) {
        pendingSnapshot = data
        guard !snapshotScheduled else { return }
        snapshotScheduled = true
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) { [weak self] in
            MainActor.assumeIsolated {
                guard let self else { return }
                self.snapshotScheduled = false
                guard let d = self.pendingSnapshot,
                      let s = try? JSONDecoder().decode(Snapshot.self, from: d) else { return }
                self.snapshot = s
                self.onSnapshot?(s)
            }
        }
    }

    private func receiveRequest(_ data: Data) {
        guard let obj = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else { return }
        onRequest?(obj)
    }

    // MARK: commands

    @discardableResult
    func send(_ cmd: [String: Any]) -> Reply {
        guard let data = try? JSONSerialization.data(withJSONObject: cmd),
              let json = String(data: data, encoding: .utf8) else { return .error("encode failed") }
        guard let out = json.withCString({ pane_command($0) }) else { return .error("no reply") }
        defer { pane_free(out) }
        let text = String(cString: out)
        guard let obj = (try? JSONSerialization.jsonObject(with: Data(text.utf8))) as? [String: Any] else {
            return .error("bad reply")
        }
        if (obj["status"] as? String) == "ok" { return .ok(obj["data"]) }
        return .error(obj["message"] as? String ?? "error")
    }

    /// Run a command whose reply data decodes to `T`.
    func request<T: Decodable>(_ cmd: [String: Any], as: T.Type) -> Result<T, PaneError> {
        switch send(cmd) {
        case let .error(m): return .failure(PaneError(message: m))
        case let .ok(data):
            guard let data, JSONSerialization.isValidJSONObject(data) || data is String,
                  let d = try? JSONSerialization.data(withJSONObject: data, options: [.fragmentsAllowed]),
                  let v = try? JSONDecoder().decode(T.self, from: d) else {
                return .failure(PaneError(message: "unexpected reply"))
            }
            return .success(v)
        }
    }

    func event(_ ev: [String: Any]) {
        guard let data = try? JSONSerialization.data(withJSONObject: ev),
              let json = String(data: data, encoding: .utf8) else { return }
        _ = json.withCString { pane_event($0) }
    }

    func shutdown() {
        _ = pane_shutdown()
    }

    static func json(_ value: some Encodable) -> Any? {
        guard let d = try? JSONEncoder().encode(value) else { return nil }
        return try? JSONSerialization.jsonObject(with: d)
    }

    // Convenience wrappers.
    func showWorkspace(_ name: String) { send(["cmd": "workspace", "name": name]) }
    func reload() -> Reply { send(["cmd": "reload"]) }
    func recover() { send(["cmd": "recover", "all_apps": false]) }
    func applyProfile(named name: String) -> Reply { send(["cmd": "profile-apply", "name": name]) }

    func capture(name: String) -> Result<Profile, PaneError> {
        request(["cmd": "profile-capture", "name": name], as: Profile.self)
    }

    func save(_ p: Profile, startup: Bool = false) -> Reply {
        guard let pj = PaneCore.json(p) else { return .error("encode failed") }
        return send(["cmd": "profile-save", "profile": pj, "startup": startup])
    }

    func apply(_ p: Profile) -> Reply {
        guard let pj = PaneCore.json(p) else { return .error("encode failed") }
        return send(["cmd": "profile-apply", "profile": pj])
    }

    func load(profile name: String) -> Result<Profile, PaneError> {
        request(["cmd": "profile-load", "name": name], as: Profile.self)
    }

    func edit(_ p: Profile, _ op: [String: Any]) -> Result<Profile, PaneError> {
        guard let pj = PaneCore.json(p) else { return .failure(PaneError(message: "encode failed")) }
        return request(["cmd": "profile-edit", "profile": pj, "edit": op], as: Profile.self)
    }
}

struct PaneError: Error, CustomStringConvertible {
    var message: String
    var description: String { message }
}
