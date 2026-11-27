import AppKit

/// The status item's menu, rebuilt from the latest snapshot each time it opens.
@MainActor
final class StatusMenu: NSObject, NSMenuDelegate {
    let menu = NSMenu()
    var openEditor: () -> Void = {}
    var openSettings: () -> Void = {}
    /// Show how the desk differs from a saved profile, offering to update it.
    var reviewDrift: (String) -> Void = { _ in }

    override init() {
        super.init()
        menu.delegate = self
        menu.autoenablesItems = false
    }

    func menuNeedsUpdate(_ menu: NSMenu) {
        rebuild()
    }

    private func item(_ title: String, _ action: Selector?, key: String = "", obj: Any? = nil, symbol: String? = nil) -> NSMenuItem {
        let i = NSMenuItem(title: title, action: action, keyEquivalent: key)
        i.target = self
        i.representedObject = obj
        if let symbol { i.image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil) }
        return i
    }

    func rebuild() {
        menu.removeAllItems()
        let snap = PaneCore.shared.snapshot
        if let r = UpdateChecker.shared.available {
            menu.addItem(item("Pane \(r.version) Is Available…", #selector(showUpdate), symbol: "arrow.down.circle"))
            menu.addItem(.separator())
        }
        if let s = snap {
            if !s.ax_trusted {
                menu.addItem(item("Grant Accessibility Access…", #selector(grantAccess), symbol: "exclamationmark.triangle"))
                menu.addItem(.separator())
            }
            if let err = s.config_error {
                let e = item("Config error: \(err)", nil, symbol: "exclamationmark.triangle")
                e.isEnabled = false
                menu.addItem(e)
                menu.addItem(.separator())
            }
            for d in s.displays.sorted(by: { $0.rank < $1.rank }) {
                menu.addItem(.sectionHeader(title: d.focused && s.displays.count > 1 ? "\(d.name) · Focused" : d.name))
                for w in s.workspaces where w.display == d.id {
                    let name = w.title.map { "\(w.label) · \($0)" } ?? w.label
                    let i = item(name, #selector(showWorkspace(_:)), obj: w.target)
                    i.state = w.visible ? .on : .off
                    if w.windows > 0 { i.badge = NSMenuItemBadge(count: w.windows) }
                    menu.addItem(i)
                }
            }
            menu.addItem(.separator())
            let pads = item("Scratchpads", nil, symbol: "rectangle.on.rectangle")
            let padMenu = NSMenu()
            for sp in s.scratchpads ?? [] {
                let i = item(sp.name, #selector(toggleScratchpad(_:)), obj: sp.name)
                i.state = sp.shown ? .on : .off
                if let k = sp.key { i.toolTip = "Key: \(k)" }
                padMenu.addItem(i)
            }
            if padMenu.items.isEmpty {
                let i = item("None — add a [[scratchpad]] to config.toml", nil)
                i.isEnabled = false
                padMenu.addItem(i)
            }
            pads.submenu = padMenu
            menu.addItem(pads)
            let summon = item("Summon", nil, symbol: "arrow.down.right.and.arrow.up.left")
            let summonMenu = NSMenu()
            let apps = NSWorkspace.shared.runningApplications
                .filter { $0.activationPolicy == .regular && $0.bundleIdentifier != Bundle.main.bundleIdentifier }
                .compactMap { a in a.bundleIdentifier.map { (id: $0, name: a.localizedName ?? $0) } }
                .sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
            for a in apps {
                let i = item(a.name, #selector(summonApp(_:)), obj: a.id)
                i.image = AppIcons.small(for: a.id)
                summonMenu.addItem(i)
            }
            summon.submenu = summonMenu
            summon.toolTip = "Bring an app's window to this workspace, next to the focused tile"
            menu.addItem(summon)
            let drifted = s.profile_drift == true
            let profileTitle = s.active_profile.map { "Profile: \($0)" + (drifted ? "  •" : "") } ?? "Profiles"
            let profiles = item(profileTitle, nil, symbol: "person.crop.rectangle.stack")
            let sub = NSMenu()
            let desk = s.desk_profiles ?? []
            for p in s.profiles {
                let i = item(desk.contains(p) ? "\(p)  (these displays)" : p, #selector(applyProfile(_:)), obj: p)
                i.state = s.active_profile == p ? .on : .off
                i.toolTip = s.active_profile == p ? "Active: re-apply to reset the layout to this profile" : "Apply"
                sub.addItem(i)
            }
            if !s.profiles.isEmpty { sub.addItem(.separator()) }
            if let a = s.active_profile, s.profiles.contains(a) {
                let u = item("Update “\(a)” from Current Desk…", #selector(reviewProfileDrift(_:)), obj: a)
                u.toolTip = drifted
                    ? "The desk no longer matches “\(a)”. Compare them side by side, then save"
                    : "The desk matches “\(a)”; new workspaces with windows can still be added"
                sub.addItem(u)
                let i = item("Apply “\(a)” on These Displays Automatically", #selector(toggleProfileAuto(_:)), obj: a)
                i.state = desk.contains(a) ? .on : .off
                i.toolTip = "Pane applies it by itself whenever exactly these displays are connected"
                sub.addItem(i)
            }
            sub.addItem(item("Save Current Desk as Profile…", #selector(saveProfile)))
            sub.addItem(item("Edit Profiles…", #selector(editProfiles)))
            profiles.submenu = sub
            menu.addItem(profiles)
        } else {
            let i = item("Pane is starting…", nil)
            i.isEnabled = false
            menu.addItem(i)
        }
        menu.addItem(item("Go to Window…", #selector(openPalette), symbol: "magnifyingglass"))
        menu.addItem(item("Restore All Windows", #selector(recover), symbol: "arrow.uturn.backward.circle"))
        menu.addItem(item("Reload Config", #selector(reload), symbol: "arrow.clockwise"))
        menu.addItem(.separator())
        menu.addItem(item("Profile Editor…", #selector(editProfiles), key: "e", symbol: "square.grid.2x2"))
        menu.addItem(item("About Pane", #selector(about), symbol: "info.circle"))
        menu.addItem(item("Settings…", #selector(settings), key: ",", symbol: "gearshape"))
        menu.addItem(item("Check for Updates…", #selector(checkForUpdates), symbol: "arrow.down.circle"))
        menu.addItem(.separator())
        menu.addItem(item("Quit Pane", #selector(quit), key: "q", symbol: "power"))
    }

    @objc private func openPalette() {
        // After the menu has closed, so the panel can take key focus.
        DispatchQueue.main.async { MainActor.assumeIsolated { CommandPalette.shared.show() } }
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

    @objc private func summonApp(_ sender: NSMenuItem) {
        guard let id = sender.representedObject as? String else { return }
        if case let .error(m) = PaneCore.shared.send(["cmd": "summon", "app": id]) { NoticeHUD.shared.show(m, symbol: NoticeHUD.errorSymbol) }
    }

    @objc private func toggleScratchpad(_ sender: NSMenuItem) {
        guard let n = sender.representedObject as? String else { return }
        if case let .error(m) = PaneCore.shared.send(["cmd": "scratchpad", "name": n]) { NoticeHUD.shared.show(m, symbol: NoticeHUD.errorSymbol) }
    }

    @objc private func applyProfile(_ sender: NSMenuItem) {
        guard let n = sender.representedObject as? String else { return }
        if case let .error(m) = PaneCore.shared.applyProfile(named: n) { NoticeHUD.shared.show(m, symbol: NoticeHUD.errorSymbol) }
    }

    @objc private func toggleProfileAuto(_ sender: NSMenuItem) {
        guard let n = sender.representedObject as? String else { return }
        let on = sender.state != .on
        switch PaneCore.shared.setProfileAuto(n, here: on) {
        case let .error(m): NoticeHUD.shared.show(m, symbol: NoticeHUD.errorSymbol)
        case .ok: NoticeHUD.shared.show(on ? "“\(n)” applies on these displays" : "“\(n)” applies only by hand", symbol: "checkmark.circle")
        }
    }

    @objc private func reviewProfileDrift(_ sender: NSMenuItem) {
        guard let n = sender.representedObject as? String else { return }
        reviewDrift(n)
    }

    @objc private func saveProfile() {
        let a = NSAlert()
        a.icon = NSApp.applicationIconImage
        a.messageText = "Save Current Desk as Profile"
        a.informativeText = "Saves which app is in which tile on which display."
        let field = NSTextField(frame: NSRect(x: 0, y: 0, width: 260, height: 24))
        field.placeholderString = "Profile name"
        a.accessoryView = field
        let save = a.addButton(withTitle: "Save")
        a.addButton(withTitle: "Cancel")
        // Save stays off until there is a name.
        save.isEnabled = false
        let guardian = NonEmptyFieldGuard(button: save)
        field.delegate = guardian
        a.window.initialFirstResponder = field
        NSApp.activate()
        let response = a.runModal()
        withExtendedLifetime(guardian) {}
        guard response == .alertFirstButtonReturn else { return }
        let name = field.stringValue.trimmingCharacters(in: .whitespaces)
        guard !name.isEmpty else { return }
        switch PaneCore.shared.capture(name: name) {
        case let .success(p):
            if case let .error(m) = PaneCore.shared.save(p) {
                NoticeHUD.shared.show(m, symbol: NoticeHUD.errorSymbol)
            } else {
                NoticeHUD.shared.show("Saved “\(name)”", symbol: "checkmark.circle")
            }
        case let .failure(e):
            NoticeHUD.shared.show(e.message, symbol: NoticeHUD.errorSymbol)
        }
    }

    @objc private func editProfiles() { openEditor() }
    @objc private func settings() { openSettings() }
    @objc private func checkForUpdates() { UpdateChecker.shared.check(userInitiated: true) }

    @objc private func showUpdate() {
        if let r = UpdateChecker.shared.available { UpdateChecker.shared.offer(r) }
    }

    @objc private func recover() { PaneCore.shared.recover() }
    @objc private func grantAccess() { WelcomeWindow.shared.show() }

    @objc private func about() {
        NSApp.activate()
        NSApp.orderFrontStandardAboutPanel(nil)
    }

    @objc private func reload() {
        if case let .error(m) = PaneCore.shared.reload() { NoticeHUD.shared.show("Config error: \(m)", symbol: NoticeHUD.errorSymbol) }
    }

    @objc private func quit() { NSApp.terminate(nil) }
}

/// Enables a button only while a text field holds something besides spaces.
@MainActor
private final class NonEmptyFieldGuard: NSObject, NSTextFieldDelegate {
    weak var button: NSButton?

    init(button: NSButton) {
        self.button = button
    }

    func controlTextDidChange(_ obj: Notification) {
        guard let f = obj.object as? NSTextField else { return }
        button?.isEnabled = !f.stringValue.trimmingCharacters(in: .whitespaces).isEmpty
    }
}
