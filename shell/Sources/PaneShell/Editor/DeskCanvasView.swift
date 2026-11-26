import AppKit
import SwiftUI

/// The real display arrangement with each display's slot tree drawn inside.
@MainActor
struct DeskCanvasView: View {
    @ObservedObject var model: EditorModel

    /// Resolve a profile display reference against connected displays,
    /// mirroring `DisplayRef::resolve` in pane-core.
    static func resolve(_ ref: String, in displays: [SnapDisplay]) -> SnapDisplay? {
        let sorted = displays.sorted { $0.rank < $1.rank }
        guard !sorted.isEmpty else { return nil }
        let main = sorted.first { $0.is_main } ?? sorted[0]
        let r = ref.trimmingCharacters(in: .whitespaces)
        switch r.lowercased() {
        case "main", "primary", "any": return main
        case "left": return sorted.first
        case "right": return sorted.last
        case "center", "centre", "middle": return sorted.count >= 3 ? sorted[sorted.count / 2] : main
        default:
            if r.hasPrefix("uuid:") { return sorted.first { $0.id.caseInsensitiveCompare(String(r.dropFirst(5))) == .orderedSame } }
            let n = r.hasPrefix("name:") ? String(r.dropFirst(5)) : r
            return sorted.first { $0.id.caseInsensitiveCompare(n) == .orderedSame || $0.name.localizedCaseInsensitiveContains(n) }
        }
    }

    private func layout(in size: CGSize) -> [CGRect] {
        let displays = model.snapshot?.displays ?? []
        var frames: [CGRect?] = model.profile.display.map { Self.resolve($0.display, in: displays)?.frame.cg }
        // Unresolved displays: same size as the first known one, to the right.
        let unit = frames.compactMap { $0 }.first ?? CGRect(x: 0, y: 0, width: 1600, height: 1000)
        var nextX = (frames.compactMap { $0 }.map(\.maxX).max() ?? 0) + 80
        for i in frames.indices where frames[i] == nil {
            frames[i] = CGRect(x: nextX, y: 0, width: unit.width, height: unit.height)
            nextX += unit.width + 80
        }
        let rects = frames.compactMap { $0 }
        guard let first = rects.first else { return [] }
        let union = rects.dropFirst().reduce(first) { $0.union($1) }
        let pad: CGFloat = 24
        let scale = min((size.width - 2 * pad) / union.width, (size.height - 2 * pad) / union.height)
        let ox = (size.width - union.width * scale) / 2
        let oy = (size.height - union.height * scale) / 2
        return rects.map {
            CGRect(x: ox + ($0.minX - union.minX) * scale, y: oy + ($0.minY - union.minY) * scale,
                   width: $0.width * scale, height: $0.height * scale).insetBy(dx: 6, dy: 6)
        }
    }

    var body: some View {
        VStack(spacing: 0) {
            GeometryReader { geo in
                let rects = layout(in: geo.size)
                ZStack(alignment: .topLeading) {
                    Color(nsColor: .underPageBackgroundColor)
                    ForEach(Array(model.profile.display.enumerated()), id: \.offset) { i, pd in
                        if i < rects.count {
                            DisplayBox(model: model, index: i, display: pd)
                                .frame(width: rects[i].width, height: rects[i].height)
                                .offset(x: rects[i].minX, y: rects[i].minY)
                        }
                    }
                    if model.profile.display.isEmpty {
                        Text("No displays in this profile. Use “New” or “Capture Current”.")
                            .foregroundStyle(.secondary)
                            .frame(maxWidth: .infinity, maxHeight: .infinity)
                    }
                }
            }
            HStack {
                Menu("Add Display") {
                    ForEach(["left", "center", "right", "main"], id: \.self) { r in
                        Button(r.capitalized) { model.addDisplay(ref: r) }
                    }
                    ForEach(model.snapshot?.displays ?? [], id: \.id) { d in
                        Button(d.name) { model.addDisplay(ref: "uuid:\(d.id)") }
                    }
                }
                .frame(width: 130)
                Spacer()
            }
            .padding(8)
        }
    }
}

@MainActor
private struct DisplayBox: View {
    @ObservedObject var model: EditorModel
    let index: Int
    let display: ProfileDisplay
    @State private var workspace = ""

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 6) {
                Image(systemName: "display")
                Text(display.display).font(.caption.bold())
                Spacer()
                Text("Workspace").font(.caption).foregroundStyle(.secondary)
                TextField("", text: $workspace)
                    .textFieldStyle(.roundedBorder)
                    .font(.caption)
                    .frame(width: 60)
                    .onSubmit { model.setWorkspace(display: index, workspace) }
                Button { model.removeDisplay(index) } label: { Image(systemName: "xmark") }
                    .buttonStyle(.borderless)
                    .help("Remove this display from the profile")
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            TileTreeView(model: model, node: display.tree, ref: TileRef(display: index, path: []))
                .padding(4)
        }
        .background(RoundedRectangle(cornerRadius: 8).fill(Color(nsColor: .windowBackgroundColor)))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(Color.secondary.opacity(0.5), lineWidth: 1))
        .onAppear { workspace = display.workspace }
        .onChange(of: display.workspace) { _, w in workspace = w }
    }
}
