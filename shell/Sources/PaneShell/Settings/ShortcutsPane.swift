import AppKit
import SwiftUI

@MainActor
struct ShortcutsPane: View {
    let model: SettingsModel
    @State private var query = ""
    @State private var selection: BindingRow.ID?
    @State private var confirmReset = false

    private var rows: [BindingRow] {
        let q = query.trimmingCharacters(in: .whitespaces)
        guard !q.isEmpty else { return model.bindings }
        return model.bindings.filter { row in
            row.command.localizedCaseInsensitiveContains(q)
                || row.chords.contains { Shortcut.pretty($0).localizedCaseInsensitiveContains(q) || $0.localizedCaseInsensitiveContains(q) }
        }
    }

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

    private func toggleRecording(_ command: String) {
        if model.recording == command { model.stopRecording() } else { model.startRecording(command) }
    }

    var body: some View {
        VStack(spacing: 0) {
            Form {
                Section {
                    LabeledContent("Modifier") {
                        HStack(spacing: 14) {
                            modifierToggle("⌃ Control", \.ctrl)
                            modifierToggle("⌥ Option", \.alt)
                            modifierToggle("⇧ Shift", \.shift)
                            modifierToggle("⌘ Command", \.cmd)
                        }
                    }
                    Toggle("Move windows by holding the modifier and pointing", isOn: Binding(
                        get: { model.settings?.gestures ?? true },
                        set: { model.set("gestures", $0) }
                    ))
                } footer: {
                    Text("Every default shortcut uses the modifier. Moving adds ⇧ and resizing adds ⌃ (or the next key not already in the modifier). To move by pointing, hold the modifier over a window and point at another tile, its edge, a screen edge or another display, then release. Esc cancels.")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .formStyle(.grouped)
            .scrollDisabled(true)
            .frame(height: 190)

            HStack {
                SearchField(text: $query, prompt: "Search shortcuts")
                    .frame(maxWidth: 240)
                Spacer()
                Button("Reset All…") { confirmReset = true }
            }
            .padding(.horizontal, 20)
            .padding(.bottom, 8)

            Table(rows, selection: $selection) {
                TableColumn("Command", value: \.command)
                TableColumn("Shortcut") { row in
                    ShortcutCell(model: model, row: row) { toggleRecording(row.command) }
                }
                .width(min: 170, ideal: 200)
            }
            .contextMenu(forSelectionType: BindingRow.ID.self) { ids in
                if let id = ids.first, let row = model.bindings.first(where: { $0.id == id }) {
                    Button("Record Shortcut") { model.startRecording(row.command) }
                    Button("Clear Shortcut") { model.rebind(row.command, to: nil) }
                        .disabled(row.chords.isEmpty)
                    Button(row.default.map { "Reset to \(Shortcut.pretty($0))" } ?? "Reset to Default") {
                        model.rebind(row.command, to: row.default)
                    }
                    .disabled(row.default == nil || row.chords == [row.default!])
                }
            } primaryAction: { ids in
                if let id = ids.first { toggleRecording(id) }
            }
            .padding(.horizontal, 20)
            .padding(.bottom, 20)
        }
        .settingsPane(model, height: 620)
        .confirmationDialog("Reset all shortcuts to the defaults?", isPresented: $confirmReset) {
            Button("Reset All Shortcuts", role: .destructive) { model.resetAll() }
        } message: {
            Text("Shortcuts you changed or cleared go back to Pane's defaults.")
        }
    }
}

/// The shortcut as keycaps; click to record. Clear and Reset appear on hover.
@MainActor
private struct ShortcutCell: View {
    let model: SettingsModel
    let row: BindingRow
    let record: () -> Void
    @State private var hovering = false

    private var recording: Bool { model.recording == row.command }
    private var canReset: Bool { row.default != nil && row.chords != [row.default!] }

    var body: some View {
        HStack(spacing: 6) {
            Button(action: record) {
                KeycapView(chord: row.chords.first, recording: recording)
            }
            .buttonStyle(.plain)
            .help("Click, then press the new shortcut")
            Spacer(minLength: 0)
            if hovering && !recording {
                if canReset {
                    Button { model.rebind(row.command, to: row.default) } label: {
                        Image(systemName: "arrow.counterclockwise")
                    }
                    .buttonStyle(.borderless)
                    .help(row.default.map { "Reset to \(Shortcut.pretty($0))" } ?? "")
                }
                if !row.chords.isEmpty {
                    Button { model.rebind(row.command, to: nil) } label: {
                        Image(systemName: "xmark.circle.fill")
                    }
                    .buttonStyle(.borderless)
                    .foregroundStyle(.secondary)
                    .help("Clear the shortcut")
                }
            }
        }
        .contentShape(Rectangle())
        .onHover { hovering = $0 }
    }
}
