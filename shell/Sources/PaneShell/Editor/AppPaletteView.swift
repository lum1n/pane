import AppKit
import SwiftUI

struct PaletteApp: Identifiable, Hashable {
    var id: String { bundleId }
    let bundleId: String
    let name: String
    let path: String
    let running: Bool
}

enum AppCatalog {
    /// Running regular apps first, then everything in the Applications folders.
    static func load() -> [PaletteApp] {
        var seen = Set<String>()
        var out: [PaletteApp] = []
        for app in NSWorkspace.shared.runningApplications where app.activationPolicy == .regular {
            guard let b = app.bundleIdentifier, let url = app.bundleURL, !seen.contains(b) else { continue }
            seen.insert(b)
            out.append(PaletteApp(bundleId: b, name: app.localizedName ?? url.deletingPathExtension().lastPathComponent, path: url.path, running: true))
        }
        let fm = FileManager.default
        let dirs = ["/Applications", "/System/Applications", "/System/Applications/Utilities", "/Applications/Utilities",
                    fm.homeDirectoryForCurrentUser.appendingPathComponent("Applications").path]
        var scanned: [PaletteApp] = []
        for dir in dirs {
            guard let items = try? fm.contentsOfDirectory(atPath: dir) else { continue }
            for item in items where item.hasSuffix(".app") {
                let path = (dir as NSString).appendingPathComponent(item)
                guard let b = Bundle(path: path)?.bundleIdentifier, !seen.contains(b) else { continue }
                seen.insert(b)
                scanned.append(PaletteApp(bundleId: b, name: fm.displayName(atPath: path).replacingOccurrences(of: ".app", with: ""), path: path, running: false))
            }
        }
        scanned.sort { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
        return out.sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending } + scanned
    }
}

enum AppIcons {
    private static var cache: [String: NSImage] = [:]

    @MainActor
    static func icon(for bundleId: String) -> NSImage {
        if let i = cache[bundleId] { return i }
        let image: NSImage
        if let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundleId) {
            image = NSWorkspace.shared.icon(forFile: url.path)
        } else {
            image = NSImage(systemSymbolName: "questionmark.app", accessibilityDescription: nil) ?? NSImage()
        }
        cache[bundleId] = image
        return image
    }

    /// A 16 pt copy of the icon, for menus.
    @MainActor
    static func small(for bundleId: String) -> NSImage {
        let image = (icon(for: bundleId).copy() as? NSImage) ?? NSImage()
        image.size = NSSize(width: 16, height: 16)
        return image
    }

    static func name(for bundleId: String) -> String {
        if let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundleId) {
            return FileManager.default.displayName(atPath: url.path).replacingOccurrences(of: ".app", with: "")
        }
        return bundleId
    }
}

/// The editor's sidebar: apps to drag onto tiles, running ones first.
@MainActor
struct AppPaletteView: View {
    @State private var apps: [PaletteApp] = []
    @State private var search = ""

    private var filtered: [PaletteApp] {
        let q = search.trimmingCharacters(in: .whitespaces)
        guard !q.isEmpty else { return apps }
        return apps.filter { $0.name.localizedCaseInsensitiveContains(q) || $0.bundleId.localizedCaseInsensitiveContains(q) }
    }

    var body: some View {
        let shown = filtered
        let running = shown.filter(\.running)
        let others = shown.filter { !$0.running }
        List {
            if !running.isEmpty {
                Section("Running") { ForEach(running) { row($0) } }
            }
            if !others.isEmpty {
                Section("Applications") { ForEach(others) { row($0) } }
            }
        }
        .listStyle(.sidebar)
        .overlay {
            if shown.isEmpty && !apps.isEmpty {
                ContentUnavailableView.search(text: search)
            }
        }
        .safeAreaInset(edge: .bottom) {
            Text("Drag an app onto a tile: the middle replaces it, the edges split it.")
                .font(.caption)
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
        }
        .searchable(text: $search, placement: .sidebar, prompt: "Search Apps")
        .onAppear { if apps.isEmpty { apps = AppCatalog.load() } }
    }

    private func row(_ app: PaletteApp) -> some View {
        Label {
            Text(app.name).lineLimit(1)
        } icon: {
            AppIconView(bundleId: app.bundleId, size: 20)
        }
        .contentShape(Rectangle())
        .draggable(DragPayload.app(bundle: app.bundleId)) {
            Label {
                Text(app.name)
            } icon: {
                AppIconView(bundleId: app.bundleId, size: 24)
            }
            .padding(6)
        }
    }
}
