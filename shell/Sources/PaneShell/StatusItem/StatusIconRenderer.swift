import AppKit

/// Draws the live desk as an 18 pt template image: one rounded box per
/// display in its real arrangement, the visible workspace's tiles inside
/// (focused tile solid), dots for other workspaces with windows, and a bar
/// under the focused display. A dot top right means the desk no longer
/// matches the active profile.
enum StatusIconRenderer {
    static let height: CGFloat = 18

    static func render(_ snapshot: Snapshot?) -> NSImage {
        guard let s = snapshot, !s.displays.isEmpty else { return placeholder() }
        let displays = s.displays.sorted { $0.rank < $1.rank }
        let union = displays.dropFirst().reduce(displays[0].frame.cg) { $0.union($1.frame.cg) }
        let boxTop: CGFloat = 1
        let boxH: CGFloat = 12
        let maxW = CGFloat(12 * displays.count + 4)
        let scale = min(boxH / max(union.height, 1), maxW / max(union.width, 1))
        let drift = s.profile_drift == true
        let desk = ceil(union.width * scale) + 2
        let width = desk + (drift ? 4 : 0)

        let image = NSImage(size: NSSize(width: width, height: height), flipped: true) { _ in
            NSColor.black.setStroke()
            for d in displays {
                let f = d.frame.cg
                let r = CGRect(
                    x: 1 + (f.minX - union.minX) * scale,
                    y: boxTop + (f.minY - union.minY) * scale + (boxH - union.height * scale) / 2,
                    width: f.width * scale,
                    height: f.height * scale
                ).insetBy(dx: 0.75, dy: 0.5)
                let outline = NSBezierPath(roundedRect: r, xRadius: 1.5, yRadius: 1.5)
                outline.lineWidth = d.focused ? 1.1 : 0.75
                outline.stroke()

                let inner = r.insetBy(dx: 1.5, dy: 1.5)
                for t in d.tiles where !t.pending {
                    let rx = CGFloat(t.rect.x), ry = CGFloat(t.rect.y)
                    let rw = CGFloat(t.rect.w), rh = CGFloat(t.rect.h)
                    let tr = CGRect(
                        x: inner.minX + rx * inner.width,
                        y: inner.minY + ry * inner.height,
                        width: rw * inner.width,
                        height: rh * inner.height
                    ).insetBy(dx: 0.35, dy: 0.35)
                    NSColor.black.withAlphaComponent(t.focused ? 1.0 : 0.45).setFill()
                    NSBezierPath(rect: tr).fill()
                }

                // Dots: hidden workspaces on this display that hold windows.
                let hidden = s.workspaces.filter { $0.display == d.id && !$0.visible && $0.windows > 0 }.count
                let dots = min(hidden, Int(max(1, (r.width - 1) / 2.6)))
                if dots > 0 {
                    NSColor.black.withAlphaComponent(0.8).setFill()
                    let total = CGFloat(dots) * 2.6 - 1.0
                    var x = r.midX - total / 2
                    for _ in 0..<dots {
                        NSBezierPath(ovalIn: CGRect(x: x, y: r.maxY + 1.2, width: 1.6, height: 1.6)).fill()
                        x += 2.6
                    }
                }
                if d.focused {
                    NSColor.black.setFill()
                    NSBezierPath(rect: CGRect(x: r.minX, y: r.maxY + 3.6, width: r.width, height: 1)).fill()
                }
            }
            if drift {
                NSColor.black.setFill()
                NSBezierPath(ovalIn: CGRect(x: desk + 0.4, y: boxTop - 0.5, width: 3.2, height: 3.2)).fill()
            }
            return true
        }
        image.isTemplate = true
        return image
    }

    static func placeholder() -> NSImage {
        let image = NSImage(size: NSSize(width: 16, height: height), flipped: true) { _ in
            NSColor.black.setStroke()
            let r = CGRect(x: 1.5, y: 3.5, width: 13, height: 10)
            let p = NSBezierPath(roundedRect: r, xRadius: 1.5, yRadius: 1.5)
            p.lineWidth = 1
            p.stroke()
            let mid = NSBezierPath()
            mid.move(to: CGPoint(x: r.midX, y: r.minY))
            mid.line(to: CGPoint(x: r.midX, y: r.maxY))
            mid.lineWidth = 1
            mid.stroke()
            return true
        }
        image.isTemplate = true
        return image
    }
}
