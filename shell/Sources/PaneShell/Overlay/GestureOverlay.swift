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
        if !label.isEmpty { view.addSubview(Self.pill(label, centeredIn: view.preview)) }
        p.contentView = view
        p.setFrame(bounds, display: true)
        p.orderFrontRegardless()
    }

    func hide() {
        panel?.orderOut(nil)
    }

    /// The label in a small glass capsule.
    private static func pill(_ text: String, centeredIn r: NSRect) -> NSView {
        let label = NSTextField(labelWithString: text)
        label.font = .systemFont(ofSize: 13, weight: .medium)
        label.textColor = .labelColor
        let size = label.fittingSize
        let w = ceil(size.width + 28), h = ceil(size.height + 14)
        let container = NSView(frame: NSRect(x: 0, y: 0, width: w, height: h))
        label.frame = NSRect(x: 14, y: (h - size.height) / 2, width: size.width, height: size.height)
        container.addSubview(label)
        let glass = NSGlassEffectView(frame: NSRect(x: r.midX - w / 2, y: r.midY - h / 2, width: w, height: h))
        glass.cornerRadius = h / 2
        glass.contentView = container
        return glass
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

    override func draw(_ dirtyRect: NSRect) {
        let accent = NSColor.controlAccentColor
        let radius = UIConstants.windowCornerRadius
        if let o = outline {
            let path = NSBezierPath(roundedRect: o.insetBy(dx: 1, dy: 1), xRadius: radius, yRadius: radius)
            path.lineWidth = 1.5
            path.setLineDash([6, 4], count: 2, phase: 0)
            accent.withAlphaComponent(0.5).setStroke()
            path.stroke()
        }
        let box = NSBezierPath(roundedRect: preview.insetBy(dx: 1, dy: 1), xRadius: radius, yRadius: radius)
        accent.withAlphaComponent(0.25).setFill()
        box.fill()
        box.lineWidth = 2
        accent.setStroke()
        box.stroke()
    }
}
