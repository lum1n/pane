import AppKit
import Combine
import SwiftUI
import UniformTypeIdentifiers

/// Mirror of `config::Settings` in crates/pane-core.
struct PaneSettings: Codable, Equatable {
    var inner_gap: Double
    var outer_gap: Double
    var smart_gaps: Bool
    var default_layout: String
    var placement_memory_days: Int
    var restore_placements: Bool
    var return_workspaces_on_reconnect: Bool
    var startup_profile: String?
    var electron_bundle_ids: [String]
    var resize_step: Double
    var auto_float_below: Double
    var modifier: String
    var float_apps: [String]
    var tile_apps: [String]
    var gestures: Bool
    var gesture_modifier: String
    var border_width: Double
    var border_color: String
    var dim_unfocused: Double
    var menu_bar_app_icons: Bool?
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
    /// The scratchpads being edited; `scratchpadsDirty` while they differ
    /// from config.toml.
    @Published var scratchpads: [SnapScratchpad] = []
    @Published var scratchpadsDirty = false
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
        if !scratchpadsDirty { scratchpads = PaneCore.shared.snapshot?.scratchpads ?? [] }
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

    /// Write the edited scratchpads to config.toml. The name is only
    /// written when it differs from the app.
    func applyScratchpads() {
        let rows: [[String: Any]] = scratchpads.map { sp in
            func clean(_ s: String?) -> String? {
                let t = s?.trimmingCharacters(in: .whitespaces) ?? ""
                return t.isEmpty ? nil : t
            }
            var d: [String: Any] = ["app": sp.app.trimmingCharacters(in: .whitespaces)]
            if let n = clean(sp.name), n != sp.app { d["name"] = n }
            if let k = clean(sp.key) { d["key"] = k }
            if let z = clean(sp.size) { d["size"] = z }
            return d
        }
        let r = PaneCore.shared.send(["cmd": "set-scratchpads", "scratchpads": rows])
        if case .ok = r { scratchpadsDirty = false }
        finish(r, ok: "Scratchpads saved")
    }

    func revertScratchpads() {
        scratchpadsDirty = false
        message = nil
        load()
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
            AppsSettingsView(model: model)
                .tabItem { Label("Apps", systemImage: "square.on.square") }
            ScratchpadSettingsView(model: model)
                .tabItem { Label("Scratchpads", systemImage: "rectangle.on.rectangle") }
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
    @State private var dim = 0.0

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

    static func isAccent(_ c: String) -> Bool {
        let t = c.trimmingCharacters(in: .whitespaces).lowercased()
        return t.isEmpty || t == "accent"
    }

    /// "#rgb", "#rrggbb" or "#rrggbbaa" -> Color (anything else: orange).
    static func color(hex: String) -> Color {
        var h = hex.trimmingCharacters(in: .whitespaces)
        if h.hasPrefix("#") { h.removeFirst() }
        if h.count == 3 { h = h.map { "\($0)\($0)" }.joined() }
        guard h.count == 6 || h.count == 8, let v = UInt64(h, radix: 16) else { return .orange }
        let n = h.count == 8 ? v : (v << 8 | 0xff)
        return Color(.sRGB,
                     red: Double((n >> 24) & 0xff) / 255,
                     green: Double((n >> 16) & 0xff) / 255,
                     blue: Double((n >> 8) & 0xff) / 255,
                     opacity: Double(n & 0xff) / 255)
    }

    /// Color -> "#rrggbbaa" in sRGB.
    static func hex(_ c: Color) -> String {
        let ns = NSColor(c).usingColorSpace(.sRGB) ?? .orange
        let b = { (x: CGFloat) in Int((min(max(x, 0), 1) * 255).rounded()) }
        return String(format: "#%02x%02x%02x%02x", b(ns.redComponent), b(ns.greenComponent), b(ns.blueComponent), b(ns.alphaComponent))
    }

    var body: some View {
        Form {
            if let s = model.settings {
                Section("Layout") {
                    TextField("Gap between windows", value: number("inner_gap") { $0.inner_gap }, format: .number)
                    TextField("Gap at screen edges", value: number("outer_gap") { $0.outer_gap }, format: .number)
                    Toggle("No gaps when a workspace shows one tile", isOn: toggle("smart_gaps") { $0.smart_gaps })
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
                Section("Focus") {
                    TextField("Border around the focused window (pt, 0 = off)", value: number("border_width") { $0.border_width }, format: .number)
                    Toggle("Use the system accent colour", isOn: Binding(
                        get: { Self.isAccent(model.settings?.border_color ?? "accent") },
                        set: { model.set("border_color", $0 ? "accent" : "#ff8000") }
                    ))
                    if !Self.isAccent(s.border_color) {
                        ColorPicker("Border colour", selection: Binding(
                            get: { Self.color(hex: s.border_color) },
                            set: { model.set("border_color", Self.hex($0)) }
                        ))
                    }
                    LabeledContent("Dim other windows") {
                        Slider(value: $dim, in: 0...0.8) { editing in
                            if !editing { model.set("dim_unfocused", (dim * 100).rounded() / 100) }
                        }
                        .frame(maxWidth: 220)
                    }
                    .onAppear { dim = s.dim_unfocused }
                    .onChange(of: s.dim_unfocused) { _, v in dim = v }
                }
                Section("Menu bar") {
                    Toggle("Show app icons on the tiles", isOn: Binding(
                        get: { model.settings?.menu_bar_app_icons ?? true },
                        set: { model.set("menu_bar_app_icons", $0) }
                    ))
                    Text("Click a tile in the menu bar icon to focus its window. Name workspaces in config.toml under [workspace_names].")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Section("Behaviour") {
                    Toggle("Put relaunched windows back in their tiles", isOn: toggle("restore_placements") { $0.restore_placements })
                    TextField("Remember window placements for (days)", value: Binding(
                        get: { Double(model.settings?.placement_memory_days ?? 30) },
                        set: { model.set("placement_memory_days", Int($0)) }
                    ), format: .number)
                    Picker("Default profile", selection: Binding(
                        get: { s.startup_profile ?? "" },
                        set: { $0.isEmpty ? model.clear("startup_profile") : model.set("startup_profile", $0) }
                    )) {
                        Text("None").tag("")
                        ForEach(PaneCore.shared.snapshot?.profiles ?? [], id: \.self) { Text($0).tag($0) }
                    }
                    Text("Applied at launch when no profile is active. A profile you apply stays active, also across restarts, until you apply another.")
                        .font(.caption).foregroundStyle(.secondary)
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
                    Toggle("Move windows by holding the modifier and pointing", isOn: Binding(
                        get: { model.settings?.gestures ?? true },
                        set: { model.set("gestures", $0) }
                    ))
                    Text("Hold the modifier over a window and point at another tile, its edge, a screen edge or another display; the outline shows where it goes. Release to drop, Esc cancels.")
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

/// An app Pane can name: bundle id, or executable name for unbundled apps.
struct KnownApp: Hashable {
    var id: String
    var name: String
}

private struct AppSummaryRow: Codable {
    var id: String
    var name: String
}

@MainActor
private struct AppsSettingsView: View {
    @ObservedObject var model: SettingsModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            AppListEditor(
                title: "Always float",
                help: "New windows of these apps start floating. Adding an app also floats its open windows.",
                apps: model.settings?.float_apps ?? [],
                onChange: { model.set("float_apps", $0) }
            )
            AppListEditor(
                title: "Always tile",
                help: "For apps Pane floats on its own (unusual windows). Windows that cannot be resized still float.",
                apps: model.settings?.tile_apps ?? [],
                onChange: { model.set("tile_apps", $0) }
            )
            HStack {
                if let m = model.message { Text(m).font(.callout).foregroundStyle(.secondary) }
                Spacer()
            }
            Text("Apps started from a terminal (like `cargo run`) are listed by their executable name. `pane query windows` shows why each window is tiled or floating.")
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(8)
    }
}

@MainActor
private struct AppListEditor: View {
    let title: String
    let help: String
    let apps: [String]
    let onChange: ([String]) -> Void
    @State private var manual = ""

    /// Running apps: what Pane observes (includes unbundled ones) plus
    /// regular apps from NSWorkspace.
    private var running: [KnownApp] {
        var seen = Set<String>()
        var out: [KnownApp] = []
        if case let .success(rows) = PaneCore.shared.request(["cmd": "query", "what": "apps"], as: [AppSummaryRow].self) {
            for r in rows where !seen.contains(r.id) {
                seen.insert(r.id)
                out.append(KnownApp(id: r.id, name: r.name))
            }
        }
        for a in NSWorkspace.shared.runningApplications where a.activationPolicy == .regular {
            guard let b = a.bundleIdentifier, !seen.contains(b) else { continue }
            seen.insert(b)
            out.append(KnownApp(id: b, name: a.localizedName ?? b))
        }
        return out.filter { !apps.contains($0.id) }
            .sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
    }

    private func add(_ id: String) {
        let id = id.trimmingCharacters(in: .whitespaces)
        guard !id.isEmpty, !apps.contains(id) else { return }
        onChange(apps + [id])
    }

    private func chooseApplication() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.application]
        panel.directoryURL = URL(fileURLWithPath: "/Applications")
        panel.allowsMultipleSelection = true
        guard panel.runModal() == .OK else { return }
        var list = apps
        for url in panel.urls {
            if let b = Bundle(url: url)?.bundleIdentifier, !list.contains(b) { list.append(b) }
        }
        if list != apps { onChange(list) }
    }

    var body: some View {
        GroupBox(title) {
            VStack(alignment: .leading, spacing: 6) {
                Text(help).font(.caption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                if apps.isEmpty {
                    Text("No apps").foregroundStyle(.secondary).padding(.vertical, 4)
                } else {
                    ForEach(apps, id: \.self) { id in
                        HStack(spacing: 8) {
                            Image(nsImage: AppIcons.icon(for: id)).resizable().frame(width: 18, height: 18)
                            Text(AppIcons.name(for: id))
                            Text(id).font(.caption).foregroundStyle(.secondary)
                            Spacer()
                            Button { onChange(apps.filter { $0 != id }) } label: { Image(systemName: "minus.circle") }
                                .buttonStyle(.borderless)
                                .help("Remove")
                        }
                    }
                }
                HStack {
                    Menu("Add Running App") {
                        ForEach(running, id: \.self) { a in
                            Button("\(a.name)  (\(a.id))") { add(a.id) }
                        }
                    }
                    .frame(width: 170)
                    Button("Choose Application…") { chooseApplication() }
                    TextField("or type an id / glob", text: $manual)
                        .textFieldStyle(.roundedBorder)
                        .onSubmit {
                            add(manual)
                            manual = ""
                        }
                }
            }
            .padding(4)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

@MainActor
private struct ScratchpadSettingsView: View {
    @ObservedObject var model: SettingsModel

    private func running() -> [KnownApp] {
        NSWorkspace.shared.runningApplications
            .filter { $0.activationPolicy == .regular }
            .compactMap { a in a.bundleIdentifier.map { KnownApp(id: $0, name: a.localizedName ?? $0) } }
            .sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
    }

    private func edit(_ i: Int, _ change: (inout SnapScratchpad) -> Void) {
        guard model.scratchpads.indices.contains(i) else { return }
        change(&model.scratchpads[i])
        model.scratchpadsDirty = true
    }

    private func text(_ i: Int, _ get: @escaping (SnapScratchpad) -> String?, _ set: @escaping (inout SnapScratchpad, String) -> Void) -> Binding<String> {
        Binding(
            get: { model.scratchpads.indices.contains(i) ? get(model.scratchpads[i]) ?? "" : "" },
            set: { v in edit(i) { set(&$0, v) } }
        )
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("A scratchpad is an app you call up with one key and dismiss with the same key. It shows the app's window as a centered float on the display you are working on.")
                .font(.caption).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            ScrollView {
                VStack(spacing: 10) {
                    if model.scratchpads.isEmpty {
                        Text("No scratchpads").foregroundStyle(.secondary).padding(.vertical, 8)
                    }
                    ForEach(model.scratchpads.indices, id: \.self) { i in
                        GroupBox {
                            VStack(alignment: .leading, spacing: 6) {
                                HStack(spacing: 8) {
                                    Image(nsImage: AppIcons.icon(for: model.scratchpads[i].app)).resizable().frame(width: 18, height: 18)
                                    TextField("App bundle id", text: text(i, { $0.app }, { if $0.name == $0.app { $0.name = $1 }; $0.app = $1 }))
                                        .textFieldStyle(.roundedBorder)
                                    Menu("Running Apps") {
                                        ForEach(running(), id: \.self) { a in
                                            Button("\(a.name)  (\(a.id))") { edit(i) { if $0.name == $0.app { $0.name = a.id }; $0.app = a.id } }
                                        }
                                    }
                                    .frame(width: 130)
                                    Button { model.scratchpads.remove(at: i); model.scratchpadsDirty = true } label: { Image(systemName: "minus.circle") }
                                        .buttonStyle(.borderless)
                                        .help("Remove")
                                }
                                HStack(spacing: 8) {
                                    TextField("Name (defaults to the app)", text: text(i, { $0.name == $0.app ? nil : $0.name }, { $0.name = $1 }))
                                        .textFieldStyle(.roundedBorder)
                                    TextField("Key, e.g. mod-grave", text: text(i, { $0.key }, { $0.key = $1 }))
                                        .textFieldStyle(.roundedBorder)
                                    TextField("Size, e.g. 60%x60%", text: text(i, { $0.size }, { $0.size = $1 }))
                                        .textFieldStyle(.roundedBorder)
                                }
                            }
                            .padding(4)
                        }
                    }
                }
            }
            HStack {
                Button("Add Scratchpad") {
                    model.scratchpads.append(SnapScratchpad(name: "", app: "", key: nil, size: nil, shown: false))
                    model.scratchpadsDirty = true
                }
                Spacer()
                if let m = model.message { Text(m).font(.callout).foregroundStyle(.secondary) }
                Button("Revert") { model.revertScratchpads() }.disabled(!model.scratchpadsDirty)
                Button("Save") { model.applyScratchpads() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(!model.scratchpadsDirty || model.scratchpads.contains { $0.app.trimmingCharacters(in: .whitespaces).isEmpty })
            }
            Text("Keys can also be changed on the Hotkeys tab. Leave the key empty for a scratchpad you only open from the menu or the palette.")
                .font(.caption).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(8)
    }
}
