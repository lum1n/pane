import AppKit
import SwiftUI

/// A scratchpad being added (`index` nil) or edited in the sheet.
private struct ScratchpadDraft: Identifiable {
    let id = UUID()
    var index: Int?
    var app: String
    /// Empty means "same as the app".
    var name: String
    var key: String
    var size: String

    init(index: Int?, _ sp: SnapScratchpad?) {
        self.index = index
        app = sp?.app ?? ""
        name = sp.map { $0.name == $0.app ? "" : $0.name } ?? ""
        key = sp?.key ?? ""
        size = sp?.size ?? ""
    }

    var scratchpad: SnapScratchpad {
        let n = name.trimmingCharacters(in: .whitespaces)
        return SnapScratchpad(name: n.isEmpty ? app : n, app: app, key: key, size: size, shown: false)
    }
}

@MainActor
struct ScratchpadsPane: View {
    let model: SettingsModel
    @State private var selection: Int?
    @State private var draft: ScratchpadDraft?

    private func remove() {
        guard let i = selection, model.scratchpads.indices.contains(i) else { return }
        var list = model.scratchpads
        list.remove(at: i)
        if model.applyScratchpads(list) { selection = nil }
    }

    private func edit(_ i: Int) {
        guard model.scratchpads.indices.contains(i) else { return }
        draft = ScratchpadDraft(index: i, model.scratchpads[i])
    }

    var body: some View {
        Form {
            Section {
                if model.scratchpads.isEmpty {
                    ContentUnavailableView {
                        Label("No Scratchpads", systemImage: "rectangle.on.rectangle")
                    } description: {
                        Text("Click + to add one.")
                    }
                    .controlSize(.small)
                    .frame(maxWidth: .infinity)
                } else {
                    ForEach(Array(model.scratchpads.enumerated()), id: \.offset) { i, sp in
                        row(sp)
                            .modifier(SelectableRow(selected: selection == i))
                            .onTapGesture(count: 2) { edit(i) }
                            .onTapGesture { selection = i }
                            .contextMenu {
                                Button("Edit…") { edit(i) }
                                Button("Remove", role: .destructive) { selection = i; remove() }
                            }
                            .accessibilityAddTraits(selection == i ? .isSelected : [])
                    }
                }
                ListEditBar(
                    canRemove: selection.map { model.scratchpads.indices.contains($0) } ?? false,
                    remove: remove,
                    add: {
                        Button { draft = ScratchpadDraft(index: nil, nil) } label: {
                            Image(systemName: "plus").frame(width: 20, height: 18)
                        }
                        .buttonStyle(.borderless)
                    },
                    trailing: AnyView(
                        Button("Edit…") { if let i = selection { edit(i) } }
                            .disabled(selection == nil)
                    )
                )
            } footer: {
                Text("A scratchpad is an app you call up with one key and dismiss with the same key. It shows the app's window as a centred float on the display you're working on. Keys can also be changed on the Shortcuts tab.")
                    .font(.footnote).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }
        }
        .settingsPane(model, height: 480)
        .sheet(item: $draft) { d in
            ScratchpadEditor(draft: d) { saved in
                var list = model.scratchpads
                if let i = saved.index, list.indices.contains(i) { list[i] = saved.scratchpad } else { list.append(saved.scratchpad) }
                return model.applyScratchpads(list) ? nil : (model.message ?? "Couldn't save the scratchpads")
            }
        }
    }

    private func row(_ sp: SnapScratchpad) -> some View {
        HStack(spacing: 8) {
            AppIconView(bundleId: sp.app, size: 24)
            Text(sp.displayName)
            Spacer()
            if let size = sp.size, !size.isEmpty {
                Text(size).font(.caption).foregroundStyle(.secondary).monospacedDigit()
            }
            KeycapView(chord: sp.key.map { Shortcut.resolving($0, modifier: model.settings?.modifier ?? "alt") })
        }
    }
}

/// Add or edit one scratchpad. Save writes config.toml; if that fails the
/// sheet stays open and shows the error.
@MainActor
private struct ScratchpadEditor: View {
    @State var draft: ScratchpadDraft
    /// Returns an error message, or nil once saved.
    let onSave: (ScratchpadDraft) -> String?
    @State private var error: String?
    @Environment(\.dismiss) private var dismiss

    private var valid: Bool { !draft.app.trimmingCharacters(in: .whitespaces).isEmpty }

    var body: some View {
        VStack(spacing: 0) {
            Form {
                Section {
                    LabeledContent("App") {
                        Menu {
                            let running = KnownApp.running()
                            if !running.isEmpty {
                                Section("Running Apps") {
                                    ForEach(running, id: \.self) { a in
                                        Button { draft.app = a.id } label: {
                                            Label { Text(a.name) } icon: { Image(nsImage: AppIcons.small(for: a.id)) }
                                        }
                                    }
                                }
                                Divider()
                            }
                            Button("Choose Application…") {
                                if let b = AppChooser.choose(multiple: false).first { draft.app = b }
                            }
                        } label: {
                            if valid {
                                Label { Text(AppIcons.name(for: draft.app)) } icon: { Image(nsImage: AppIcons.small(for: draft.app)) }
                            } else {
                                Text("Choose an App")
                            }
                        }
                        .fixedSize()
                    }
                    TextField("Name", text: $draft.name, prompt: Text(valid ? AppIcons.name(for: draft.app) : "Same as the app"))
                    TextField("Key", text: $draft.key, prompt: Text("mod-grave"))
                    TextField("Size", text: $draft.size, prompt: Text("60%x60%"))
                } header: {
                    Text(draft.index == nil ? "New Scratchpad" : "Edit Scratchpad")
                } footer: {
                    Text("Leave the key empty for a scratchpad you only open from the menu or the palette.")
                        .font(.footnote).foregroundStyle(.secondary)
                }
            }
            .formStyle(.grouped)
            .scrollDisabled(true)
            HStack {
                if let error {
                    Label(error, systemImage: "exclamationmark.triangle.fill")
                        .foregroundStyle(.red)
                        .font(.callout)
                        .lineLimit(2)
                }
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Save") {
                    error = onSave(draft)
                    if error == nil { dismiss() }
                }
                    .keyboardShortcut(.defaultAction)
                    .disabled(!valid)
            }
            .padding([.horizontal, .bottom], 20)
        }
        .frame(width: 440, height: 340)
    }
}
