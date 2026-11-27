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
@Observable
final class EditorModel {
    var profile: Profile
    var snapshot: Snapshot?
    var selection: TileRef?
    /// Transient feedback, shown in a capsule over the canvas.
    var status: String?
    /// Selected workspace tab per display box (group key -> entry index).
    var selectedTab: [String: Int] = [:]
    /// Shown instead of the canvas while comparing a saved profile with the desk.
    var diff: ProfileDiff?
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

    func layout(at t: TileRef) -> String {
        if case let .container(_, l, _, _)? = profile.display[safe: t.display]?.tree.node(at: t.path) { return l }
        return "tiles"
    }

    func setOrientation(_ t: TileRef, _ orientation: String) {
        guard orientation != self.orientation(at: t) else { return }
        perform(["op": "set_orientation", "at": t.json, "orientation": orientation])
    }

    /// Whether the open profile is saved under its name (Compare needs that).
    var isSaved: Bool { (snapshot?.profiles ?? []).contains(profile.name) }

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
            // SwiftUI's .toolbar and .navigationTitle drive the NSWindow.
            host.sceneBridgingOptions = [.toolbars, .title]
            let w = NSWindow(contentViewController: host)
            w.styleMask = [.titled, .closable, .resizable, .miniaturizable, .fullSizeContentView]
            w.toolbarStyle = .unified
            w.isReleasedWhenClosed = false
            w.setContentSize(NSSize(width: 1180, height: 720))
            w.center()
            w.setFrameAutosaveName("PaneProfiles")
            _ = w.setFrameUsingName("PaneProfiles")
            window = w
            if model.profile.display.isEmpty { model.newProfile() }
        }
        NSApp.activate()
        window?.makeKeyAndOrderFront(nil)
    }
}

@MainActor
struct EditorRootView: View {
    @Bindable var model: EditorModel
    @State private var inspectorShown = true
    @State private var renaming = false

    /// The window title is the profile name; editing it renames the profile.
    private var title: Binding<String> {
        Binding(get: { model.profile.name }, set: { model.rename($0) })
    }

    var body: some View {
        NavigationSplitView {
            AppPaletteView()
                .navigationSplitViewColumnWidth(min: 200, ideal: 240, max: 320)
        } detail: {
            Group {
                if let d = model.diff {
                    ProfileDiffView(model: model, diff: d)
                } else {
                    DeskCanvasView(model: model)
                }
            }
            .frame(minWidth: 500)
            .statusCapsule($model.status)
            .inspector(isPresented: model.diff == nil ? $inspectorShown : .constant(false)) {
                InspectorView(model: model)
                    .inspectorColumnWidth(min: 240, ideal: 270, max: 340)
            }
        }
        .navigationTitle(title)
        .toolbar { toolbar }
        .frame(minWidth: 960, minHeight: 560)
        .sheet(isPresented: $renaming) {
            RenameSheet(name: model.profile.name) { model.rename($0) }
        }
    }

    @ToolbarContentBuilder
    private var toolbar: some ToolbarContent {
        ToolbarItemGroup(placement: .navigation) {
            Menu {
                let profiles = model.snapshot?.profiles ?? []
                if profiles.isEmpty {
                    Text("No Saved Profiles")
                } else {
                    Section("Open") {
                        ForEach(profiles, id: \.self) { p in
                            Button(p) { model.load(p) }
                        }
                    }
                }
                Divider()
                Button("Rename…") { renaming = true }
            } label: {
                Label("Profiles", systemImage: "folder")
            }
            .help("Open a saved profile")
            Button { model.newProfile() } label: { Label("New", systemImage: "plus") }
                .help("Start a profile for the connected displays")
            Button { model.capture() } label: { Label("Capture", systemImage: "camera.viewfinder") }
                .help("Capture the current desk")
            Button { model.compareWithDesk() } label: { Label("Compare", systemImage: "arrow.left.arrow.right") }
                .disabled(!model.isSaved || model.diff != nil)
                .help("Show how the current desk differs from the saved profile, and update it")
        }
        ToolbarItemGroup {
            Button { model.undo() } label: { Label("Undo", systemImage: "arrow.uturn.backward") }
                .disabled(!model.canUndo)
                .keyboardShortcut("z", modifiers: .command)
            Button { model.redo() } label: { Label("Redo", systemImage: "arrow.uturn.forward") }
                .disabled(!model.canRedo)
                .keyboardShortcut("z", modifiers: [.command, .shift])
        }
        ToolbarSpacer(.flexible)
        if model.diff != nil {
            ToolbarItemGroup(placement: .primaryAction) {
                Button("Cancel") { model.diff = nil }
                    .keyboardShortcut(.cancelAction)
                Button("Edit Before Saving") { model.editDiff() }
                    .help("Open the updated profile in the editor without saving it")
                Button("Update Profile") { model.acceptDiff() }
                    .buttonStyle(.glassProminent)
                    .keyboardShortcut(.defaultAction)
            }
        } else {
            ToolbarItem {
                Menu {
                    Section("By Position") {
                        ForEach(["left", "center", "right", "main"], id: \.self) { r in
                            Button(r.capitalized) { model.addDisplay(ref: r) }
                        }
                    }
                    if let displays = model.snapshot?.displays, !displays.isEmpty {
                        Section("Connected") {
                            ForEach(displays, id: \.id) { d in
                                Button(d.name) { model.addDisplay(ref: "uuid:\(d.id)") }
                            }
                        }
                    }
                } label: {
                    Label("Add Display", systemImage: "plus.rectangle.on.rectangle")
                }
                .help("Add a display to the profile")
            }
            ToolbarItemGroup(placement: .primaryAction) {
                Menu {
                    Button("Save as Startup Profile") { model.save(startup: true) }
                } label: {
                    Label("Save", systemImage: "square.and.arrow.down")
                } primaryAction: {
                    model.save(startup: false)
                }
                .keyboardShortcut("s", modifiers: .command)
                .help("Save the profile (⌘S)")
                Button("Apply") { model.apply() }
                    .buttonStyle(.glassProminent)
                    .help("Arrange the desk like this profile")
            }
            ToolbarItem(placement: .primaryAction) {
                Button { inspectorShown.toggle() } label: { Label("Inspector", systemImage: "sidebar.right") }
                    .help(inspectorShown ? "Hide the inspector" : "Show the inspector")
            }
        }
    }
}

/// Rename the open profile (the editable window title does the same).
@MainActor
private struct RenameSheet: View {
    @State var name: String
    let onRename: (String) -> Void
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Rename Profile").font(.headline)
            TextField("Profile name", text: $name)
                .textFieldStyle(.roundedBorder)
                .onSubmit(commit)
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Rename", action: commit)
                    .keyboardShortcut(.defaultAction)
                    .disabled(name.trimmingCharacters(in: .whitespaces).isEmpty)
            }
        }
        .padding(20)
        .frame(width: 320)
    }

    private func commit() {
        onRename(name)
        dismiss()
    }
}
