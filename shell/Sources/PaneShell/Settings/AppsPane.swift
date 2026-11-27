import AppKit
import SwiftUI
import UniformTypeIdentifiers

@MainActor
struct AppsPane: View {
    let model: SettingsModel

    var body: some View {
        Form {
            AppRuleSection(
                title: "Always Float",
                footer: "New windows of these apps start floating. Adding an app also floats its open windows.",
                apps: model.settings?.float_apps ?? [],
                onChange: { model.set("float_apps", $0) }
            )
            AppRuleSection(
                title: "Always Tile",
                footer: "For apps Pane floats on its own (unusual windows). Windows that can't be resized still float. Apps started from a terminal (like `cargo run`) are listed by their executable name; `pane query windows` shows why each window is tiled or floating.",
                apps: model.settings?.tile_apps ?? [],
                onChange: { model.set("tile_apps", $0) }
            )
        }
        .settingsPane(model, height: 600)
    }
}

/// Picks apps in an open panel, starting in /Applications.
@MainActor
enum AppChooser {
    static func choose(multiple: Bool) -> [String] {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.application]
        panel.directoryURL = URL(fileURLWithPath: "/Applications")
        panel.allowsMultipleSelection = multiple
        guard panel.runModal() == .OK else { return [] }
        return panel.urls.compactMap { Bundle(url: $0)?.bundleIdentifier }
    }
}

/// An app list setting: app rows (icon and name), then the +/− bar.
@MainActor
struct AppRuleSection: View {
    let title: String
    let footer: String
    let apps: [String]
    let onChange: ([String]) -> Void
    /// Offer "Enter Pattern…" (globs and executable names).
    var allowsPatterns = true
    @State private var selection: String?
    @State private var enteringId = false

    private func add(_ id: String) {
        let id = id.trimmingCharacters(in: .whitespaces)
        guard !id.isEmpty, !apps.contains(id) else { return }
        onChange(apps + [id])
    }

    private func remove(_ id: String?) {
        guard let id else { return }
        onChange(apps.filter { $0 != id })
        selection = nil
    }

    private func chooseApplication() {
        var list = apps
        for b in AppChooser.choose(multiple: true) where !list.contains(b) { list.append(b) }
        if list != apps { onChange(list) }
    }

    var body: some View {
        Section {
            if apps.isEmpty {
                ContentUnavailableView {
                    Label("No Apps", systemImage: "square.dashed")
                } description: {
                    Text("Click + to add one.")
                }
                .controlSize(.small)
                .frame(maxWidth: .infinity)
            } else {
                ForEach(apps, id: \.self) { id in
                    HStack(spacing: 8) {
                        AppIconView(bundleId: id, size: 24)
                        Text(AppIcons.name(for: id))
                        Spacer()
                    }
                    .modifier(SelectableRow(selected: selection == id))
                    .onTapGesture { selection = id }
                    .contextMenu { Button("Remove", role: .destructive) { remove(id) } }
                    .accessibilityAddTraits(selection == id ? .isSelected : [])
                }
            }
            ListEditBar(canRemove: selection.map { apps.contains($0) } ?? false, remove: { remove(selection) }) {
                Menu {
                    let running = KnownApp.running().filter { !apps.contains($0.id) }
                    if !running.isEmpty {
                        Section("Running Apps") {
                            ForEach(running, id: \.self) { a in
                                Button { add(a.id) } label: {
                                    Label { Text(a.name) } icon: { Image(nsImage: AppIcons.small(for: a.id)) }
                                }
                            }
                        }
                        Divider()
                    }
                    Button("Choose Application…") { chooseApplication() }
                    if allowsPatterns {
                        Button("Enter Pattern…") { enteringId = true }
                    }
                } label: {
                    Image(systemName: "plus").frame(width: 20, height: 18)
                }
            }
        } header: {
            Text(title)
        } footer: {
            // LocalizedStringKey, so the `code` spans render.
            Text(LocalizedStringKey(footer)).font(.footnote).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
        }
        .sheet(isPresented: $enteringId) {
            BundleIdSheet(title: "Add to “\(title)”") { add($0) }
        }
    }
}

/// Alert-style sheet asking for a bundle id or glob.
@MainActor
private struct BundleIdSheet: View {
    let title: String
    let onAdd: (String) -> Void
    @State private var text = ""
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(title).font(.headline)
            Text("A bundle ID like com.apple.Safari, an executable name, or a glob like com.jetbrains.*")
                .font(.callout)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            TextField("Bundle ID or glob", text: $text)
                .textFieldStyle(.roundedBorder)
                .onSubmit(commit)
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Add", action: commit)
                    .keyboardShortcut(.defaultAction)
                    .disabled(text.trimmingCharacters(in: .whitespaces).isEmpty)
            }
        }
        .padding(20)
        .frame(width: 360)
    }

    private func commit() {
        guard !text.trimmingCharacters(in: .whitespaces).isEmpty else { return }
        onAdd(text)
        dismiss()
    }
}
