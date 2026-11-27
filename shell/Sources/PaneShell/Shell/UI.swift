import AppKit
import SwiftUI

/// Shared measurements, so the overlay, the focus border and the windows agree.
enum UIConstants {
    /// Corner radius of macOS 26 windows (the focus border and gesture
    /// overlay follow it).
    static let windowCornerRadius: CGFloat = 12
    /// Width of every Settings pane.
    static let settingsWidth: CGFloat = 560
    /// How long a status capsule stays up.
    static let statusDismissDelay: Duration = .seconds(4)
}

// MARK: - Keycaps

/// A chord such as `ctrl-alt-h` drawn as rounded keycaps (⌃ ⌥ H). While
/// `recording`, it shows a "Type shortcut…" prompt with an accent outline.
struct KeycapView: View {
    let chord: String?
    var recording = false

    var body: some View {
        if recording {
            Text("Type shortcut…")
                .font(.callout)
                .foregroundStyle(.tint)
                .padding(.horizontal, 8)
                .frame(minHeight: 22)
                .overlay(Capsule().strokeBorder(Color.accentColor, lineWidth: 1.5))
        } else if let chord, !chord.isEmpty {
            HStack(spacing: 3) {
                ForEach(Array(Shortcut.glyphs(chord).enumerated()), id: \.offset) { _, g in
                    Keycap(glyph: g)
                }
            }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(Shortcut.pretty(chord))
        } else {
            Text("—").foregroundStyle(.tertiary)
        }
    }
}

private struct Keycap: View {
    let glyph: String

    var body: some View {
        Text(glyph)
            .font(.system(size: 11, weight: .medium, design: .rounded))
            .foregroundStyle(.secondary)
            .padding(.horizontal, 5)
            .frame(minWidth: 20, minHeight: 20)
            .background(RoundedRectangle(cornerRadius: 5, style: .continuous).fill(.quaternary))
            .overlay(RoundedRectangle(cornerRadius: 5, style: .continuous).strokeBorder(.separator))
    }
}

/// AppKit twin of `KeycapView`, for the palette. The images draw lazily, so
/// they follow light and dark mode.
enum KeycapDrawing {
    private static let height: CGFloat = 18
    private static let font = NSFont.systemFont(ofSize: 11, weight: .medium)

    static func image(for chord: String) -> NSImage? {
        let glyphs = Shortcut.glyphs(chord)
        guard !glyphs.isEmpty else { return nil }
        let font = Self.font, h = Self.height, gap: CGFloat = 3, pad: CGFloat = 5
        let attrs: [NSAttributedString.Key: Any] = [.font: font]
        let widths = glyphs.map { max(h, ceil(($0 as NSString).size(withAttributes: attrs).width) + 2 * pad) }
        let total = widths.reduce(0, +) + gap * CGFloat(glyphs.count - 1)
        let image = NSImage(size: NSSize(width: total, height: h), flipped: false) { _ in
            var x: CGFloat = 0
            for (g, w) in zip(glyphs, widths) {
                let r = NSRect(x: x, y: 0, width: w, height: h).insetBy(dx: 0.5, dy: 0.5)
                let path = NSBezierPath(roundedRect: r, xRadius: 5, yRadius: 5)
                NSColor.quaternaryLabelColor.setFill()
                path.fill()
                NSColor.separatorColor.setStroke()
                path.lineWidth = 1
                path.stroke()
                draw(g, centeredIn: r, font: font, color: .secondaryLabelColor)
                x += w + gap
            }
            return true
        }
        image.accessibilityDescription = Shortcut.pretty(chord)
        return image
    }

    /// A small capsule tag such as "Current".
    static func tag(_ text: String) -> NSImage {
        let font = NSFont.systemFont(ofSize: 10, weight: .medium), h: CGFloat = 16
        let w = ceil((text as NSString).size(withAttributes: [.font: font]).width) + 12
        let image = NSImage(size: NSSize(width: w, height: h), flipped: false) { rect in
            NSColor.quaternaryLabelColor.setFill()
            NSBezierPath(roundedRect: rect, xRadius: h / 2, yRadius: h / 2).fill()
            draw(text, centeredIn: rect, font: font, color: .secondaryLabelColor)
            return true
        }
        image.accessibilityDescription = text
        return image
    }

    private static func draw(_ s: String, centeredIn r: NSRect, font: NSFont, color: NSColor) {
        let attrs: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: color]
        let size = (s as NSString).size(withAttributes: attrs)
        (s as NSString).draw(at: NSPoint(x: r.midX - size.width / 2, y: r.midY - size.height / 2), withAttributes: attrs)
    }
}

// MARK: - Status capsule

/// A short message in a floating glass capsule.
struct StatusCapsule: View {
    let text: String
    var symbol = "info.circle"

    var body: some View {
        Label {
            Text(text).lineLimit(2).multilineTextAlignment(.leading)
        } icon: {
            Image(systemName: symbol).symbolRenderingMode(.hierarchical)
        }
        .font(.callout)
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
        .glassEffect(.regular, in: .capsule)
        .accessibilityElement(children: .combine)
    }
}

private struct StatusCapsuleModifier: ViewModifier {
    @Binding var message: String?
    let autoDismiss: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    func body(content: Content) -> some View {
        content
            .overlay(alignment: .bottom) {
                if let m = message {
                    StatusCapsule(text: m)
                        .padding(.horizontal, 24)
                        .padding(.bottom, 16)
                        .transition(.opacity)
                }
            }
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.2), value: message)
            .task(id: TaskKey(message: message, autoDismiss: autoDismiss)) {
                guard autoDismiss, message != nil else { return }
                try? await Task.sleep(for: UIConstants.statusDismissDelay)
                if !Task.isCancelled { message = nil }
            }
    }

    private struct TaskKey: Equatable {
        var message: String?
        var autoDismiss: Bool
    }
}

extension View {
    /// Float `message` in a glass capsule at the bottom and clear it after
    /// a few seconds (unless `autoDismiss` is false, e.g. while recording).
    func statusCapsule(_ message: Binding<String?>, autoDismiss: Bool = true) -> some View {
        modifier(StatusCapsuleModifier(message: message, autoDismiss: autoDismiss))
    }
}

// MARK: - App icons

/// An app's Finder icon (cached by `AppIcons`).
struct AppIconView: View {
    let bundleId: String
    var size: CGFloat = 20

    var body: some View {
        Image(nsImage: AppIcons.icon(for: bundleId))
            .resizable()
            .interpolation(.high)
            .frame(width: size, height: size)
            .accessibilityHidden(true)
    }
}

// MARK: - Search field

/// A native `NSSearchField`, for views without a window toolbar to host
/// `.searchable` (the Settings panes live under a tab controller's toolbar).
struct SearchField: NSViewRepresentable {
    @Binding var text: String
    var prompt = "Search"

    func makeNSView(context: Context) -> NSSearchField {
        let f = NSSearchField()
        f.placeholderString = prompt
        f.delegate = context.coordinator
        f.sendsSearchStringImmediately = true
        return f
    }

    func updateNSView(_ f: NSSearchField, context: Context) {
        if f.stringValue != text { f.stringValue = text }
    }

    func makeCoordinator() -> Coordinator { Coordinator(text: $text) }

    @MainActor
    final class Coordinator: NSObject, NSSearchFieldDelegate {
        var text: Binding<String>
        init(text: Binding<String>) { self.text = text }

        func controlTextDidChange(_ obj: Notification) {
            if let f = obj.object as? NSSearchField { text.wrappedValue = f.stringValue }
        }
    }
}
