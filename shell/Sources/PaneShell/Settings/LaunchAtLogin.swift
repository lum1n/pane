import Foundation
import ServiceManagement

/// Open Pane at login, through `SMAppService.mainApp` (macOS 13+). The state
/// lives with the system (System Settings > General > Login Items), not in
/// config.toml, so the toggle always shows what macOS will actually do.
@MainActor
enum LaunchAtLogin {
    enum State: Equatable {
        case on
        case off
        /// Registered, but the user has to allow it in System Settings.
        case needsApproval
    }

    static var state: State {
        switch SMAppService.mainApp.status {
        case .enabled: return .on
        case .requiresApproval: return .needsApproval
        default: return .off
        }
    }

    /// Register or unregister. Returns an error message, or nil on success.
    @discardableResult
    static func set(_ on: Bool) -> String? {
        do {
            if on {
                try SMAppService.mainApp.register()
            } else {
                try SMAppService.mainApp.unregister()
            }
            return nil
        } catch {
            return error.localizedDescription
        }
    }

    static func openSystemSettings() {
        SMAppService.openSystemSettingsLoginItems()
    }
}
