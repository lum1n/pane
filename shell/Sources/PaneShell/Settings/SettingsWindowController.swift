import AppKit
import Combine
import SwiftUI

/// The Settings window: toolbar tabs (like Finder and Safari), one SwiftUI
/// pane per tab. The window takes each pane's height when you switch, and
/// its title follows the selected tab.
@MainActor
final class SettingsWindowController: NSWindowController, NSWindowDelegate {
    private let model: SettingsModel

    init() {
        let model = SettingsModel()
        self.model = model
        let tabs = SettingsTabViewController()
        tabs.tabStyle = .toolbar
        tabs.transitionOptions = NSWorkspace.shared.accessibilityDisplayShouldReduceMotion ? [] : [.crossfade]

        func add(_ title: String, _ symbol: String, _ view: some View) {
            let host = NSHostingController(rootView: view)
            host.sizingOptions = .preferredContentSize
            host.title = title
            let item = NSTabViewItem(viewController: host)
            item.label = title
            item.image = NSImage(systemSymbolName: symbol, accessibilityDescription: title)
            tabs.addTabViewItem(item)
        }
        add("General", "gearshape", GeneralPane(model: model))
        add("Layout", "rectangle.split.2x1", LayoutPane(model: model))
        add("Focus", "scope", FocusPane(model: model))
        add("Apps", "square.on.square", AppsPane(model: model))
        add("Scratchpads", "rectangle.on.rectangle", ScratchpadsPane(model: model))
        add("Shortcuts", "keyboard", ShortcutsPane(model: model))

        let window = NSWindow(contentViewController: tabs)
        window.styleMask = [.titled, .closable]
        window.toolbarStyle = .preference
        window.isReleasedWhenClosed = false
        super.init(window: window)
        window.delegate = self
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    func show() {
        model.load()
        NSApp.activate()
        // No saved frame: the height depends on the pane.
        if window?.isVisible != true { window?.center() }
        showWindow(nil)
    }

    func windowWillClose(_ notification: Notification) {
        model.stopRecording()
    }
}

/// Resizes the window to the selected pane, keeping its top edge still.
private final class SettingsTabViewController: NSTabViewController {
    override func tabView(_ tabView: NSTabView, didSelect item: NSTabViewItem?) {
        super.tabView(tabView, didSelect: item)
        guard let window = view.window, let size = item?.viewController?.preferredContentSize,
              size.width > 0, size.height > 0 else { return }
        var frame = window.frameRect(forContentRect: NSRect(origin: .zero, size: size))
        frame.origin = NSPoint(x: window.frame.minX, y: window.frame.maxY - frame.height)
        let animate = window.isVisible && !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
        window.setFrame(frame, display: true, animate: animate)
    }
}

// MARK: - Pane chrome

private struct SettingsPaneModifier: ViewModifier {
    @Bindable var model: SettingsModel
    let height: CGFloat

    func body(content: Content) -> some View {
        content
            // System Settings look: inset rounded groups, labels left and
            // controls right. Without this, macOS falls back to `.columns`.
            .formStyle(.grouped)
            .statusCapsule($model.message, autoDismiss: model.recording == nil)
            .frame(width: UIConstants.settingsWidth, height: height)
            .onAppear { model.load() }
            // The window is reused; refresh whenever it comes forward.
            .onReceive(NotificationCenter.default.publisher(for: NSWindow.didBecomeKeyNotification)) { _ in
                if model.recording == nil { model.load() }
            }
    }
}

extension View {
    /// Fixed-size Settings pane with the shared status capsule and refresh.
    func settingsPane(_ model: SettingsModel, height: CGFloat) -> some View {
        modifier(SettingsPaneModifier(model: model, height: height))
    }
}

/// A number with a stepper and a unit ("8 pt"), as one form row.
struct NumberRow: View {
    let title: String
    @Binding var value: Double
    var range: ClosedRange<Double> = 0...100
    var step: Double = 1
    var unit = "pt"

    var body: some View {
        LabeledContent(title) {
            HStack(spacing: 4) {
                TextField(title, value: $value, format: .number)
                    .labelsHidden()
                    .multilineTextAlignment(.trailing)
                    .frame(width: 56)
                Stepper(title, value: $value, in: range, step: step)
                    .labelsHidden()
                Text(unit).foregroundStyle(.secondary)
            }
        }
    }
}

/// The +/− bar under a list (the Login Items pattern).
struct ListEditBar<AddMenu: View>: View {
    let canRemove: Bool
    let remove: () -> Void
    @ViewBuilder var add: AddMenu
    var trailing: AnyView? = nil

    var body: some View {
        HStack(spacing: 2) {
            add
                .menuStyle(.button)
                .buttonStyle(.borderless)
                .menuIndicator(.hidden)
                .fixedSize()
                .help("Add")
            Divider().frame(height: 14)
            Button(action: remove) { Image(systemName: "minus").frame(width: 20, height: 18) }
                .buttonStyle(.borderless)
                .disabled(!canRemove)
                .help("Remove the selected item")
            Spacer()
            if let trailing { trailing }
        }
        .padding(.vertical, 2)
    }
}

/// A list row that can be selected by clicking it.
struct SelectableRow: ViewModifier {
    let selected: Bool

    func body(content: Content) -> some View {
        content
            .padding(.vertical, 2)
            .padding(.horizontal, 6)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                RoundedRectangle(cornerRadius: 6, style: .continuous)
                    .fill(selected ? Color.accentColor.opacity(0.18) : .clear)
            )
            .contentShape(Rectangle())
    }
}
