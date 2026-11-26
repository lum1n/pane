import AppKit
import SwiftUI

/// Recursive view of a slot tree: nested rounded rects with app icons.
@MainActor
struct TileTreeView: View {
    @ObservedObject var model: EditorModel
    let node: SerNode
    let ref: TileRef

    var body: some View {
        switch node {
        case let .container(orientation, layout, weights, children):
            if children.isEmpty {
                AnyView(EmptyDrop(model: model, ref: ref))
            } else {
                AnyView(SplitView(model: model, ref: ref, horizontal: orientation == "horizontal",
                                  accordion: layout == "accordion", weights: weights, children: children))
            }
        case let .slot(slot):
            AnyView(SlotTile(model: model, slot: slot, ref: ref))
        case let .window(id):
            AnyView(Text("window \(id)").frame(maxWidth: .infinity, maxHeight: .infinity))
        }
    }
}

@MainActor
private struct EmptyDrop: View {
    @ObservedObject var model: EditorModel
    let ref: TileRef
    @State private var targeted = false

    var body: some View {
        GeometryReader { geo in
            RoundedRectangle(cornerRadius: 6)
                .strokeBorder(style: StrokeStyle(lineWidth: 1.5, dash: [5, 4]))
                .foregroundStyle(targeted ? Color.accentColor : Color.secondary)
                .overlay(Text("Drop an app here").font(.caption).foregroundStyle(.secondary))
                .contentShape(Rectangle())
                .dropDestination(for: String.self) { items, loc in
                    guard let p = items.first else { return false }
                    model.drop(payload: p, on: ref, zone: dropZone(at: loc, in: geo.size))
                    return true
                } isTargeted: { targeted = $0 }
        }
    }
}

@MainActor
private struct SplitView: View {
    @ObservedObject var model: EditorModel
    let ref: TileRef
    let horizontal: Bool
    let accordion: Bool
    let weights: [Double]
    let children: [SerNode]
    @State private var drag: (index: Int, delta: CGFloat)?

    private let gap: CGFloat = 4

    var body: some View {
        GeometryReader { geo in
            let total = horizontal ? geo.size.width : geo.size.height
            let avail = max(total - gap * CGFloat(children.count - 1), 1)
            let w = children.indices.map { weights.indices.contains($0) ? max(weights[$0], 0.01) : 1.0 }
            let sum = w.reduce(0, +)
            let sizes = sizesWithDrag(w.map { CGFloat($0 / sum) * avail })
            ZStack(alignment: .topLeading) {
                ForEach(children.indices, id: \.self) { i in
                    let start = sizes.prefix(i).reduce(0, +) + gap * CGFloat(i)
                    TileTreeView(model: model, node: children[i], ref: TileRef(display: ref.display, path: ref.path + [i]))
                        .frame(width: horizontal ? sizes[i] : geo.size.width, height: horizontal ? geo.size.height : sizes[i])
                        .offset(x: horizontal ? start : 0, y: horizontal ? 0 : start)
                    if i < children.count - 1 {
                        divider(i: i, at: start + sizes[i], sizes: sizes, size: geo.size)
                    }
                }
                if accordion {
                    Text("accordion").font(.caption2).padding(3)
                        .background(Capsule().fill(Color.secondary.opacity(0.25)))
                        .padding(4)
                }
            }
        }
    }

    private func sizesWithDrag(_ base: [CGFloat]) -> [CGFloat] {
        guard let d = drag, base.indices.contains(d.index + 1) else { return base }
        var s = base
        let minSize: CGFloat = 30
        let delta = max(min(d.delta, s[d.index + 1] - minSize), minSize - s[d.index])
        s[d.index] += delta
        s[d.index + 1] -= delta
        return s
    }

    private func divider(i: Int, at pos: CGFloat, sizes: [CGFloat], size: CGSize) -> some View {
        Rectangle()
            .fill(Color.clear)
            .contentShape(Rectangle())
            .frame(width: horizontal ? gap + 4 : size.width, height: horizontal ? size.height : gap + 4)
            .offset(x: horizontal ? pos - 2 : 0, y: horizontal ? 0 : pos - 2)
            .onHover { inside in
                if inside { (horizontal ? NSCursor.resizeLeftRight : NSCursor.resizeUpDown).push() } else { NSCursor.pop() }
            }
            .gesture(
                DragGesture(minimumDistance: 1)
                    .onChanged { v in drag = (index: i, delta: horizontal ? v.translation.width : v.translation.height) }
                    .onEnded { _ in
                        let s = sizesWithDrag(sizes)
                        let pairTotal = weights.indices.contains(i + 1) ? weights[i] + weights[i + 1] : 2
                        let pairSize = s[i] + s[i + 1]
                        drag = nil
                        guard pairSize > 0 else { return }
                        let a = Double(s[i] / pairSize) * pairTotal
                        model.setWeights(container: ref, index: i, a, pairTotal - a)
                    }
            )
    }
}

@MainActor
private struct SlotTile: View {
    @ObservedObject var model: EditorModel
    let slot: AppSlot
    let ref: TileRef
    @State private var targeted = false

    var body: some View {
        GeometryReader { geo in
            let selected = model.selection == ref
            RoundedRectangle(cornerRadius: 6)
                .fill(Color.accentColor.opacity(targeted ? 0.35 : 0.12))
                .overlay(
                    RoundedRectangle(cornerRadius: 6)
                        .stroke(selected ? Color.accentColor : Color.secondary.opacity(0.6), lineWidth: selected ? 2 : 1)
                )
                .overlay(
                    VStack(spacing: 4) {
                        Image(nsImage: AppIcons.icon(for: slot.bundle_id))
                            .resizable()
                            .frame(width: min(40, geo.size.width * 0.4), height: min(40, geo.size.width * 0.4))
                        Text(AppIcons.name(for: slot.bundle_id)).font(.caption).lineLimit(1)
                        if let t = slot.title, !t.isEmpty {
                            Text(t).font(.caption2).foregroundStyle(.secondary).lineLimit(1)
                        }
                    }
                    .padding(4)
                )
                .contentShape(Rectangle())
                .onTapGesture { model.selection = ref }
                .draggable(DragPayload.tile(ref)) {
                    Image(nsImage: AppIcons.icon(for: slot.bundle_id)).resizable().frame(width: 32, height: 32)
                }
                .dropDestination(for: String.self) { items, loc in
                    guard let p = items.first else { return false }
                    model.drop(payload: p, on: ref, zone: dropZone(at: loc, in: geo.size))
                    return true
                } isTargeted: { targeted = $0 }
                .contextMenu {
                    Button("Remove") { model.remove(ref) }
                    Button("Toggle Split Direction") { model.toggleOrientation(ref) }
                    if let parent = ref.parent {
                        Button("Toggle Accordion") { model.toggleLayout(parent) }
                    }
                }
        }
    }
}
