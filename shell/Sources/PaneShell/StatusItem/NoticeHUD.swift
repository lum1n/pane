import AppKit

/// Short-lived message under the menu bar, in a glass capsule. No
/// notification permission needed.
@MainActor
final class NoticeHUD {
    static let shared = NoticeHUD()
    /// Symbol for errors; plain notices use `info.circle`.
    static let errorSymbol = "exclamationmark.triangle.fill"

    private var panel: NSPanel?
    private var hideWork: DispatchWorkItem?
    /// Bumped on every show, so a fade-out that finishes late doesn't hide
    /// a newer message.
    private var generation = 0
    weak var anchor: NSStatusBarButton?

    private static let fade: TimeInterval = 0.15
    private static let maxTextWidth: CGFloat = 360

    private var reduceMotion: Bool { NSWorkspace.shared.accessibilityDisplayShouldReduceMotion }

    func show(_ message: String, symbol: String = "info.circle") {
        generation += 1
        let label = NSTextField(wrappingLabelWithString: message)
        label.font = .systemFont(ofSize: 13)
        label.textColor = .labelColor
        label.lineBreakMode = .byTruncatingTail
        label.maximumNumberOfLines = 2
        label.preferredMaxLayoutWidth = Self.maxTextWidth
        label.isSelectable = false
        let text = label.fittingSize

        let icon = NSImageView()
        let config = NSImage.SymbolConfiguration(pointSize: 14, weight: .medium)
            .applying(NSImage.SymbolConfiguration(hierarchicalColor: symbol == Self.errorSymbol ? .systemOrange : .secondaryLabelColor))
        icon.image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil)?.withSymbolConfiguration(config)
        let iconSize = icon.image?.size ?? .zero

        let padX: CGFloat = 16, padY: CGFloat = 9, spacing: CGFloat = 8
        let w = ceil(padX + iconSize.width + spacing + min(text.width, Self.maxTextWidth) + padX)
        let h = ceil(max(text.height, iconSize.height) + 2 * padY)

        let container = NSView(frame: NSRect(x: 0, y: 0, width: w, height: h))
        icon.frame = NSRect(x: padX, y: (h - iconSize.height) / 2, width: iconSize.width, height: iconSize.height)
        label.frame = NSRect(x: padX + iconSize.width + spacing, y: (h - text.height) / 2,
                             width: min(text.width, Self.maxTextWidth), height: text.height)
        container.addSubview(icon)
        container.addSubview(label)

        let glass = NSGlassEffectView(frame: container.frame)
        glass.cornerRadius = min(h / 2, 20)
        glass.contentView = container

        let p = panel ?? makePanel()
        panel = p
        p.contentView = glass
        p.setAccessibilityLabel(message)

        var origin = NSPoint(x: 0, y: 0)
        if let b = anchor, let win = b.window {
            let r = win.convertToScreen(b.convert(b.bounds, to: nil))
            origin = NSPoint(x: r.midX - w / 2, y: r.minY - h - 6)
        } else if let screen = NSScreen.main {
            origin = NSPoint(x: screen.visibleFrame.maxX - w - 12, y: screen.visibleFrame.maxY - h - 6)
        }
        p.setFrame(NSRect(origin: origin, size: NSSize(width: w, height: h)), display: true)

        let wasVisible = p.isVisible && p.alphaValue > 0
        p.orderFrontRegardless()
        if wasVisible || reduceMotion {
            p.alphaValue = 1
        } else {
            p.alphaValue = 0
            NSAnimationContext.runAnimationGroup { ctx in
                ctx.duration = Self.fade
                p.animator().alphaValue = 1
            }
        }

        hideWork?.cancel()
        let gen = generation
        let work = DispatchWorkItem { [weak self] in
            MainActor.assumeIsolated { self?.hide(generation: gen) }
        }
        hideWork = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 3.5, execute: work)
    }

    private func hide(generation gen: Int) {
        guard gen == generation, let p = panel else { return }
        if reduceMotion {
            p.orderOut(nil)
            return
        }
        NSAnimationContext.runAnimationGroup({ ctx in
            ctx.duration = Self.fade
            p.animator().alphaValue = 0
        }, completionHandler: { [weak self] in
            MainActor.assumeIsolated {
                guard let self, self.generation == gen else { return }
                self.panel?.orderOut(nil)
            }
        })
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
