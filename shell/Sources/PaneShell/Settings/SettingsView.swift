import AppKit
import SwiftUI

/// v1 settings: where the config lives, whether it parses, permissions.
@MainActor
struct SettingsView: View {
    @State private var configError: String?
    @State private var axGranted = false
    @State private var message: String?

    private var supportDir: URL {
        if let d = ProcessInfo.processInfo.environment["PANE_SUPPORT_DIR"] { return URL(fileURLWithPath: d) }
        return FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/Pane")
    }

    private var configURL: URL { supportDir.appendingPathComponent("config.toml") }

    var body: some View {
        Form {
            Section("Configuration") {
                LabeledContent("File") {
                    Text(configURL.path).textSelection(.enabled).font(.system(.body, design: .monospaced))
                }
                if let e = configError {
                    Label(e, systemImage: "exclamationmark.triangle").foregroundStyle(.red)
                } else {
                    Label("Config is valid", systemImage: "checkmark.circle").foregroundStyle(.green)
                }
                HStack {
                    Button("Open in Editor") { NSWorkspace.shared.open(configURL) }
                    Button("Reload") {
                        switch PaneCore.shared.reload() {
                        case .ok:
                            configError = nil
                            message = "Reloaded"
                        case let .error(m):
                            configError = m
                        }
                    }
                    Button("Show Folder") { NSWorkspace.shared.activateFileViewerSelecting([configURL]) }
                }
                if let m = message { Text(m).foregroundStyle(.secondary) }
            }
            Section("Permissions") {
                LabeledContent("Accessibility") {
                    Text(axGranted ? "Granted" : "Not granted").foregroundStyle(axGranted ? .green : .red)
                }
                if !axGranted {
                    Button("Open Accessibility Settings") { Permissions.openAccessibilitySettings() }
                }
            }
            Section("Recovery") {
                Text("If windows ever go missing, use “Restore All Windows” in the menu or run `pane recover --all-apps`.")
                    .foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
        .frame(width: 520, height: 460)
        .padding(.vertical, 8)
        .onAppear {
            axGranted = Permissions.accessibilityGranted
            configError = PaneCore.shared.snapshot?.config_error
        }
    }
}
