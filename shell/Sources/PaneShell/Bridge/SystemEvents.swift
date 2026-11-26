import AppKit

/// Forwards NSWorkspace / NSScreen notifications to the core.
@MainActor
final class SystemEvents {
    private var tokens: [NSObjectProtocol] = []

    func start() {
        let ws = NSWorkspace.shared.notificationCenter
        tokens.append(ws.addObserver(forName: NSWorkspace.didLaunchApplicationNotification, object: nil, queue: .main) { note in
            guard let app = note.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication else { return }
            MainActor.assumeIsolated {
                var ev: [String: Any] = [
                    "event": "app_launched",
                    "pid": Int(app.processIdentifier),
                    "name": app.localizedName ?? "",
                    "regular": app.activationPolicy == .regular,
                ]
                if let b = app.bundleIdentifier { ev["bundle_id"] = b }
                PaneCore.shared.event(ev)
            }
        })
        tokens.append(ws.addObserver(forName: NSWorkspace.didTerminateApplicationNotification, object: nil, queue: .main) { note in
            guard let app = note.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication else { return }
            MainActor.assumeIsolated {
                PaneCore.shared.event(["event": "app_terminated", "pid": Int(app.processIdentifier)])
            }
        })
        tokens.append(ws.addObserver(forName: NSWorkspace.willSleepNotification, object: nil, queue: .main) { _ in
            MainActor.assumeIsolated { PaneCore.shared.event(["event": "will_sleep"]) }
        })
        tokens.append(ws.addObserver(forName: NSWorkspace.didWakeNotification, object: nil, queue: .main) { _ in
            MainActor.assumeIsolated { PaneCore.shared.event(["event": "did_wake"]) }
        })
        tokens.append(NotificationCenter.default.addObserver(
            forName: NSApplication.didChangeScreenParametersNotification, object: nil, queue: .main
        ) { _ in
            MainActor.assumeIsolated { PaneCore.shared.event(["event": "screens_changed"]) }
        })
    }
}
