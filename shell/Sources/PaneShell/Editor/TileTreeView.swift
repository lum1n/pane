import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// Recursive view of a slot tree: nested rounded rects with app icons.
@MainActor
struct TileTreeView: View {
    let model: EditorModel
    let node: SerNode
    let ref: TileRef

    var body: some View {
        switch node {
        case let .container(orientation, layout, weights, children):
            if children.isEmpty {
                EmptyDrop(model: model, ref: ref)
            } else {
                SplitView(model: model, ref: ref, horizontal: orientation == "horizontal",
                          accordion: layout == "accordion", weights: weights, children: children)
            }
        case let .slot(slot):
            SlotTile(model: model, slot: slot, ref: ref)
        case let .window(id):
            Text("window \(id)").frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }
}

private let tileRadius: CGFloat = 8

/// Tracks the drop zone under the pointer while dragging over a tile, so
/// the tile can show where the drop will land, and hands the payload over.
private struct TileDropDelegate: DropDelegate {
    let size: CGSize
    /// The middle only (empty containers have no edges to split).
    var centerOnly = false
    @Binding var zone: String?
    let onDrop: (String, String) -> Void

    private func zoneAt(_ p: CGPoint) -> String {
        centerOnly ? "center" : dropZone(at: p, in: size)
    }

    func validateDrop(info: DropInfo) -> Bool {
        info.hasItemsConforming(to: [.plainText])
    }

    func dropEntered(info: DropInfo) {
        zone = zoneAt(info.location)
    }

    func dropUpdated(info: DropInfo) -> DropProposal? {
        zone = zoneAt(info.location)
        return DropProposal(operation: .move)
    }

    func dropExited(info: DropInfo) {
        zone = nil
    }

    func performDrop(info: DropInfo) -> Bool {
        let z = zoneAt(info.location)
        zone = nil
        guard let provider = info.itemProviders(for: [.plainText]).first else { return false }
        let deliver = onDrop
        _ = provider.loadObject(ofClass: String.self) { payload, _ in
            guard let payload else { return }
            DispatchQueue.main.async { MainActor.assumeIsolated { deliver(payload, z) } }
        }
        return true
    }
}

/// Accent highlight of the part of a tile a drop will take.
private struct DropZoneHighlight: View {
    let zone: String?
    let size: CGSize

    var body: some View {
        if let zone {
            let r = dropZoneRect(zone, in: size).insetBy(dx: 3, dy: 3)
            RoundedRectangle(cornerRadius: tileRadius - 2, style: .continuous)
                .fill(Color.accentColor.opacity(0.28))
                .overlay(
                    RoundedRectangle(cornerRadius: tileRadius - 2, style: .continuous)
                        .strokeBorder(Color.accentColor, lineWidth: 2)
                )
                .frame(width: max(r.width, 0), height: max(r.height, 0))
                .offset(x: r.minX, y: r.minY)
                .frame(width: size.width, height: size.height, alignment: .topLeading)
                .allowsHitTesting(false)
        }
    }
}

@MainActor
private struct EmptyDrop: View {
    let model: EditorModel
    let ref: TileRef
    @State private var zone: String?

    var body: some View {
        GeometryReader { geo in
            RoundedRectangle(cornerRadius: tileRadius, style: .continuous)
                .strokeBorder(style: StrokeStyle(lineWidth: 1.5, dash: [5, 4]))
                .foregroundStyle(zone != nil ? Color.accentColor : Color.secondary.opacity(0.6))
                .overlay {
                    Label("Drop an app", systemImage: "plus.app")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
                .overlay(DropZoneHighlight(zone: zone, size: geo.size))
                .contentShape(Rectangle())
                .onDrop(of: [.plainText], delegate: TileDropDelegate(size: geo.size, centerOnly: true, zone: $zone) { p, z in
                    model.drop(payload: p, on: ref, zone: z)
                })
        }
    }
}

@MainActor
private struct SplitView: View {
    let model: EditorModel
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
                    Label("Accordion", systemImage: "rectangle.stack")
                        .font(.caption2)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(Capsule().fill(.regularMaterial))
                        .padding(5)
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
    let model: EditorModel
    let slot: AppSlot
    let ref: TileRef
    @State private var zone: String?
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        GeometryReader { geo in
            let selected = model.selection == ref
            let shape = RoundedRectangle(cornerRadius: tileRadius, style: .continuous)
            let side = max(min(40, geo.size.width * 0.4, geo.size.height * 0.4), 12)
            shape
                .fill(Color.accentColor.opacity(colorScheme == .dark ? 0.22 : 0.12))
                .overlay(shape.strokeBorder(Color.primary.opacity(0.12), lineWidth: 1))
                .overlay {
                    VStack(spacing: 4) {
                        AppIconView(bundleId: slot.bundle_id, size: side)
                        if geo.size.height > 60 {
                            Text(AppIcons.name(for: slot.bundle_id)).font(.callout).lineLimit(1)
                        }
                        if let t = slot.title, !t.isEmpty, geo.size.height > 90 {
                            Text(t).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                        }
                    }
                    .padding(4)
                }
                .overlay(DropZoneHighlight(zone: zone, size: geo.size))
                // Drawn inside the tile, so it isn't clipped by its neighbours.
                .overlay(shape.strokeBorder(Color.accentColor, lineWidth: selected ? 2.5 : 0))
                .contentShape(shape)
                .onTapGesture { model.selection = ref }
                .draggable(DragPayload.tile(ref)) {
                    AppIconView(bundleId: slot.bundle_id, size: 32)
                }
                .onDrop(of: [.plainText], delegate: TileDropDelegate(size: geo.size, zone: $zone) { p, z in
                    model.drop(payload: p, on: ref, zone: z)
                })
                .contextMenu {
                    Button("Toggle Split Direction") { model.toggleOrientation(ref) }
                    if let parent = ref.parent {
                        Button("Toggle Accordion") { model.toggleLayout(parent) }
                    }
                    Divider()
                    Button("Remove Tile", role: .destructive) { model.remove(ref) }
                }
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(AppIcons.name(for: slot.bundle_id))
                .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
        }
    }
}
