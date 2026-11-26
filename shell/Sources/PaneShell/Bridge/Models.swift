import CoreGraphics
import Foundation

// Mirrors of the JSON types in crates/pane-core (snapshot.rs, profile.rs,
// tree.rs). Field names follow the Rust (snake_case) wire format.

struct PRect: Codable, Equatable {
    var x: Double
    var y: Double
    var w: Double
    var h: Double

    var cg: CGRect { CGRect(x: x, y: y, width: w, height: h) }
}

struct SnapTile: Codable, Equatable {
    var wid: UInt32?
    var bundle_id: String
    var title: String
    var rect: PRect
    var focused: Bool
    var floating: Bool
    var pending: Bool
}

struct SnapDisplay: Codable, Equatable {
    var id: String
    var name: String
    var frame: PRect
    var visible_frame: PRect
    var is_main: Bool
    var rank: Int
    var visible_workspace: String?
    var focused: Bool
    var tiles: [SnapTile]
}

struct SnapWorkspace: Codable, Equatable {
    var name: String
    var display: String
    var visible: Bool
    var windows: Int
}

struct Snapshot: Codable, Equatable {
    var displays: [SnapDisplay]
    var workspaces: [SnapWorkspace]
    var focused_display: String?
    var focused_window: UInt32?
    var config_error: String?
    var profiles: [String]
    var ax_trusted: Bool
}

struct AppSlot: Codable, Equatable, Hashable {
    var bundle_id: String
    var title: String?
    var launch_args: [String]?
}

/// `tree::SerNode`, tagged by "type".
indirect enum SerNode: Codable, Equatable {
    case container(orientation: String, layout: String, weights: [Double], children: [SerNode])
    case window(id: UInt32)
    case slot(AppSlot)

    static var empty: SerNode { .container(orientation: "horizontal", layout: "tiles", weights: [], children: []) }

    private enum K: String, CodingKey {
        case type, orientation, layout, weights, children, id, bundle_id, title, launch_args
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: K.self)
        switch try c.decode(String.self, forKey: .type) {
        case "container":
            self = .container(
                orientation: try c.decodeIfPresent(String.self, forKey: .orientation) ?? "horizontal",
                layout: try c.decodeIfPresent(String.self, forKey: .layout) ?? "tiles",
                weights: try c.decodeIfPresent([Double].self, forKey: .weights) ?? [],
                children: try c.decodeIfPresent([SerNode].self, forKey: .children) ?? [])
        case "window":
            self = .window(id: try c.decode(UInt32.self, forKey: .id))
        default:
            self = .slot(AppSlot(
                bundle_id: try c.decode(String.self, forKey: .bundle_id),
                title: try c.decodeIfPresent(String.self, forKey: .title),
                launch_args: try c.decodeIfPresent([String].self, forKey: .launch_args)))
        }
    }

    func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: K.self)
        switch self {
        case let .container(o, l, w, ch):
            try c.encode("container", forKey: .type)
            try c.encode(o, forKey: .orientation)
            try c.encode(l, forKey: .layout)
            try c.encode(w, forKey: .weights)
            try c.encode(ch, forKey: .children)
        case let .window(id):
            try c.encode("window", forKey: .type)
            try c.encode(id, forKey: .id)
        case let .slot(s):
            try c.encode("slot", forKey: .type)
            try c.encode(s.bundle_id, forKey: .bundle_id)
            try c.encodeIfPresent(s.title, forKey: .title)
            if let a = s.launch_args, !a.isEmpty { try c.encode(a, forKey: .launch_args) }
        }
    }

    var children: [SerNode] {
        if case let .container(_, _, _, ch) = self { return ch }
        return []
    }

    func node(at path: [Int]) -> SerNode? {
        var cur = self
        for i in path {
            let ch = cur.children
            guard i < ch.count else { return nil }
            cur = ch[i]
        }
        return cur
    }
}

struct ProfileDisplay: Codable, Equatable {
    var display: String
    var workspace: String
    var tree: SerNode
}

struct Profile: Codable, Equatable {
    var name: String
    var version: Int = 1
    var display: [ProfileDisplay]
}

enum Reply {
    case ok(Any?)
    case error(String)
}
