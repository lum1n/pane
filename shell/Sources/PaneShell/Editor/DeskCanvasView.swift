import AppKit
import SwiftUI

/// The real display arrangement with each display's slot tree drawn inside.
@MainActor
struct DeskCanvasView: View {
    let model: EditorModel

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

    /// Profile entries grouped by the display they resolve to.
    private var groups: [DisplayGroup] {
        let displays = model.snapshot?.displays ?? []
        var out: [DisplayGroup] = []
        for (i, pd) in model.profile.display.enumerated() {
            let resolved = Self.resolve(pd.display, in: displays)
            let key = resolved?.id ?? pd.display.lowercased()
            if let g = out.firstIndex(where: { $0.key == key }) {
                out[g].entries.append(i)
            } else {
                out.append(DisplayGroup(key: key, ref: pd.display, title: resolved?.name ?? pd.display,
                                        frame: resolved?.frame.cg, entries: [i]))
            }
        }
        return out
    }

    private func layout(_ groups: [DisplayGroup], in size: CGSize) -> [CGRect] {
        var frames: [CGRect?] = groups.map(\.frame)
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
        GeometryReader { geo in
            let gs = groups
            let rects = layout(gs, in: geo.size)
            ZStack(alignment: .topLeading) {
                ForEach(Array(gs.enumerated()), id: \.element.key) { i, g in
                    if i < rects.count {
                        DisplayBox(model: model, group: g)
                            .frame(width: rects[i].width, height: rects[i].height)
                            .offset(x: rects[i].minX, y: rects[i].minY)
                    }
                }
            }
        }
        .background(.windowBackground)
        .overlay {
            if model.profile.display.isEmpty {
                ContentUnavailableView {
                    Label("No Displays", systemImage: "display")
                } description: {
                    Text("Start from the connected displays, or capture the desk as it is now.")
                } actions: {
                    Button("New Profile") { model.newProfile() }
                    Button("Capture Current Desk") { model.capture() }
                        .buttonStyle(.borderedProminent)
                }
            }
        }
    }
}

struct DisplayGroup {
    var key: String
    var ref: String
    var title: String
    var frame: CGRect?
    /// Indices into `profile.display`, one per workspace.
    var entries: [Int]
}

@MainActor
private struct DisplayBox: View {
    let model: EditorModel
    let group: DisplayGroup
    @State private var renaming = false
    @State private var name = ""

    /// Entry shown in this box: the selected tab, else the visible one, else the first.
    private var current: Int {
        if let sel = model.selectedTab[group.key], group.entries.contains(sel) { return sel }
        return group.entries.first { model.profile.display[safe: $0]?.isVisible == true } ?? group.entries[0]
    }

    var body: some View {
        let cur = current
        let shape = RoundedRectangle(cornerRadius: 14, style: .continuous)
        VStack(spacing: 6) {
            HStack(spacing: 6) {
                Image(systemName: "display").foregroundStyle(.secondary)
                Text(group.title).font(.callout.weight(.semibold)).lineLimit(1)
                Spacer()
                Button { model.removeEntries(group.entries) } label: {
                    Image(systemName: "xmark.circle.fill").symbolRenderingMode(.hierarchical)
                }
                .buttonStyle(.borderless)
                .foregroundStyle(.secondary)
                .help("Remove this display from the profile")
            }
            HStack(spacing: 6) {
                tabs(cur)
                Button { model.addWorkspace(like: cur, groupKey: group.key) } label: { Image(systemName: "plus") }
                    .buttonStyle(.borderless)
                    .help("Add a workspace to this display")
                Spacer()
                Button {
                    name = model.profile.display[safe: cur]?.workspace ?? ""
                    renaming = true
                } label: { Image(systemName: "pencil") }
                .buttonStyle(.borderless)
                .help("Rename this workspace")
                .popover(isPresented: $renaming, arrowEdge: .bottom) {
                    HStack {
                        TextField("Workspace", text: $name)
                            .textFieldStyle(.roundedBorder)
                            .frame(width: 140)
                            .onSubmit {
                                model.setWorkspace(display: cur, name)
                                renaming = false
                            }
                        Button("Rename") {
                            model.setWorkspace(display: cur, name)
                            renaming = false
                        }
                        .keyboardShortcut(.defaultAction)
                    }
                    .padding(12)
                }
                Button { model.setVisible(cur) } label: {
                    Image(systemName: model.profile.display[safe: cur]?.isVisible == true ? "star.fill" : "star")
                }
                .buttonStyle(.borderless)
                .foregroundStyle(model.profile.display[safe: cur]?.isVisible == true ? Color.yellow : Color.secondary)
                .help("Show this workspace on the display after applying")
            }
            if let pd = model.profile.display[safe: cur] {
                TileTreeView(model: model, node: pd.tree, ref: TileRef(display: cur, path: []))
            }
        }
        .padding(10)
        .background(shape.fill(.background))
        .overlay(shape.strokeBorder(.separator))
        .shadow(color: .black.opacity(0.12), radius: 8, y: 2)
    }

    /// Workspace tabs as compact capsules; ★ marks the one shown after applying.
    private func tabs(_ cur: Int) -> some View {
        HStack(spacing: 2) {
            ForEach(group.entries, id: \.self) { i in
                if let pd = model.profile.display[safe: i] {
                    Button {
                        model.selectedTab[group.key] = i
                    } label: {
                        HStack(spacing: 3) {
                            if pd.isVisible {
                                Image(systemName: "star.fill").font(.system(size: 8)).foregroundStyle(.yellow)
                            }
                            Text(pd.workspace)
                        }
                        .font(.caption.weight(i == cur ? .semibold : .regular))
                        .padding(.horizontal, 9)
                        .padding(.vertical, 3)
                        .background(Capsule().fill(i == cur ? Color.accentColor.opacity(0.22) : .clear))
                        .contentShape(Capsule())
                    }
                    .buttonStyle(.plain)
                    .contextMenu {
                        Button("Show This Workspace After Applying") { model.setVisible(i) }
                        Button("Remove Workspace", role: .destructive) { model.removeEntries([i]) }
                    }
                    .help("Workspace \(pd.workspace). Right-click for options.")
                    .accessibilityAddTraits(i == cur ? .isSelected : [])
                }
            }
        }
        .padding(2)
        .background(Capsule().fill(Color.secondary.opacity(0.1)))
    }
}
