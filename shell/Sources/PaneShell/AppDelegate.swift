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
    private var appearanceObservation: NSKeyValueObservation?

    func applicationDidFinishLaunching(_ notification: Notification) {
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        statusItem.button?.image = StatusIconRenderer.placeholder()
        statusItem.button?.imagePosition = .imageOnly
        statusItem.button?.toolTip = "Pane"
        // No `statusItem.menu`: a click on a window's tile focuses it, any
        // other click opens the menu (see `statusClicked`).
        statusItem.button?.target = self
        statusItem.button?.action = #selector(statusClicked(_:))
        statusItem.button?.sendAction(on: [.leftMouseDown, .rightMouseDown])
        // Full-colour icons (app icons on) follow light and dark menu bars.
        appearanceObservation = statusItem.button?.observe(\.effectiveAppearance) { [weak self] _, _ in
            DispatchQueue.main.async { MainActor.assumeIsolated { self?.redrawIcon() } }
        }
        NoticeHUD.shared.anchor = statusItem.button
        statusMenu.openEditor = { [weak self] in self?.showEditor() }
        statusMenu.openSettings = { [weak self] in self?.showSettings() }
        statusMenu.reviewDrift = { [weak self] name in self?.reviewDrift(name) }

        let core = PaneCore.shared
        core.onSnapshot = { [weak self] s in
            self?.redrawIcon(s)
            self?.editor?.model.snapshot = s
            FocusBorder.shared.update(s)
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
        case "gesture_overlay":
            guard let frame = GestureOverlay.rect(r["frame"]) else { return }
            GestureOverlay.shared.show(
                frame: frame,
                target: GestureOverlay.rect(r["target"]),
                label: r["label"] as? String ?? ""
            )
        case "gesture_overlay_hide":
            GestureOverlay.shared.hide()
        case "open_editor":
            showEditor()
        case "profile_picker":
            statusMenu.popUpProfilePicker()
        case "palette":
            CommandPalette.shared.toggle()
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

    private func redrawIcon(_ snapshot: Snapshot? = nil) {
        guard let button = statusItem.button else { return }
        let dark = button.effectiveAppearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
        let s = snapshot ?? PaneCore.shared.snapshot
        button.image = StatusIconRenderer.render(s, dark: dark)
        button.toolTip = s?.profile_drift == true
            ? "Pane: the desk differs from “\(s?.active_profile ?? "")”"
            : "Pane"
    }

    /// Left click on a window's tile in the icon focuses that window; a
    /// click anywhere else, on the focused tile, or a right/control click
    /// opens the menu.
    @objc private func statusClicked(_ sender: NSStatusBarButton) {
        if let event = NSApp.currentEvent, event.type == .leftMouseDown,
           !event.modifierFlags.contains(.control),
           let s = PaneCore.shared.snapshot, let image = sender.image {
            let p = sender.convert(event.locationInWindow, from: nil)
            // The image is centred in the button; the layout's origin is top left.
            let fromTop = sender.isFlipped ? p.y : sender.bounds.height - p.y
            let point = CGPoint(
                x: p.x - (sender.bounds.width - image.size.width) / 2,
                y: fromTop - (sender.bounds.height - image.size.height) / 2
            )
            if let t = StatusIconRenderer.tile(at: point, in: s), let wid = t.wid, !t.focused {
                if case let .error(m) = PaneCore.shared.send(["cmd": "focus-window", "wid": Int(wid)]) {
                    NoticeHUD.shared.show(m)
                }
                return
            }
        }
        // Attach the menu just for this click so the next one is ours again.
        statusItem.menu = statusMenu.menu
        sender.performClick(nil)
        statusItem.menu = nil
    }

    func showEditor() {
        if editor == nil { editor = ProfileEditorWindow() }
        editor?.model.snapshot = PaneCore.shared.snapshot
        editor?.show()
    }

    /// Open the editor on the diff between the desk and saved profile `name`.
    func reviewDrift(_ name: String) {
        showEditor()
        editor?.model.compareWithDesk(name)
    }

    func showSettings() {
        if settingsWindow == nil {
            let w = NSWindow(contentViewController: NSHostingController(rootView: SettingsView()))
            w.title = "Pane Settings"
            w.styleMask = [.titled, .closable]
            w.setContentSize(NSSize(width: 620, height: 640))
            w.isReleasedWhenClosed = false
            settingsWindow = w
        }
        NSApp.activate(ignoringOtherApps: true)
        settingsWindow?.center()
        settingsWindow?.makeKeyAndOrderFront(nil)
    }
}
