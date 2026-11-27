import AppKit
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
    var mouse_follows_focus: Bool
    var focus_follows_mouse: Bool
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
@Observable
final class SettingsModel {
    var settings: PaneSettings?
    var bindings: [BindingRow] = []
    var configError: String?
    /// Transient feedback, shown in the pane's status capsule.
    var message: String?
    var recording: String?
    /// The scratchpads in config.toml.
    var scratchpads: [SnapScratchpad] = []
    @ObservationIgnored private var keyNames: [UInt16: String] = [:]
    @ObservationIgnored private var monitor: Any?

    func load() {
        configError = PaneCore.shared.snapshot?.config_error
        switch PaneCore.shared.request(["cmd": "query", "what": "config"], as: ConfigReply.self) {
        case let .success(c): settings = c.settings
        case let .failure(e): configError = e.message
        }
        if case let .success(rows) = PaneCore.shared.request(["cmd": "query", "what": "bindings"], as: [BindingRow].self) {
            bindings = rows
        }
        scratchpads = PaneCore.shared.snapshot?.scratchpads ?? []
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

    /// Write `list` to config.toml as the scratchpads. The name is only
    /// written when it differs from the app. Returns false on an error
    /// (shown in `message`), so an edit sheet can stay open.
    @discardableResult
    func applyScratchpads(_ list: [SnapScratchpad]) -> Bool {
        let rows: [[String: Any]] = list.map { sp in
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
        finish(r, ok: "Scratchpads saved")
        guard case .ok = r else { return false }
        // The snapshot can lag the write; show what was saved.
        scratchpads = list
        return true
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
        glyphs(chord).joined()
    }

    /// One string per key: modifiers in macOS order (⌃⌥⇧⌘), then the key.
    /// Modifiers Pane doesn't draw as symbols (`mod`, `hyper`) keep their name.
    static func glyphs(_ chord: String) -> [String] {
        var parts = chord.split(separator: "-").map(String.init)
        guard let key = parts.popLast() else { return [] }
        let symbols: [String: String] = ["cmd": "⌘", "ctrl": "⌃", "alt": "⌥", "shift": "⇧"]
        let order = ["ctrl", "alt", "shift", "cmd"]
        let named = parts.filter { symbols[$0] == nil }.map { $0.capitalized }
        return named + order.filter { parts.contains($0) }.compactMap { symbols[$0] } + [prettyKey(key)]
    }

    /// Replace `mod` in a chord with the configured modifier keys, so
    /// `mod-x` draws as ⌥ X rather than "Mod X".
    static func resolving(_ chord: String, modifier: String) -> String {
        let f = flags(modifier)
        let mods = [f.ctrl ? "ctrl" : nil, f.alt ? "alt" : nil, f.shift ? "shift" : nil, f.cmd ? "cmd" : nil].compactMap { $0 }
        guard !mods.isEmpty else { return chord }
        return chord.split(separator: "-").flatMap { p -> [String] in p == "mod" ? mods : [String(p)] }.joined(separator: "-")
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

extension KnownApp {
    /// Running apps by name: what Pane observes (includes unbundled ones)
    /// plus regular apps from NSWorkspace.
    @MainActor
    static func running() -> [KnownApp] {
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
        return out.sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
    }
}

/// Colour helpers for the border colour setting.
enum BorderColor {
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
}
