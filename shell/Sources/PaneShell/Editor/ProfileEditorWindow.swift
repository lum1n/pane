import AppKit
import SwiftUI

/// Address of a tile: display index in the profile plus index path.
struct TileRef: Equatable, Hashable {
    var display: Int
    var path: [Int]

    var json: [String: Any] { ["display": display, "path": path] }
    var parent: TileRef? { path.isEmpty ? nil : TileRef(display: display, path: Array(path.dropLast())) }
}

/// Editor state. Every tree edit is a `profile-edit` round trip to the Rust
/// core, so the editor and live layouts share one tree implementation.
@MainActor
final class EditorModel: ObservableObject {
    @Published var profile: Profile
    @Published var snapshot: Snapshot?
    @Published var selection: TileRef?
    @Published var status: String?
    /// Selected workspace tab per display box (group key -> entry index).
    @Published var selectedTab: [String: Int] = [:]
    /// Shown instead of the canvas while comparing a saved profile with the desk.
    @Published var diff: ProfileDiff?
    private var undoStack: [Profile] = []
    private var redoStack: [Profile] = []

    init() {
        profile = Profile(name: "Untitled", display: [])
    }

    var canUndo: Bool { !undoStack.isEmpty }
    var canRedo: Bool { !redoStack.isEmpty }

    private func replace(with p: Profile) {
        guard p != profile else { return }
        undoStack.append(profile)
        if undoStack.count > 200 { undoStack.removeFirst() }
        redoStack.removeAll()
        profile = p
    }

    func perform(_ op: [String: Any]) {
        switch PaneCore.shared.edit(profile, op) {
        case let .success(p):
            replace(with: p)
            status = nil
        case let .failure(e):
            status = e.message
        }
    }

    func undo() {
        guard let p = undoStack.popLast() else { return }
        redoStack.append(profile)
        profile = p
        selection = nil
    }

    func redo() {
        guard let p = redoStack.popLast() else { return }
        undoStack.append(profile)
        profile = p
        selection = nil
    }

    func newProfile() {
        var p = Profile(name: "Untitled", display: [])
        for (i, d) in (snapshot?.displays ?? []).sorted(by: { $0.rank < $1.rank }).enumerated() {
            p.display.append(ProfileDisplay(display: defaultRef(rank: i, count: snapshot?.displays.count ?? 1, id: d.id),
                                            workspace: d.visible_workspace ?? "1", tree: .empty, visible: true))
        }
        replace(with: p)
        selection = nil
    }

    private func defaultRef(rank: Int, count: Int, id: String) -> String {
        switch (count, rank) {
        case (1, _): return "main"
        case (2, 0), (3, 0): return "left"
        case (2, _), (3, 2): return "right"
        case (3, _): return "center"
        default: return "uuid:\(id)"
        }
    }

    func capture() {
        switch PaneCore.shared.capture(name: profile.name) {
        case let .success(p):
            replace(with: p)
            status = "Captured the current desk"
        case let .failure(e): status = e.message
        }
    }

    /// Compare saved profile `name` (default: the open one) with the desk.
    func compareWithDesk(_ name: String? = nil) {
        switch PaneCore.shared.profileDiff(name: name ?? profile.name) {
        case let .success(d):
            diff = d
            status = d.drifted ? nil : "The desk matches “\(d.name)”"
        case let .failure(e): status = e.message
        }
    }

    /// Save the profile as updated from the desk and open it.
    func acceptDiff() {
        guard let d = diff else { return }
        switch PaneCore.shared.save(d.updated) {
        case .ok:
            replace(with: d.updated)
            selection = nil
            selectedTab = [:]
            diff = nil
            status = "Updated “\(d.name)” from the current desk"
        case let .error(m): status = m
        }
    }

    /// Open the updated profile in the editor without saving it.
    func editDiff() {
        guard let d = diff else { return }
        replace(with: d.updated)
        selection = nil
        selectedTab = [:]
        diff = nil
        status = "Updated from the current desk; Save to keep it"
    }

    func load(_ name: String) {
        switch PaneCore.shared.load(profile: name) {
        case let .success(p):
            replace(with: p)
            selection = nil
            status = "Opened “\(name)”"
        case let .failure(e): status = e.message
        }
    }

    func save(startup: Bool) {
        switch PaneCore.shared.save(profile, startup: startup) {
        case .ok: status = startup ? "Saved; applies at startup" : "Saved “\(profile.name)”"
        case let .error(m): status = m
        }
    }

    func apply() {
        switch PaneCore.shared.apply(profile) {
        case .ok: status = "Applied"
        case let .error(m): status = m
        }
    }

    func rename(_ name: String) {
        let n = name.trimmingCharacters(in: .whitespaces)
        guard !n.isEmpty, n != profile.name else { return }
        perform(["op": "rename", "name": n])
    }

    // MARK: tree operations

    func drop(payload: String, on target: TileRef, zone: String) {
        if let bundle = DragPayload.app(payload) {
            perform(["op": "drop", "at": target.json, "zone": zone, "slot": ["bundle_id": bundle]])
        } else if let from = DragPayload.tile(payload) {
            guard from != target else { return }
            if zone == "center" {
                perform(["op": "swap", "a": from.json, "b": target.json])
            } else {
                perform(["op": "move_tile", "from": from.json, "to": target.json, "zone": zone])
            }
            selection = nil
        }
    }

    func remove(_ t: TileRef) {
        perform(["op": "remove", "at": t.json])
        selection = nil
    }

    func toggleOrientation(_ t: TileRef) {
        guard let node = profile.display[safe: t.display]?.tree.node(at: t.path) else { return }
        // A leaf flips its parent container; a container flips itself.
        let target: TileRef
        if case .container = node { target = t } else { target = t.parent ?? t }
        let current = orientation(at: target)
        perform(["op": "set_orientation", "at": target.json, "orientation": current == "horizontal" ? "vertical" : "horizontal"])
    }

    func toggleLayout(_ t: TileRef) {
        perform(["op": "toggle_layout", "at": t.json])
    }

    func orientation(at t: TileRef) -> String {
        if case let .container(o, _, _, _)? = profile.display[safe: t.display]?.tree.node(at: t.path) { return o }
        return "horizontal"
    }

    /// Move the divider between children `i` and `i + 1` of a container.
    func setWeights(container: TileRef, index i: Int, _ a: Double, _ b: Double) {
        guard case let .success(p1) = PaneCore.shared.edit(profile, [
            "op": "set_weight", "at": TileRef(display: container.display, path: container.path + [i]).json, "weight": a,
        ]) else { return }
        switch PaneCore.shared.edit(p1, [
            "op": "set_weight", "at": TileRef(display: container.display, path: container.path + [i + 1]).json, "weight": b,
        ]) {
        case let .success(p2): replace(with: p2)
        case let .failure(e): status = e.message
        }
    }

    func setSlot(_ t: TileRef, _ slot: AppSlot) {
        var s: [String: Any] = ["bundle_id": slot.bundle_id]
        if let title = slot.title, !title.isEmpty { s["title"] = title }
        if let args = slot.launch_args, !args.isEmpty { s["launch_args"] = args }
        perform(["op": "set_slot", "at": t.json, "slot": s])
    }

    func setWorkspace(display: Int, _ name: String) {
        perform(["op": "set_workspace", "display": display, "workspace": name])
    }

    func addDisplay(ref: String) {
        let used = Set(profile.display.filter { $0.display == ref }.map(\.workspace))
        let ws = (1...99).map(String.init).first { !used.contains($0) } ?? "1"
        perform(["op": "add_display", "display_ref": ref, "workspace": ws])
    }

    /// Add the next free workspace number to the display of entry `i`.
    func addWorkspace(like i: Int, groupKey: String) {
        guard let ref = profile.display[safe: i]?.display else { return }
        let before = profile.display.count
        addDisplay(ref: ref)
        if profile.display.count > before {
            selectedTab[groupKey] = profile.display.count - 1
        }
    }

    func setVisible(_ i: Int) {
        perform(["op": "set_visible", "display": i])
    }

    /// Remove profile entries (highest index first so indices stay valid).
    func removeEntries(_ indices: [Int]) {
        var p = profile
        for i in indices.sorted(by: >) {
            switch PaneCore.shared.edit(p, ["op": "remove_display", "display": i]) {
            case let .success(np): p = np
            case let .failure(e):
                status = e.message
                return
            }
        }
        replace(with: p)
        selection = nil
        selectedTab = [:]
    }
}

extension Array {
    subscript(safe i: Int) -> Element? { indices.contains(i) ? self[i] : nil }
}

@MainActor
final class ProfileEditorWindow {
    let model = EditorModel()
    private var window: NSWindow?

    func show() {
        if window == nil {
            let host = NSHostingController(rootView: EditorRootView(model: model))
            let w = NSWindow(contentViewController: host)
            w.title = "Pane Profiles"
            w.setContentSize(NSSize(width: 1180, height: 720))
            w.styleMask = [.titled, .closable, .resizable, .miniaturizable]
            w.isReleasedWhenClosed = false
            w.center()
            window = w
            if model.profile.display.isEmpty { model.newProfile() }
        }
        NSApp.activate(ignoringOtherApps: true)
        window?.makeKeyAndOrderFront(nil)
    }
}

@MainActor
struct EditorRootView: View {
    @ObservedObject var model: EditorModel
    @State private var name = ""

    var body: some View {
        VStack(spacing: 0) {
            toolbar
            Divider()
            if let d = model.diff {
                ProfileDiffView(model: model, diff: d)
            } else {
                HSplitView {
                    AppPaletteView()
                        .frame(minWidth: 200, idealWidth: 230, maxWidth: 320)
                    DeskCanvasView(model: model)
                        .frame(minWidth: 500)
                    InspectorView(model: model)
                        .frame(minWidth: 220, idealWidth: 250, maxWidth: 320)
                }
            }
            if let s = model.status {
                Divider()
                HStack {
                    Text(s).font(.callout).foregroundStyle(.secondary)
                    Spacer()
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 6)
            }
        }
        .frame(minWidth: 960, idealWidth: 1180, minHeight: 560, idealHeight: 720)
        .onAppear { name = model.profile.name }
        .onChange(of: model.profile.name) { _, n in name = n }
    }

    private var toolbar: some View {
        HStack(spacing: 8) {
            TextField("Profile name", text: $name)
                .textFieldStyle(.roundedBorder)
                .frame(width: 200)
                .onSubmit { model.rename(name) }
            Menu("Open") {
                ForEach(model.snapshot?.profiles ?? [], id: \.self) { p in
                    Button(p) { model.load(p) }
                }
                if (model.snapshot?.profiles ?? []).isEmpty { Text("No saved profiles") }
            }
            .frame(width: 80)
            Button("New") { model.newProfile() }
            Button("Capture Current") { model.capture() }
            Button("Compare with Desk") { model.compareWithDesk() }
                .disabled(!(model.snapshot?.profiles ?? []).contains(model.profile.name))
                .help("Show how the current desk differs from the saved profile, and update it")
            Divider().frame(height: 18)
            Button { model.undo() } label: { Image(systemName: "arrow.uturn.backward") }
                .disabled(!model.canUndo)
                .keyboardShortcut("z", modifiers: .command)
            Button { model.redo() } label: { Image(systemName: "arrow.uturn.forward") }
                .disabled(!model.canRedo)
                .keyboardShortcut("z", modifiers: [.command, .shift])
            Spacer()
            Button("Save") { model.rename(name); model.save(startup: false) }
                .keyboardShortcut("s", modifiers: .command)
            Button("Save as Startup") { model.rename(name); model.save(startup: true) }
            Button("Apply") { model.apply() }
                .buttonStyle(.borderedProminent)
        }
        .padding(10)
    }
}
