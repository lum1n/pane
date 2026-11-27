import AppKit
import Combine
import SwiftUI

/// Bindings that write straight through to config.toml.
@MainActor
extension SettingsModel {
    func number(_ key: String, _ get: @escaping (PaneSettings) -> Double) -> Binding<Double> {
        Binding(get: { self.settings.map(get) ?? 0 }, set: { self.set(key, $0) })
    }

    func toggle(_ key: String, _ get: @escaping (PaneSettings) -> Bool) -> Binding<Bool> {
        Binding(get: { self.settings.map(get) ?? false }, set: { self.set(key, $0) })
    }
}

@MainActor
struct GeneralPane: View {
    let model: SettingsModel
    @State private var axGranted = Permissions.accessibilityGranted
    @State private var login = LaunchAtLogin.State.off
    @State private var loginError: String?

    private var updates: UpdateChecker { .shared }

    private var supportDir: URL {
        if let d = ProcessInfo.processInfo.environment["PANE_SUPPORT_DIR"] { return URL(fileURLWithPath: d) }
        return FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/Pane")
    }

    private var configURL: URL { supportDir.appendingPathComponent("config.toml") }

    var body: some View {
        Form {
            Section { header }
            startupSection
            if let s = model.settings {
                Section {
                    Toggle("Show app icons on the tiles", isOn: Binding(
                        get: { model.settings?.menu_bar_app_icons ?? true },
                        set: { model.set("menu_bar_app_icons", $0) }
                    ))
                } header: {
                    Text("Menu Bar")
                } footer: {
                    footer("Click a tile in the menu bar icon to focus its window. Name workspaces in config.toml under [workspace_names].")
                }
                placementSection(s)
                profileSection(s)
            }
            configSection
            recoverySection
        }
        .settingsPane(model, height: 680)
        .onAppear {
            axGranted = Permissions.accessibilityGranted
            login = LaunchAtLogin.state
        }
        // Approving in System Settings happens outside Pane; pick it up on return.
        .onReceive(NotificationCenter.default.publisher(for: NSWindow.didBecomeKeyNotification)) { _ in
            login = LaunchAtLogin.state
            axGranted = Permissions.accessibilityGranted
        }
    }

    private func footer(_ text: String) -> some View {
        Text(text).font(.footnote).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
    }

    // MARK: Sections

    private var header: some View {
        HStack(spacing: 12) {
            Image(nsImage: NSApp.applicationIconImage)
                .resizable()
                .frame(width: 48, height: 48)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                Text("Pane").font(.title3.weight(.semibold))
                Text("Version \(AppVersion.current)").foregroundStyle(.secondary)
            }
            Spacer()
            if axGranted {
                Label("Accessibility On", systemImage: "checkmark.circle.fill")
                    .foregroundStyle(.green)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 4)
                    .background(Capsule().fill(Color.green.opacity(0.15)))
            } else {
                HStack(spacing: 8) {
                    Label("Accessibility Off", systemImage: "exclamationmark.triangle.fill")
                        .foregroundStyle(.orange)
                        .padding(.horizontal, 10)
                        .padding(.vertical, 4)
                        .background(Capsule().fill(Color.orange.opacity(0.15)))
                    Button("Open Settings") { Permissions.openAccessibilitySettings() }
                }
            }
        }
        .padding(.vertical, 4)
    }

    private var startupSection: some View {
        Section {
            Toggle("Open Pane at login", isOn: Binding(
                get: { login != .off },
                set: { on in
                    loginError = LaunchAtLogin.set(on)
                    login = LaunchAtLogin.state
                }
            ))
            if login == .needsApproval {
                LabeledContent {
                    Button("Open Login Items") { LaunchAtLogin.openSystemSettings() }
                } label: {
                    Label("Allow Pane in Login Items to finish", systemImage: "exclamationmark.triangle.fill")
                        .foregroundStyle(.orange)
                }
            }
            if let e = loginError {
                Label(e, systemImage: "exclamationmark.triangle.fill").foregroundStyle(.red)
            }
            Toggle("Check for updates automatically", isOn: Bindable(updates).automatic)
            LabeledContent {
                HStack(spacing: 8) {
                    if updates.checking { ProgressView().controlSize(.small) }
                    Button("Check Now") { updates.check(userInitiated: true) }
                        .disabled(updates.checking)
                }
            } label: {
                Text("Updates")
                Text(updates.status ?? "You have Pane \(AppVersion.current).")
            }
            if let r = updates.available {
                LabeledContent("Pane \(r.version) is available") {
                    Link("Open Release Page", destination: URL(string: r.html_url) ?? UpdateChecker.releasesPage)
                }
            }
        } header: {
            Text("Startup and Updates")
        } footer: {
            footer("Updates come from the GitHub releases page. Pane only tells you about them; it never installs anything.")
        }
    }

    private func placementSection(_ s: PaneSettings) -> some View {
        Section {
            Toggle("Put relaunched windows back in their tiles", isOn: model.toggle("restore_placements") { $0.restore_placements })
            LabeledContent("Remember placements for") {
                Stepper(value: Binding(
                    get: { model.settings?.placement_memory_days ?? 30 },
                    set: { model.set("placement_memory_days", $0) }
                ), in: 1...365) {
                    Text("\(s.placement_memory_days) days").monospacedDigit()
                }
            }
            .disabled(!s.restore_placements)
        } header: {
            Text("Window Placement")
        }
    }

    private func profileSection(_ s: PaneSettings) -> some View {
        Section {
            Picker("Default profile", selection: Binding(
                get: { s.startup_profile ?? "" },
                set: { $0.isEmpty ? model.clear("startup_profile") : model.set("startup_profile", $0) }
            )) {
                Text("None").tag("")
                ForEach(PaneCore.shared.snapshot?.profiles ?? [], id: \.self) { Text($0).tag($0) }
            }
        } header: {
            Text("Profiles")
        } footer: {
            footer("Applied at launch when no profile is active. A profile you apply stays active, also across restarts, until you apply another.")
        }
    }

    /// The config path with the home folder shown as `~`.
    private var configPathForDisplay: String {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        let path = configURL.path
        return path.hasPrefix(home) ? "~" + path.dropFirst(home.count) : path
    }

    private var configSection: some View {
        Section {
            LabeledContent {
                HStack(spacing: 6) {
                    Button { model.reload() } label: { Image(systemName: "arrow.clockwise") }
                        .help("Reload config.toml from disk")
                        .accessibilityLabel("Reload")
                    Button { NSWorkspace.shared.activateFileViewerSelecting([configURL]) } label: { Image(systemName: "folder") }
                        .help("Show in Finder")
                        .accessibilityLabel("Show in Finder")
                    Button("Open") { NSWorkspace.shared.open(configURL) }
                        .help("Open config.toml in your editor")
                }
            } label: {
                Text("config.toml")
                Text(configPathForDisplay)
                    .textSelection(.enabled)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            if let e = model.configError {
                Label(e, systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(.red)
                    .fixedSize(horizontal: false, vertical: true)
            }
        } header: {
            Text("Configuration File")
        } footer: {
            footer("Displays, workspaces and window rules are edited in the file. Changes made here are written to it, keeping your comments.")
        }
    }

    private var recoverySection: some View {
        Section {
            LabeledContent {
                Button("Restore All Windows") { PaneCore.shared.recover() }
            } label: {
                Text("Missing windows")
                Text("Brings every window back on screen. From a terminal: `pane recover --all-apps`")
            }
        } header: {
            Text("Recovery")
        }
    }
}
