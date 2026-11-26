import AppKit

/// The status item's menu, rebuilt from the latest snapshot each time it opens.
@MainActor
final class StatusMenu: NSObject, NSMenuDelegate {
    let menu = NSMenu()
    var openEditor: () -> Void = {}
    var openSettings: () -> Void = {}

    override init() {
        super.init()
        menu.delegate = self
        menu.autoenablesItems = false
    }

    func menuNeedsUpdate(_ menu: NSMenu) {
        rebuild()
    }

    private func item(_ title: String, _ action: Selector?, key: String = "", obj: Any? = nil) -> NSMenuItem {
        let i = NSMenuItem(title: title, action: action, keyEquivalent: key)
        i.target = self
        i.representedObject = obj
        return i
    }

    func rebuild() {
        menu.removeAllItems()
        let snap = PaneCore.shared.snapshot
        if let s = snap {
            if !s.ax_trusted {
                menu.addItem(item("Grant Accessibility Access…", #selector(grantAccess)))
                menu.addItem(.separator())
            }
            if let err = s.config_error {
                let e = item("Config error: \(err)", nil)
                e.isEnabled = false
                menu.addItem(e)
                menu.addItem(.separator())
            }
            for d in s.displays.sorted(by: { $0.rank < $1.rank }) {
                let header = item(d.name + (d.focused ? "  ●" : ""), nil)
                header.isEnabled = false
                menu.addItem(header)
                for w in s.workspaces where w.display == d.id {
                    let title = w.windows > 0 ? "  \(w.label)  (\(w.windows))" : "  \(w.label)"
                    let i = item(title, #selector(showWorkspace(_:)), obj: w.target)
                    i.state = w.visible ? .on : .off
                    menu.addItem(i)
                }
            }
            menu.addItem(.separator())
            let profiles = NSMenuItem(title: s.active_profile.map { "Profile: \($0)" } ?? "Profiles", action: nil, keyEquivalent: "")
            let sub = NSMenu()
            for p in s.profiles {
                let i = item(p, #selector(applyProfile(_:)), obj: p)
                i.state = s.active_profile == p ? .on : .off
                i.toolTip = s.active_profile == p ? "Active: re-apply to reset the layout to this profile" : "Apply"
                sub.addItem(i)
            }
            if !s.profiles.isEmpty { sub.addItem(.separator()) }
            sub.addItem(item("Save Current Desk as Profile…", #selector(saveProfile)))
            sub.addItem(item("Edit Profiles…", #selector(editProfiles)))
            profiles.submenu = sub
            menu.addItem(profiles)
        } else {
            let i = item("Pane is starting…", nil)
            i.isEnabled = false
            menu.addItem(i)
        }
        menu.addItem(item("Restore All Windows", #selector(recover)))
        menu.addItem(item("Reload Config", #selector(reload)))
        menu.addItem(.separator())
        menu.addItem(item("Profile Editor…", #selector(editProfiles), key: "e"))
        menu.addItem(item("Settings…", #selector(settings), key: ","))
        menu.addItem(.separator())
        menu.addItem(item("Quit Pane", #selector(quit), key: "q"))
    }

    /// A small menu of profiles at the mouse (the profile picker hotkey).
    func popUpProfilePicker() {
        let m = NSMenu()
        for p in PaneCore.shared.snapshot?.profiles ?? [] {
            m.addItem(item(p, #selector(applyProfile(_:)), obj: p))
        }
        if m.items.isEmpty {
            let i = item("No profiles yet", nil)
            i.isEnabled = false
            m.addItem(i)
        }
        m.popUp(positioning: nil, at: NSEvent.mouseLocation, in: nil)
    }

    @objc private func showWorkspace(_ sender: NSMenuItem) {
        if let n = sender.representedObject as? String { PaneCore.shared.showWorkspace(n) }
    }

    @objc private func applyProfile(_ sender: NSMenuItem) {
        guard let n = sender.representedObject as? String else { return }
        if case let .error(m) = PaneCore.shared.applyProfile(named: n) { NoticeHUD.shared.show(m) }
    }

    @objc private func saveProfile() {
        let a = NSAlert()
        a.messageText = "Save current desk as profile"
        a.informativeText = "Which app is in which pane on which display."
        let field = NSTextField(frame: NSRect(x: 0, y: 0, width: 240, height: 24))
        field.placeholderString = "Profile name"
        a.accessoryView = field
        a.addButton(withTitle: "Save")
        a.addButton(withTitle: "Cancel")
        NSApp.activate(ignoringOtherApps: true)
        guard a.runModal() == .alertFirstButtonReturn else { return }
        let name = field.stringValue.trimmingCharacters(in: .whitespaces)
        guard !name.isEmpty else { return }
        switch PaneCore.shared.capture(name: name) {
        case let .success(p):
            if case let .error(m) = PaneCore.shared.save(p) { NoticeHUD.shared.show(m) } else { NoticeHUD.shared.show("Saved “\(name)”") }
        case let .failure(e):
            NoticeHUD.shared.show(e.message)
        }
    }

    @objc private func editProfiles() { openEditor() }
    @objc private func settings() { openSettings() }
    @objc private func recover() { PaneCore.shared.recover() }
    @objc private func grantAccess() { Permissions.explain() }

    @objc private func reload() {
        if case let .error(m) = PaneCore.shared.reload() { NoticeHUD.shared.show("Config error: \(m)") }
    }

    @objc private func quit() { NSApp.terminate(nil) }
}
