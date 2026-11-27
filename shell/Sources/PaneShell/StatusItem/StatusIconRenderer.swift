import AppKit

/// Draws the live desk as an 18 pt image: one rounded box per display in its
/// real arrangement, the visible workspace's tiles inside (focused tile
/// solid, each with a tiny app icon when there is room), dots for other
/// workspaces with windows, a bar under the focused display, and the name of
/// the focused display's workspace from `[workspace_names]` beside it.
///
/// A dot top right means the desk no longer matches the active profile.
///
/// With app icons off the image is a template, so the menu bar tints it.
/// With them on it is drawn in full colour for `dark` or light menu bars.
enum StatusIconRenderer {
    static let height: CGFloat = 18

    /// Where everything goes, in image points (origin top left). Shared by
    /// drawing and `tile(at:)`, so a click hits what was drawn.
    struct Layout {
        struct Box { var display: SnapDisplay; var rect: CGRect }
        struct Tile { var tile: SnapTile; var rect: CGRect }
        var size: NSSize
        /// Width of the boxes alone; a drift dot and the name come after it.
        var deskW: CGFloat = 0
        var boxes: [Box] = []
        /// In drawing order: later tiles are on top.
        var tiles: [Tile] = []
        var name: NSAttributedString?
        var nameOrigin: CGPoint = .zero
    }

    private static let boxTop: CGFloat = 1
    private static let boxH: CGFloat = 12
    private static let nameGap: CGFloat = 4
    private static let nameMaxChars = 16
    private static var icons: [String: NSImage?] = [:]

    static func layout(_ s: Snapshot) -> Layout? {
        guard !s.displays.isEmpty else { return nil }
        let displays = s.displays.sorted { $0.rank < $1.rank }
        let union = displays.dropFirst().reduce(displays[0].frame.cg) { $0.union($1.frame.cg) }
        let maxW = CGFloat(12 * displays.count + 4)
        let scale = min(boxH / max(union.height, 1), maxW / max(union.width, 1))
        let drift = s.profile_drift == true
        let deskW = ceil(union.width * scale) + 2
        let width = deskW + (drift ? 4 : 0)

        var l = Layout(size: NSSize(width: width, height: height))
        l.deskW = deskW
        for d in displays {
            let f = d.frame.cg
            let r = CGRect(
                x: 1 + (f.minX - union.minX) * scale,
                y: boxTop + (f.minY - union.minY) * scale + (boxH - union.height * scale) / 2,
                width: f.width * scale,
                height: f.height * scale
            ).insetBy(dx: 0.75, dy: 0.5)
            l.boxes.append(.init(display: d, rect: r))
            let inner = r.insetBy(dx: 1.5, dy: 1.5)
            for t in d.tiles where !t.pending {
                let tr = CGRect(
                    x: inner.minX + CGFloat(t.rect.x) * inner.width,
                    y: inner.minY + CGFloat(t.rect.y) * inner.height,
                    width: CGFloat(t.rect.w) * inner.width,
                    height: CGFloat(t.rect.h) * inner.height
                ).insetBy(dx: 0.35, dy: 0.35)
                l.tiles.append(.init(tile: t, rect: tr))
            }
        }

        let focused = displays.first(where: { $0.focused }) ?? displays.first(where: { $0.id == s.focused_display })
        if let title = focused?.visible_workspace_title, !title.isEmpty {
            let short = title.count > nameMaxChars ? String(title.prefix(nameMaxChars - 1)) + "…" : title
            let text = NSAttributedString(string: short, attributes: [
                .font: NSFont.systemFont(ofSize: 11, weight: .medium),
            ])
            let ts = text.size()
            l.name = text
            l.nameOrigin = CGPoint(x: width + nameGap, y: (height - ts.height) / 2)
            l.size.width = ceil(width + nameGap + ts.width + 1)
        }
        return l
    }

    /// The window tile under `point` (image points, origin top left).
    static func tile(at point: CGPoint, in s: Snapshot) -> SnapTile? {
        layout(s)?.tiles.last(where: { $0.tile.wid != nil && $0.rect.insetBy(dx: -0.5, dy: -0.5).contains(point) })?.tile
    }

    static func render(_ snapshot: Snapshot?, dark: Bool = false) -> NSImage {
        guard let s = snapshot, let l = layout(s) else { return placeholder() }
        let appIcons = s.appearance?.app_icons ?? false
        let drift = s.profile_drift == true
        // A template is tinted by the menu bar; black is the template colour.
        let ink: NSColor = appIcons && dark ? .white : .black
        // Looked up here, not while drawing.
        let tileIcons = l.tiles.map { appIcons ? icon(for: $0.tile.bundle_id, fitting: $0.rect) : nil }

        let image = NSImage(size: l.size, flipped: true) { _ in
            ink.setStroke()
            for b in l.boxes {
                let outline = NSBezierPath(roundedRect: b.rect, xRadius: 1.5, yRadius: 1.5)
                outline.lineWidth = b.display.focused ? 1.1 : 0.75
                outline.stroke()
            }

            for (t, icon) in zip(l.tiles, tileIcons) {
                if case let (img, rect)? = icon {
                    ink.withAlphaComponent(t.tile.focused ? 0.55 : 0.22).setFill()
                    NSBezierPath(rect: t.rect).fill()
                    img.draw(in: rect, from: .zero, operation: .sourceOver,
                             fraction: t.tile.focused ? 1 : 0.7, respectFlipped: true, hints: nil)
                    if t.tile.focused {
                        ink.setStroke()
                        let p = NSBezierPath(rect: t.rect.insetBy(dx: 0.3, dy: 0.3))
                        p.lineWidth = 0.6
                        p.stroke()
                    }
                } else {
                    ink.withAlphaComponent(t.tile.focused ? 1.0 : 0.45).setFill()
                    NSBezierPath(rect: t.rect).fill()
                }
            }

            for b in l.boxes {
                let r = b.rect
                // Dots: hidden workspaces on this display that hold windows.
                let hidden = s.workspaces.filter { $0.display == b.display.id && !$0.visible && $0.windows > 0 }.count
                let dots = min(hidden, Int(max(1, (r.width - 1) / 2.6)))
                if dots > 0 {
                    ink.withAlphaComponent(0.8).setFill()
                    let total = CGFloat(dots) * 2.6 - 1.0
                    var x = r.midX - total / 2
                    for _ in 0..<dots {
                        NSBezierPath(ovalIn: CGRect(x: x, y: r.maxY + 1.2, width: 1.6, height: 1.6)).fill()
                        x += 2.6
                    }
                }
                if b.display.focused {
                    ink.setFill()
                    NSBezierPath(rect: CGRect(x: r.minX, y: r.maxY + 3.6, width: r.width, height: 1)).fill()
                }
            }
            if drift {
                ink.setFill()
                NSBezierPath(ovalIn: CGRect(x: l.deskW + 0.4, y: boxTop - 0.5, width: 3.2, height: 3.2)).fill()
            }

            if let name = l.name {
                let text = NSMutableAttributedString(attributedString: name)
                text.addAttribute(.foregroundColor, value: ink, range: NSRange(location: 0, length: text.length))
                text.draw(at: l.nameOrigin)
            }
            return true
        }
        image.isTemplate = !appIcons
        return image
    }

    /// The app's icon centred in `tile`, if the tile has room for one
    /// (at least 5 pt) and the app is known to Launch Services.
    private static func icon(for bundleId: String, fitting tile: CGRect) -> (NSImage, CGRect)? {
        let side = min(min(tile.width, tile.height) - 1, 10)
        guard side >= 5, let img = cachedIcon(bundleId) else { return nil }
        let r = CGRect(x: tile.midX - side / 2, y: tile.midY - side / 2, width: side, height: side)
        return (img, r)
    }

    private static func cachedIcon(_ bundleId: String) -> NSImage? {
        if let hit = icons[bundleId] { return hit }
        let img = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundleId)
            .map { NSWorkspace.shared.icon(forFile: $0.path) }
            ?? NSRunningApplication.runningApplications(withBundleIdentifier: bundleId).first?.icon
        icons[bundleId] = img
        return img
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
