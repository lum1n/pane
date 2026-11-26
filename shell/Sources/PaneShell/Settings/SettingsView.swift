import AppKit
import Combine
import SwiftUI

/// Mirror of `config::Settings` in crates/pane-core.
struct PaneSettings: Codable, Equatable {
    var inner_gap: Double
    var outer_gap: Double
    var default_layout: String
    var placement_memory_days: Int
    var restore_placements: Bool
    var return_workspaces_on_reconnect: Bool
    var startup_profile: String?
    var electron_bundle_ids: [String]
    var resize_step: Double
    var auto_float_below: Double
    var modifier: String
}

private struct ConfigReply: Codable {
    var settings: PaneSettings
}

/// `config::BindingInfo`.
struct BindingRow: Codable, Equatable, Identifiable {
    var command: String
    var chords: [String]
    var `default`: String?
    var id: String { command }
}

/// Settings state. Every change is sent to the core, which edits
/// config.toml (keeping comments), validates it and reloads.
@MainActor
final class SettingsModel: ObservableObject {
    @Published var settings: PaneSettings?
    @Published var bindings: [BindingRow] = []
    @Published var configError: String?
    @Published var message: String?
    @Published var recording: String?
    private var keyNames: [UInt16: String] = [:]
    private var monitor: Any?

    func load() {
        configError = PaneCore.shared.snapshot?.config_error
        switch PaneCore.shared.request(["cmd": "query", "what": "config"], as: ConfigReply.self) {
        case let .success(c): settings = c.settings
        case let .failure(e): configError = e.message
        }
        if case let .success(rows) = PaneCore.shared.request(["cmd": "query", "what": "bindings"], as: [BindingRow].self) {
            bindings = rows
        }
        if keyNames.isEmpty,
           case let .success(keys) = PaneCore.shared.request(["cmd": "query", "what": "keys"], as: [String: UInt16].self) {
            keyNames = Dictionary(keys.map { ($0.value, $0.key) }, uniquingKeysWith: { a, _ in a })
        }
    }

    private func finish(_ r: Reply, ok: String? = nil) {
        switch r {
        case .ok:
            configError = nil
            message = ok
        case let .error(m):
            message = m
        }
        load()
    }

    func set(_ key: String, _ value: Any) {
        finish(PaneCore.shared.send(["cmd": "set-setting", "key": key, "value": value]))
    }

    func clear(_ key: String) {
        finish(PaneCore.shared.send(["cmd": "set-setting", "key": key, "value": NSNull()]))
    }

    func reload() {
        finish(PaneCore.shared.reload(), ok: "Reloaded")
    }

    func rebind(_ command: String, to chord: String?) {
        let thief = chord.flatMap { c in bindings.first { $0.command != command && $0.chords.contains(c) } }
        var cmd: [String: Any] = ["cmd": "rebind", "command": command]
        if let chord { cmd["chord"] = chord }
        let note = thief.map { "\(Shortcut.pretty(chord!)) was taken from “\($0.command)”" }
        finish(PaneCore.shared.send(cmd), ok: note)
    }

    func resetAll() {
        finish(PaneCore.shared.send(["cmd": "reset-shortcuts"]), ok: "All shortcuts reset to the defaults")
    }

    // MARK: recording

    func startRecording(_ command: String) {
        stopRecording()
        recording = command
        message = "Press the new shortcut for “\(command)” (Esc cancels)"
        PaneCore.shared.send(["cmd": "suspend-hotkeys", "suspended": true])
        monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            MainActor.assumeIsolated { self?.handle(event) }
            return nil
        }
    }

    func stopRecording() {
        if let m = monitor { NSEvent.removeMonitor(m) }
        monitor = nil
        if recording != nil {
            recording = nil
            PaneCore.shared.send(["cmd": "suspend-hotkeys", "suspended": false])
        }
    }

    private func handle(_ event: NSEvent) {
        guard let command = recording else { return }
        let f = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        if event.keyCode == 0x35, f.intersection([.command, .control, .option]).isEmpty {
            stopRecording()
            message = nil
            return
        }
        guard let key = keyNames[event.keyCode] else {
            message = "That key can't be used for shortcuts"
            return
        }
        guard !f.intersection([.command, .control, .option]).isEmpty else {
            message = "Include ⌘, ⌃ or ⌥ (Esc cancels)"
            return
        }
        var parts: [String] = []
        if f.contains(.command) { parts.append("cmd") }
        if f.contains(.control) { parts.append("ctrl") }
        if f.contains(.option) { parts.append("alt") }
        if f.contains(.shift) { parts.append("shift") }
        parts.append(key)
        stopRecording()
        rebind(command, to: parts.joined(separator: "-"))
    }
}

/// Display helpers for chords like `ctrl-alt-shift-h`.
enum Shortcut {
    static func pretty(_ chord: String) -> String {
        var parts = chord.split(separator: "-").map(String.init)
        guard let key = parts.popLast() else { return chord }
        let symbols: [String: String] = ["cmd": "⌘", "ctrl": "⌃", "alt": "⌥", "shift": "⇧"]
        let order = ["ctrl", "alt", "shift", "cmd"]
        let mods = order.filter { parts.contains($0) }.compactMap { symbols[$0] }.joined()
        return mods + prettyKey(key)
    }

    static func prettyKey(_ k: String) -> String {
        let named: [String: String] = [
            "return": "↩", "tab": "⇥", "space": "Space", "escape": "⎋", "backspace": "⌫",
            "comma": ",", "period": ".", "slash": "/", "semicolon": ";", "quote": "'",
            "leftbracket": "[", "rightbracket": "]", "minus": "-", "equal": "=", "backslash": "\\",
            "grave": "`", "left": "←", "right": "→", "up": "↑", "down": "↓",
        ]
        return named[k] ?? k.uppercased()
    }

    /// Parse a modifier setting into flags (cmd, ctrl, alt, shift).
    static func flags(_ modifier: String) -> (cmd: Bool, ctrl: Bool, alt: Bool, shift: Bool) {
        var r = (cmd: false, ctrl: false, alt: false, shift: false)
        for t in modifier.lowercased().split(whereSeparator: { "-+ ".contains($0) }) {
            switch t {
            case "cmd", "command", "super": r.cmd = true
            case "ctrl", "control": r.ctrl = true
            case "alt", "opt", "option": r.alt = true
            case "shift": r.shift = true
            case "hyper": r = (true, true, true, true)
            case "meh": r = (r.cmd, true, true, true)
            default: break
            }
        }
        return r
    }
}

@MainActor
struct SettingsView: View {
    @StateObject private var model = SettingsModel()

    var body: some View {
        TabView {
            GeneralSettingsView(model: model)
                .tabItem { Label("General", systemImage: "gearshape") }
            HotkeySettingsView(model: model)
                .tabItem { Label("Hotkeys", systemImage: "keyboard") }
        }
        .padding(8)
        .frame(width: 620, height: 640)
        .onAppear { model.load() }
        .onDisappear { model.stopRecording() }
        // The window is reused; refresh whenever it comes forward.
        .onReceive(NotificationCenter.default.publisher(for: NSWindow.didBecomeKeyNotification)) { _ in
            if model.recording == nil { model.load() }
        }
    }
}

@MainActor
private struct GeneralSettingsView: View {
    @ObservedObject var model: SettingsModel
    @State private var axGranted = false
    @State private var electron = ""

    private var supportDir: URL {
        if let d = ProcessInfo.processInfo.environment["PANE_SUPPORT_DIR"] { return URL(fileURLWithPath: d) }
        return FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/Pane")
    }

    private var configURL: URL { supportDir.appendingPathComponent("config.toml") }

    private func number(_ key: String, _ get: @escaping (PaneSettings) -> Double) -> Binding<Double> {
        Binding(get: { model.settings.map(get) ?? 0 }, set: { model.set(key, $0) })
    }

    private func toggle(_ key: String, _ get: @escaping (PaneSettings) -> Bool) -> Binding<Bool> {
        Binding(get: { model.settings.map(get) ?? false }, set: { model.set(key, $0) })
    }

    var body: some View {
        Form {
            if let s = model.settings {
                Section("Layout") {
                    TextField("Gap between windows", value: number("inner_gap") { $0.inner_gap }, format: .number)
                    TextField("Gap at screen edges", value: number("outer_gap") { $0.outer_gap }, format: .number)
                    Picker("New containers", selection: Binding(
                        get: { s.default_layout },
                        set: { model.set("default_layout", $0) }
                    )) {
                        Text("Tiles").tag("tiles")
                        Text("Accordion").tag("accordion")
                    }
                    TextField("Resize step (%)", value: Binding(
                        get: { (model.settings?.resize_step ?? 0.05) * 100 },
                        set: { model.set("resize_step", $0 / 100) }
                    ), format: .number)
                    TextField("Float windows smaller than (pt)", value: number("auto_float_below") { $0.auto_float_below }, format: .number)
                }
                Section("Behaviour") {
                    Toggle("Put relaunched windows back in their tiles", isOn: toggle("restore_placements") { $0.restore_placements })
                    TextField("Remember window placements for (days)", value: Binding(
                        get: { Double(model.settings?.placement_memory_days ?? 30) },
                        set: { model.set("placement_memory_days", Int($0)) }
                    ), format: .number)
                    Toggle("Return workspaces when a display reconnects", isOn: toggle("return_workspaces_on_reconnect") { $0.return_workspaces_on_reconnect })
                    Picker("Startup profile", selection: Binding(
                        get: { s.startup_profile ?? "" },
                        set: { $0.isEmpty ? model.clear("startup_profile") : model.set("startup_profile", $0) }
                    )) {
                        Text("None").tag("")
                        ForEach(PaneCore.shared.snapshot?.profiles ?? [], id: \.self) { Text($0).tag($0) }
                    }
                }
                Section("Electron apps") {
                    TextField("Bundle IDs, comma separated", text: $electron)
                        .onSubmit {
                            let ids = electron.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
                            model.set("electron_bundle_ids", ids)
                        }
                    Text("These apps get an extra resize pass so they land exactly. Press Return to save.")
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
            Section("Configuration file") {
                LabeledContent("File") {
                    Text(configURL.path).textSelection(.enabled).font(.system(.caption, design: .monospaced))
                }
                if let e = model.configError {
                    Label(e, systemImage: "exclamationmark.triangle").foregroundStyle(.red)
                }
                HStack {
                    Button("Open in Editor") { NSWorkspace.shared.open(configURL) }
                    Button("Reload") { model.reload() }
                    Button("Show Folder") { NSWorkspace.shared.activateFileViewerSelecting([configURL]) }
                }
                Text("Displays, workspaces and window rules are edited in the file.")
                    .font(.caption).foregroundStyle(.secondary)
                if let m = model.message { Text(m).foregroundStyle(.secondary) }
            }
            Section("Permissions") {
                LabeledContent("Accessibility") {
                    Text(axGranted ? "Granted" : "Not granted").foregroundStyle(axGranted ? .green : .red)
                }
                if !axGranted {
                    Button("Open Accessibility Settings") { Permissions.openAccessibilitySettings() }
                }
            }
            Section("Recovery") {
                Text("If windows ever go missing, use “Restore All Windows” in the menu or run `pane recover --all-apps`.")
                    .foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
        .onAppear {
            axGranted = Permissions.accessibilityGranted
            electron = model.settings?.electron_bundle_ids.joined(separator: ", ") ?? ""
        }
        .onChange(of: model.settings) { _, s in
            electron = s?.electron_bundle_ids.joined(separator: ", ") ?? ""
        }
    }
}

@MainActor
private struct HotkeySettingsView: View {
    @ObservedObject var model: SettingsModel

    private func modifierToggle(_ label: String, _ which: WritableKeyPath<ModFlags, Bool>) -> some View {
        let current = ModFlags(Shortcut.flags(model.settings?.modifier ?? "alt"))
        return Toggle(label, isOn: Binding(
            get: { current[keyPath: which] },
            set: { on in
                var f = current
                f[keyPath: which] = on
                guard f.cmd || f.ctrl || f.alt else {
                    model.message = "The modifier needs at least one of ⌘, ⌃ or ⌥"
                    return
                }
                model.set("modifier", f.setting)
            }
        ))
        .toggleStyle(.checkbox)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            GroupBox("Modifier") {
                VStack(alignment: .leading, spacing: 6) {
                    HStack(spacing: 16) {
                        modifierToggle("⌃ Control", \.ctrl)
                        modifierToggle("⌥ Option", \.alt)
                        modifierToggle("⇧ Shift", \.shift)
                        modifierToggle("⌘ Command", \.cmd)
                    }
                    Text("Every default shortcut uses this. Moving adds ⇧ and resizing adds ⌃ (or the next key not already in the modifier). Shortcuts you set below follow the modifier when they include it.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .padding(4)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            List(model.bindings) { row in
                HStack {
                    Text(row.command)
                    Spacer()
                    Button {
                        if model.recording == row.command { model.stopRecording() } else { model.startRecording(row.command) }
                    } label: {
                        Text(model.recording == row.command ? "Type shortcut…" : (row.chords.first.map(Shortcut.pretty) ?? "—"))
                            .font(.system(.body, design: .rounded))
                            .frame(minWidth: 110)
                    }
                    .help("Click, then press the new shortcut")
                    Button { model.rebind(row.command, to: nil) } label: { Image(systemName: "xmark.circle") }
                        .buttonStyle(.borderless)
                        .help("Remove the shortcut")
                        .disabled(row.chords.isEmpty)
                    Button { model.rebind(row.command, to: row.default) } label: { Image(systemName: "arrow.counterclockwise") }
                        .buttonStyle(.borderless)
                        .help(row.default.map { "Reset to \(Shortcut.pretty($0))" } ?? "No default")
                        .disabled(row.default == nil || row.chords == [row.default!])
                }
            }
            HStack {
                if let m = model.message { Text(m).font(.callout).foregroundStyle(.secondary) }
                Spacer()
                Button("Reset All Shortcuts") { model.resetAll() }
            }
        }
        .padding(8)
    }
}

/// Mutable modifier flags for the checkboxes.
struct ModFlags {
    var cmd = false, ctrl = false, alt = false, shift = false

    init(_ t: (cmd: Bool, ctrl: Bool, alt: Bool, shift: Bool)) {
        cmd = t.cmd
        ctrl = t.ctrl
        alt = t.alt
        shift = t.shift
    }

    var setting: String {
        var p: [String] = []
        if cmd { p.append("cmd") }
        if ctrl { p.append("ctrl") }
        if alt { p.append("alt") }
        if shift { p.append("shift") }
        return p.joined(separator: "-")
    }
}
