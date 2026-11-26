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

    static func name(for bundleId: String) -> String {
        if let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundleId) {
            return FileManager.default.displayName(atPath: url.path).replacingOccurrences(of: ".app", with: "")
        }
        return bundleId
    }
}

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
        VStack(spacing: 0) {
            TextField("Search apps", text: $search)
                .textFieldStyle(.roundedBorder)
                .padding(8)
            List(filtered) { app in
                HStack(spacing: 8) {
                    Image(nsImage: AppIcons.icon(for: app.bundleId))
                        .resizable()
                        .frame(width: 22, height: 22)
                    VStack(alignment: .leading, spacing: 0) {
                        Text(app.name).lineLimit(1)
                        if app.running {
                            Text("running").font(.caption2).foregroundStyle(.secondary)
                        }
                    }
                    Spacer()
                }
                .contentShape(Rectangle())
                .draggable(DragPayload.app(bundle: app.bundleId)) {
                    Label(app.name, systemImage: "app")
                }
            }
            Text("Drag an app onto a tile: middle replaces, edges split.")
                .font(.caption)
                .foregroundStyle(.secondary)
                .padding(8)
        }
        .onAppear { if apps.isEmpty { apps = AppCatalog.load() } }
    }
}
