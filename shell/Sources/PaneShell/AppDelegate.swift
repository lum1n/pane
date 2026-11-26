import AppKit
import SwiftUI

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private var statusItem: NSStatusItem!
    private let statusMenu = StatusMenu()
    private let events = SystemEvents()
    private var editor: ProfileEditorWindow?
    private var settingsWindow: NSWindow?
    private var shownAxPrompt = false

    func applicationDidFinishLaunching(_ notification: Notification) {
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        statusItem.button?.image = StatusIconRenderer.placeholder()
        statusItem.button?.imagePosition = .imageOnly
        statusItem.button?.toolTip = "Pane"
        statusItem.menu = statusMenu.menu
        NoticeHUD.shared.anchor = statusItem.button
        statusMenu.openEditor = { [weak self] in self?.showEditor() }
        statusMenu.openSettings = { [weak self] in self?.showSettings() }

        let core = PaneCore.shared
        core.onSnapshot = { [weak self] s in
            self?.statusItem.button?.image = StatusIconRenderer.render(s)
            self?.editor?.model.snapshot = s
        }
        core.onRequest = { [weak self] r in self?.handle(request: r) }
        if let err = core.start() {
            let a = NSAlert()
            a.messageText = "Pane could not start"
            a.informativeText = err
            a.runModal()
            NSApp.terminate(nil)
            return
        }
        events.start()
    }

    func applicationWillTerminate(_ notification: Notification) {
        // Unpark every window before we go (bounded to ~3 s by the core).
        PaneCore.shared.shutdown()
    }

    private func handle(request r: [String: Any]) {
        switch r["request"] as? String {
        case "launch_app":
            guard let bundle = r["bundle_id"] as? String, let id = r["request_id"] as? Int else { return }
            launch(bundle: bundle, args: r["args"] as? [String] ?? [], requestId: id)
        case "notice":
            if let m = r["message"] as? String { NoticeHUD.shared.show(m) }
        case "open_editor":
            showEditor()
        case "profile_picker":
            statusMenu.popUpProfilePicker()
        case "ax_permission_missing":
            if !shownAxPrompt {
                shownAxPrompt = true
                // macOS shows its own prompt first; explain only if still missing.
                DispatchQueue.main.asyncAfter(deadline: .now() + 2) {
                    MainActor.assumeIsolated {
                        if !Permissions.accessibilityGranted { Permissions.explain() }
                    }
                }
            }
        default:
            break
        }
    }

    private func launch(bundle: String, args: [String], requestId: Int) {
        guard let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundle) else {
            PaneCore.shared.event(["event": "app_open_result", "request_id": requestId, "ok": false, "error": "not installed"])
            return
        }
        let cfg = NSWorkspace.OpenConfiguration()
        cfg.activates = false
        cfg.arguments = args
        NSWorkspace.shared.openApplication(at: url, configuration: cfg) { _, error in
            DispatchQueue.main.async {
                MainActor.assumeIsolated {
                    var ev: [String: Any] = ["event": "app_open_result", "request_id": requestId, "ok": error == nil]
                    if let error { ev["error"] = error.localizedDescription }
                    PaneCore.shared.event(ev)
                }
            }
        }
    }

    func showEditor() {
        if editor == nil { editor = ProfileEditorWindow() }
        editor?.model.snapshot = PaneCore.shared.snapshot
        editor?.show()
    }

    func showSettings() {
        if settingsWindow == nil {
            let w = NSWindow(contentViewController: NSHostingController(rootView: SettingsView()))
            w.title = "Pane Settings"
            w.styleMask = [.titled, .closable]
            w.isReleasedWhenClosed = false
            settingsWindow = w
        }
        NSApp.activate(ignoringOtherApps: true)
        settingsWindow?.center()
        settingsWindow?.makeKeyAndOrderFront(nil)
    }
}
