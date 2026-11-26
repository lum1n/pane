import AppKit
import ApplicationServices

enum Permissions {
    static var accessibilityGranted: Bool { AXIsProcessTrusted() }

    static func openAccessibilitySettings() {
        if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility") {
            NSWorkspace.shared.open(url)
        }
    }

    /// Explain why Pane needs Accessibility and offer to open Settings.
    @MainActor
    static func explain() {
        let a = NSAlert()
        a.messageText = "Pane needs Accessibility access"
        a.informativeText = """
        Pane moves and resizes windows through the macOS Accessibility API. \
        Turn on Pane in System Settings > Privacy & Security > Accessibility. \
        Pane starts tiling as soon as access is granted.
        """
        a.addButton(withTitle: "Open System Settings")
        a.addButton(withTitle: "Later")
        NSApp.activate(ignoringOtherApps: true)
        if a.runModal() == .alertFirstButtonReturn {
            openAccessibilitySettings()
        }
    }
}
