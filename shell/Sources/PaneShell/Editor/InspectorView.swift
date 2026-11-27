import AppKit
import SwiftUI

/// Details of the selected tile and its workspace.
@MainActor
struct InspectorView: View {
    let model: EditorModel
    @State private var title = ""
    @State private var args = ""
    @State private var workspace = ""

    private var slot: AppSlot? {
        guard let sel = model.selection,
              case let .slot(s)? = model.profile.display[safe: sel.display]?.tree.node(at: sel.path) else { return nil }
        return s
    }

    var body: some View {
        Group {
            if let sel = model.selection, let s = slot {
                form(sel, s)
            } else {
                ContentUnavailableView {
                    Label("No Tile Selected", systemImage: "square.dashed")
                } description: {
                    Text("Click a tile to edit it. Drag tiles to swap (middle) or move (edges), and drag the gaps between tiles to resize.")
                }
            }
        }
        .onChange(of: model.selection) { _, _ in load() }
        .onChange(of: model.profile) { _, _ in load() }
        .onAppear { load() }
    }

    private func form(_ sel: TileRef, _ s: AppSlot) -> some View {
        Form {
            Section {
                HStack(spacing: 12) {
                    AppIconView(bundleId: s.bundle_id, size: 48)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(AppIcons.name(for: s.bundle_id)).font(.headline).lineLimit(2)
                        Text(s.bundle_id)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .textSelection(.enabled)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                }
                .padding(.vertical, 4)
            }
            Section {
                TextField("Title", text: $title, prompt: Text("Regex, optional"))
                    .onSubmit { commit(sel, s) }
                TextField("Arguments", text: $args, prompt: Text("Launch arguments"))
                    .onSubmit { commit(sel, s) }
                Button("Update Matching") { commit(sel, s) }
                    .disabled(title == (s.title ?? "") && args == (s.launch_args ?? []).joined(separator: " "))
            } header: {
                Text("Matching")
            } footer: {
                Text("The title regex picks one of the app's windows; the arguments are used when Pane launches the app.")
                    .font(.footnote).foregroundStyle(.secondary)
            }
            Section("Workspace") {
                TextField("Name", text: $workspace)
                    .onSubmit { model.setWorkspace(display: sel.display, workspace) }
                Toggle("Show after applying", isOn: Binding(
                    get: { model.profile.display[safe: sel.display]?.isVisible == true },
                    set: { if $0 { model.setVisible(sel.display) } }
                ))
            }
            if let parent = sel.parent {
                Section("Layout") {
                    Picker("Split", selection: Binding(
                        get: { model.orientation(at: parent) },
                        set: { model.setOrientation(parent, $0) }
                    )) {
                        Label("Side by Side", systemImage: "rectangle.split.2x1").tag("horizontal")
                        Label("Stacked", systemImage: "rectangle.split.1x2").tag("vertical")
                    }
                    .pickerStyle(.segmented)
                    Toggle("Accordion", isOn: Binding(
                        get: { model.layout(at: parent) == "accordion" },
                        set: { _ in model.toggleLayout(parent) }
                    ))
                }
            }
            Section {
                Button("Remove Tile", role: .destructive) { model.remove(sel) }
                    .frame(maxWidth: .infinity)
            }
        }
        .formStyle(.grouped)
    }

    private func load() {
        title = slot?.title ?? ""
        args = (slot?.launch_args ?? []).joined(separator: " ")
        workspace = model.selection.flatMap { model.profile.display[safe: $0.display]?.workspace } ?? ""
    }

    private func commit(_ sel: TileRef, _ s: AppSlot) {
        let a = args.split(separator: " ").map(String.init)
        model.setSlot(sel, AppSlot(bundle_id: s.bundle_id, title: title.isEmpty ? nil : title, launch_args: a.isEmpty ? nil : a))
    }
}
