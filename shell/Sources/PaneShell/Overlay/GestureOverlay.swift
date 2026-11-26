import AppKit

/// Shows where a window moved by a gesture (hold the modifier and point)
/// will go: the frame it will get, a label, and for splits a faint outline
/// of the tile being split. No animation: Pane has none.
@MainActor
final class GestureOverlay {
    static let shared = GestureOverlay()
    private var panel: NSPanel?

    /// `frame` and `target` are in global top-left points (like AX).
    func show(frame: CGRect, target: CGRect?, label: String) {
        let preview = Self.cocoa(frame)
        let outline = target.map(Self.cocoa)
        let bounds = outline.map { preview.union($0) } ?? preview
        let p = panel ?? makePanel()
        panel = p
        let view = OverlayView(frame: NSRect(origin: .zero, size: bounds.size))
        view.preview = preview.offsetBy(dx: -bounds.minX, dy: -bounds.minY)
        view.outline = outline?.offsetBy(dx: -bounds.minX, dy: -bounds.minY)
        view.label = label
        p.contentView = view
        p.setFrame(bounds, display: true)
        p.orderFrontRegardless()
    }

    func hide() {
        panel?.orderOut(nil)
    }

    /// Global top-left rect -> Cocoa screen coordinates (bottom-left origin
    /// of the primary screen).
    static func cocoa(_ r: CGRect) -> NSRect {
        let top = NSScreen.screens.first?.frame.maxY ?? 0
        return NSRect(x: r.minX, y: top - (r.minY + r.height), width: r.width, height: r.height)
    }

    static func rect(_ any: Any?) -> CGRect? {
        guard let d = any as? [String: Any],
              let x = (d["x"] as? NSNumber)?.doubleValue,
              let y = (d["y"] as? NSNumber)?.doubleValue,
              let w = (d["w"] as? NSNumber)?.doubleValue,
              let h = (d["h"] as? NSNumber)?.doubleValue
        else { return nil }
        return CGRect(x: x, y: y, width: w, height: h)
    }

    private func makePanel() -> NSPanel {
        let p = NSPanel(contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: true)
        p.isOpaque = false
        p.backgroundColor = .clear
        p.level = .statusBar
        p.hasShadow = false
        p.ignoresMouseEvents = true
        p.collectionBehavior = [.canJoinAllSpaces, .stationary, .ignoresCycle]
        return p
    }
}

private final class OverlayView: NSView {
    var preview: NSRect = .zero
    var outline: NSRect?
    var label = ""

    override func draw(_ dirtyRect: NSRect) {
        let accent = NSColor.controlAccentColor
        if let o = outline {
            let path = NSBezierPath(roundedRect: o.insetBy(dx: 1, dy: 1), xRadius: 8, yRadius: 8)
            path.lineWidth = 1.5
            path.setLineDash([6, 4], count: 2, phase: 0)
            accent.withAlphaComponent(0.5).setStroke()
            path.stroke()
        }
        let box = NSBezierPath(roundedRect: preview.insetBy(dx: 1, dy: 1), xRadius: 8, yRadius: 8)
        accent.withAlphaComponent(0.25).setFill()
        box.fill()
        box.lineWidth = 2
        accent.setStroke()
        box.stroke()

        guard !label.isEmpty else { return }
        let attrs: [NSAttributedString.Key: Any] = [
            .font: NSFont.systemFont(ofSize: 13, weight: .medium),
            .foregroundColor: NSColor.white,
        ]
        let text = NSAttributedString(string: label, attributes: attrs)
        let size = text.size()
        let pill = NSRect(
            x: preview.midX - size.width / 2 - 12,
            y: preview.midY - size.height / 2 - 6,
            width: size.width + 24,
            height: size.height + 12
        )
        NSColor.black.withAlphaComponent(0.65).setFill()
        NSBezierPath(roundedRect: pill, xRadius: pill.height / 2, yRadius: pill.height / 2).fill()
        text.draw(at: NSPoint(x: pill.minX + 12, y: pill.minY + 6))
    }
}
