import AppKit
import SwiftUI

/// A saved profile next to the current desk, one row per display and
/// workspace, with “Update Profile” to save the desk into it.
@MainActor
struct ProfileDiffView: View {
    @ObservedObject var model: EditorModel
    let diff: ProfileDiff

    private var summary: String {
        let changed = diff.entries.filter { $0.change == "changed" || $0.change == "removed" }.count
        let added = diff.entries.filter { $0.change == "added" }.count
        var parts: [String] = []
        if changed > 0 { parts.append("\(changed) workspace\(changed == 1 ? "" : "s") changed") }
        if added > 0 { parts.append("\(added) new workspace\(added == 1 ? "" : "s") with windows") }
        if parts.isEmpty { return "The desk matches the profile." }
        return parts.joined(separator: ", ") + ". Updating saves the desk into the profile; displays that aren't connected stay as they are."
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack(alignment: .top) {
                VStack(alignment: .leading, spacing: 2) {
                    Text("“\(diff.name)” compared with the current desk").font(.headline)
                    Text(summary).font(.callout).foregroundStyle(.secondary)
                }
                Spacer()
                Button("Cancel") { model.diff = nil }
                    .keyboardShortcut(.cancelAction)
                Button("Edit Before Saving") { model.editDiff() }
                    .help("Open the updated profile in the editor without saving it")
                Button("Update Profile") { model.acceptDiff() }
                    .buttonStyle(.borderedProminent)
                    .keyboardShortcut(.defaultAction)
            }
            .padding(12)
            Divider()
            HStack(spacing: 12) {
                Text("Saved profile").frame(maxWidth: .infinity)
                Text("Current desk").frame(maxWidth: .infinity)
            }
            .font(.caption.bold())
            .foregroundStyle(.secondary)
            .padding(.horizontal, 16)
            .padding(.top, 8)
            ScrollView {
                VStack(spacing: 14) {
                    ForEach(Array(diff.entries.enumerated()), id: \.offset) { _, e in
                        DiffRow(entry: e, saved: e.saved.flatMap { diff.saved.display[safe: $0] }, displays: model.snapshot?.displays ?? [])
                    }
                }
                .padding(16)
            }
        }
        .background(Color(nsColor: .underPageBackgroundColor))
    }
}

@MainActor
private struct DiffRow: View {
    let entry: EntryDiff
    let saved: ProfileDisplay?
    let displays: [SnapDisplay]

    private var title: String {
        let name = DeskCanvasView.resolve(entry.display, in: displays)?.name ?? entry.display
        return "\(name) · workspace \(entry.workspace)"
    }

    private var badge: (String, Color) {
        switch entry.change {
        case "changed": return ("Changed", .orange)
        case "removed": return ("Gone from the desk", .red)
        case "added": return ("Not in the profile", .green)
        case "not_connected": return ("Display not connected", .secondary)
        default: return ("Same", .secondary)
        }
    }

    /// What changed, in words.
    private var details: String {
        var out: [String] = []
        if !entry.removed.isEmpty { out.append("− " + names(entry.removed)) }
        if !entry.added.isEmpty { out.append("+ " + names(entry.added)) }
        if entry.arrangement == true { out.append("Arrangement changed") }
        if entry.sizes == true { out.append("Sizes changed") }
        return out.joined(separator: "   ")
    }

    private func names(_ ids: [String]) -> String {
        ids.map { AppIcons.name(for: $0) }.joined(separator: ", ")
    }

    var body: some View {
        let quiet = entry.change == "unchanged" || entry.change == "not_connected"
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                Image(systemName: "display")
                Text(title).font(.callout.bold())
                Text(badge.0)
                    .font(.caption)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 1)
                    .background(Capsule().fill(badge.1.opacity(0.2)))
                Text(details).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                Spacer()
            }
            HStack(spacing: 12) {
                side(saved.map(\.tree), empty: "Not in the profile", highlight: Set(entry.removed), color: .red)
                if entry.change == "not_connected" {
                    placeholder("Display not connected")
                } else {
                    side(entry.live, empty: "No windows", highlight: Set(entry.added), color: .green)
                }
            }
            .frame(height: quiet ? 90 : 150)
        }
        .padding(10)
        .background(RoundedRectangle(cornerRadius: 8).fill(Color(nsColor: .windowBackgroundColor)))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(Color.secondary.opacity(0.4), lineWidth: 1))
        .opacity(quiet ? 0.7 : 1)
    }

    @ViewBuilder
    private func side(_ tree: SerNode?, empty: String, highlight: Set<String>, color: Color) -> some View {
        if let t = tree, !t.children.isEmpty {
            MiniTree(node: t, highlight: highlight, color: color)
                .padding(4)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(RoundedRectangle(cornerRadius: 6).fill(Color.secondary.opacity(0.08)))
        } else {
            placeholder(empty)
        }
    }

    private func placeholder(_ text: String) -> some View {
        RoundedRectangle(cornerRadius: 6)
            .strokeBorder(style: StrokeStyle(lineWidth: 1, dash: [4, 3]))
            .foregroundStyle(.secondary)
            .overlay(Text(text).font(.caption).foregroundStyle(.secondary))
            .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// Read-only slot tree: nested tiles with app icons. Slots whose app is in
/// `highlight` get a coloured outline.
@MainActor
struct MiniTree: View {
    let node: SerNode
    let highlight: Set<String>
    let color: Color

    private let gap: CGFloat = 3

    var body: some View {
        switch node {
        case let .container(orientation, layout, weights, children):
            if children.isEmpty {
                AnyView(Color.clear)
            } else {
                AnyView(split(horizontal: orientation == "horizontal", accordion: layout == "accordion",
                              weights: weights, children: children))
            }
        case let .slot(slot):
            AnyView(tile(slot))
        case .window:
            AnyView(Color.clear)
        }
    }

    private func split(horizontal: Bool, accordion: Bool, weights: [Double], children: [SerNode]) -> some View {
        GeometryReader { geo in
            let total = horizontal ? geo.size.width : geo.size.height
            let avail = max(total - gap * CGFloat(children.count - 1), 1)
            let w = children.indices.map { weights.indices.contains($0) ? max(weights[$0], 0.01) : 1.0 }
            let sum = w.reduce(0, +)
            let sizes = w.map { CGFloat($0 / sum) * avail }
            ZStack(alignment: .topLeading) {
                ForEach(children.indices, id: \.self) { i in
                    let start = sizes.prefix(i).reduce(0, +) + gap * CGFloat(i)
                    MiniTree(node: children[i], highlight: highlight, color: color)
                        .frame(width: horizontal ? sizes[i] : geo.size.width,
                               height: horizontal ? geo.size.height : sizes[i])
                        .offset(x: horizontal ? start : 0, y: horizontal ? 0 : start)
                }
                if accordion {
                    Text("accordion").font(.caption2).padding(2)
                        .background(Capsule().fill(Color.secondary.opacity(0.25)))
                        .padding(3)
                }
            }
        }
    }

    private func tile(_ slot: AppSlot) -> some View {
        let marked = highlight.contains(slot.bundle_id)
        return GeometryReader { geo in
            let side = max(min(28, geo.size.width * 0.4, geo.size.height * 0.45), 8)
            RoundedRectangle(cornerRadius: 5)
                .fill((marked ? color : Color.accentColor).opacity(marked ? 0.18 : 0.1))
                .overlay(
                    RoundedRectangle(cornerRadius: 5)
                        .stroke(marked ? color : Color.secondary.opacity(0.5), lineWidth: marked ? 2 : 1)
                )
                .overlay(
                    VStack(spacing: 2) {
                        Image(nsImage: AppIcons.icon(for: slot.bundle_id))
                            .resizable()
                            .frame(width: side, height: side)
                        if geo.size.height > 50 {
                            Text(AppIcons.name(for: slot.bundle_id)).font(.caption2).lineLimit(1)
                        }
                    }
                    .padding(2)
                )
                .help(slot.title.map { "\(AppIcons.name(for: slot.bundle_id)) — \($0)" } ?? AppIcons.name(for: slot.bundle_id))
        }
    }
}
