import CoreGraphics

/// Drag payloads are plain strings: `app:<bundle id>` from the palette and
/// `tile:<display>:<i,j,...>` for existing tiles.
enum DragPayload {
    static func app(bundle: String) -> String { "app:\(bundle)" }

    static func tile(_ t: TileRef) -> String {
        "tile:\(t.display):\(t.path.map(String.init).joined(separator: ","))"
    }

    static func app(_ s: String) -> String? {
        s.hasPrefix("app:") ? String(s.dropFirst(4)) : nil
    }

    static func tile(_ s: String) -> TileRef? {
        guard s.hasPrefix("tile:") else { return nil }
        let parts = s.dropFirst(5).split(separator: ":", omittingEmptySubsequences: false)
        guard parts.count == 2, let d = Int(parts[0]) else { return nil }
        let path = parts[1].split(separator: ",").compactMap { Int($0) }
        return TileRef(display: d, path: path)
    }
}

/// i3-style drop zones: the middle 40% replaces/swaps, the rest splits
/// towards the nearest edge.
func dropZone(at p: CGPoint, in size: CGSize) -> String {
    guard size.width > 0, size.height > 0 else { return "center" }
    let nx = p.x / size.width
    let ny = p.y / size.height
    if (0.3...0.7).contains(nx) && (0.3...0.7).contains(ny) { return "center" }
    let d: [(String, CGFloat)] = [("left", nx), ("right", 1 - nx), ("up", ny), ("down", 1 - ny)]
    return d.min { $0.1 < $1.1 }!.0
}
