import AppKit

/// Short-lived message under the menu bar. No notification permission needed.
@MainActor
final class NoticeHUD {
    static let shared = NoticeHUD()
    private var panel: NSPanel?
    private var hideWork: DispatchWorkItem?
    weak var anchor: NSStatusBarButton?

    func show(_ message: String) {
        let label = NSTextField(labelWithString: message)
        label.font = .systemFont(ofSize: 12)
        label.textColor = .labelColor
        label.lineBreakMode = .byTruncatingTail
        label.maximumNumberOfLines = 2
        label.preferredMaxLayoutWidth = 340
        let size = label.fittingSize
        let w = min(max(size.width + 24, 120), 364)
        let h = size.height + 14

        let p = panel ?? makePanel()
        panel = p
        let effect = NSVisualEffectView(frame: NSRect(x: 0, y: 0, width: w, height: h))
        effect.material = .hudWindow
        effect.state = .active
        effect.wantsLayer = true
        effect.layer?.cornerRadius = 8
        label.frame = NSRect(x: 12, y: 7, width: w - 24, height: size.height)
        effect.addSubview(label)
        p.contentView = effect

        var origin = NSPoint(x: 0, y: 0)
        if let b = anchor, let win = b.window {
            let r = win.convertToScreen(b.convert(b.bounds, to: nil))
            origin = NSPoint(x: r.midX - w / 2, y: r.minY - h - 6)
        } else if let screen = NSScreen.main {
            origin = NSPoint(x: screen.visibleFrame.maxX - w - 12, y: screen.visibleFrame.maxY - h - 6)
        }
        p.setFrame(NSRect(origin: origin, size: NSSize(width: w, height: h)), display: true)
        p.orderFrontRegardless()

        hideWork?.cancel()
        let work = DispatchWorkItem { [weak self] in
            MainActor.assumeIsolated { self?.panel?.orderOut(nil) }
        }
        hideWork = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 3.5, execute: work)
    }

    private func makePanel() -> NSPanel {
        let p = NSPanel(contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: true)
        p.isOpaque = false
        p.backgroundColor = .clear
        p.level = .statusBar
        p.hasShadow = true
        p.ignoresMouseEvents = true
        p.collectionBehavior = [.canJoinAllSpaces, .stationary, .ignoresCycle]
        return p
    }
}
