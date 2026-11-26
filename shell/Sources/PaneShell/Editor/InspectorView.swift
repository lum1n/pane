import AppKit
import SwiftUI

/// Details of the selected tile.
@MainActor
struct InspectorView: View {
    @ObservedObject var model: EditorModel
    @State private var title = ""
    @State private var args = ""

    private var slot: AppSlot? {
        guard let sel = model.selection,
              case let .slot(s)? = model.profile.display[safe: sel.display]?.tree.node(at: sel.path) else { return nil }
        return s
    }

    var body: some View {
        Form {
            if let sel = model.selection, let s = slot {
                Section("App") {
                    LabeledContent("Name", value: AppIcons.name(for: s.bundle_id))
                    LabeledContent("Bundle ID") { Text(s.bundle_id).textSelection(.enabled).font(.caption) }
                }
                Section("Matching") {
                    TextField("Title regex (optional)", text: $title)
                        .onSubmit { commit(sel, s) }
                    TextField("Launch arguments", text: $args)
                        .onSubmit { commit(sel, s) }
                    Button("Update") { commit(sel, s) }
                }
                Section("Layout") {
                    if let parent = sel.parent {
                        LabeledContent("Split", value: model.orientation(at: parent))
                        Button("Toggle Split Direction") { model.toggleOrientation(sel) }
                        Button("Toggle Accordion") { model.toggleLayout(parent) }
                    }
                    Button("Remove Tile", role: .destructive) { model.remove(sel) }
                }
            } else {
                Text("Select a tile to edit it.").foregroundStyle(.secondary)
                Text("Right-click a tile for more. Drag tiles to swap (middle) or move (edges). Drag the gaps between tiles to resize.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
        .onChange(of: model.selection) { _, _ in load() }
        .onChange(of: model.profile) { _, _ in load() }
        .onAppear { load() }
    }

    private func load() {
        title = slot?.title ?? ""
        args = (slot?.launch_args ?? []).joined(separator: " ")
    }

    private func commit(_ sel: TileRef, _ s: AppSlot) {
        let a = args.split(separator: " ").map(String.init)
        model.setSlot(sel, AppSlot(bundle_id: s.bundle_id, title: title.isEmpty ? nil : title, launch_args: a.isEmpty ? nil : a))
    }
}
