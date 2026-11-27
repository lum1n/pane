import AppKit

/// Draws a border around the focused window and, optionally, darkens the
/// other windows. Both follow the snapshot (tile rects are the frames Pane
/// gives windows), so they move when the layout does. No animation.
///
/// The dimming panels sit above normal windows, so they would cover the
/// focused window too; its rect is cut out of the dimmed area instead.
@MainActor
final class FocusBorder {
    static let shared = FocusBorder()

    private var border: NSPanel?
    private var dims: [String: NSPanel] = [:]
    private var lastBorder: BorderState?
    private var lastDims: [String: DimState] = [:]

    /// Just below the gesture preview (`.statusBar`), above app windows.
    private static let dimLevel = NSWindow.Level.floating
    private static let borderLevel = NSWindow.Level(rawValue: NSWindow.Level.floating.rawValue + 1)
    /// Corner radius of macOS windows.
    private static let windowRadius = UIConstants.windowCornerRadius

    func update(_ s: Snapshot) {
        let look = s.appearance ?? SnapAppearance(border_width: 0, border_color: nil, dim_unfocused: 0)
        let focused = focusedRect(s)
        updateBorder(focused: focused, look: look)
        updateDims(s, focused: focused, look: look)
    }

    // MARK: Border

    private struct BorderState: Equatable {
        var rect: CGRect
        var width: CGFloat
        var color: [Double]?
    }

    private func updateBorder(focused: CGRect?, look: SnapAppearance) {
        guard let rect = focused, look.border_width > 0 else {
            lastBorder = nil
            border?.orderOut(nil)
            return
        }
        let state = BorderState(rect: rect, width: CGFloat(look.border_width), color: look.border_color)
        if state == lastBorder, border?.isVisible == true { return }
        lastBorder = state

        let w = state.width
        // The stroke sits just outside the window so it never covers content.
        let frame = GestureOverlay.cocoa(rect).insetBy(dx: -w, dy: -w)
        let p = border ?? Self.makePanel(level: Self.borderLevel)
        border = p
        let view = BorderView(frame: NSRect(origin: .zero, size: frame.size))
        view.width = w
        view.color = Self.color(look.border_color)
        view.radius = Self.windowRadius + w / 2
        p.contentView = view
        p.setFrame(frame, display: true)
        p.orderFrontRegardless()
    }

    // MARK: Dimming

    private struct DimState: Equatable {
        var frame: NSRect
        var rects: [NSRect]
        var cut: NSRect?
        var alpha: Double
    }

    private func updateDims(_ s: Snapshot, focused: CGRect?, look: SnapAppearance) {
        // Nothing to dim against when no managed window has focus (e.g. the
        // desktop or a Pane window is frontmost).
        let on = look.dim_unfocused > 0 && focused != nil
        var seen = Set<String>()
        if on {
            for d in s.displays {
                let frame = GestureOverlay.cocoa(d.frame.cg)
                let rects = d.tiles
                    .filter { !$0.focused && !$0.pending }
                    .map { GestureOverlay.cocoa(Self.global($0.rect, in: d.frame)) }
                    .map { $0.offsetBy(dx: -frame.minX, dy: -frame.minY) }
                guard !rects.isEmpty else { continue }
                seen.insert(d.id)
                let cut = focused
                    .map { GestureOverlay.cocoa($0).offsetBy(dx: -frame.minX, dy: -frame.minY) }
                let state = DimState(frame: frame, rects: rects, cut: cut, alpha: look.dim_unfocused)
                let p = dims[d.id] ?? Self.makePanel(level: Self.dimLevel)
                dims[d.id] = p
                if state == lastDims[d.id], p.isVisible { continue }
                lastDims[d.id] = state
                let view = DimView(frame: NSRect(origin: .zero, size: frame.size))
                view.rects = rects
                view.cut = cut
                view.alpha = CGFloat(look.dim_unfocused)
                p.contentView = view
                p.setFrame(frame, display: true)
                p.orderFrontRegardless()
            }
        }
        for (id, p) in dims where !seen.contains(id) {
            p.orderOut(nil)
            lastDims[id] = nil
        }
        // Keep the border above the dimming after reordering.
        if let b = border, b.isVisible { b.orderFrontRegardless() }
    }

    // MARK: Helpers

    /// Global top-left rect of the focused tile, if a managed window has focus.
    private func focusedRect(_ s: Snapshot) -> CGRect? {
        for d in s.displays {
            if let t = d.tiles.first(where: { $0.focused }) {
                return Self.global(t.rect, in: d.frame)
            }
        }
        return nil
    }

    /// Tile rects are fractions of their display's frame.
    private static func global(_ r: PRect, in f: PRect) -> CGRect {
        CGRect(x: f.x + r.x * f.w, y: f.y + r.y * f.h, width: r.w * f.w, height: r.h * f.h)
    }

    private static func color(_ rgba: [Double]?) -> NSColor {
        guard let c = rgba, c.count == 4 else { return .controlAccentColor }
        return NSColor(srgbRed: c[0], green: c[1], blue: c[2], alpha: c[3])
    }

    private static func makePanel(level: NSWindow.Level) -> NSPanel {
        let p = NSPanel(contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: true)
        p.isOpaque = false
        p.backgroundColor = .clear
        p.level = level
        p.hasShadow = false
        p.ignoresMouseEvents = true
        p.isReleasedWhenClosed = false
        p.collectionBehavior = [.canJoinAllSpaces, .stationary, .ignoresCycle, .fullScreenAuxiliary]
        return p
    }
}

private final class BorderView: NSView {
    var width: CGFloat = 2
    var color: NSColor = .controlAccentColor
    var radius: CGFloat = UIConstants.windowCornerRadius

    override func draw(_ dirtyRect: NSRect) {
        let r = bounds.insetBy(dx: width / 2, dy: width / 2)
        let path = NSBezierPath(roundedRect: r, xRadius: radius, yRadius: radius)
        path.lineWidth = width
        color.setStroke()
        path.stroke()
    }
}

private final class DimView: NSView {
    var rects: [NSRect] = []
    var cut: NSRect?
    var alpha: CGFloat = 0.3

    override func draw(_ dirtyRect: NSRect) {
        guard let ctx = NSGraphicsContext.current else { return }
        ctx.saveGraphicsState()
        if let cut {
            // Everything except the focused window.
            let clip = NSBezierPath(rect: bounds)
            clip.append(NSBezierPath(rect: cut))
            clip.windingRule = .evenOdd
            clip.addClip()
        }
        // One path with non-zero winding so overlapping windows (a float over
        // a tile) are not darkened twice.
        let path = NSBezierPath()
        for r in rects { path.appendRect(r) }
        path.windingRule = .nonZero
        NSColor.black.withAlphaComponent(alpha).setFill()
        path.fill()
        ctx.restoreGraphicsState()
    }
}
