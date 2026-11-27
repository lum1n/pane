import AppKit
import Observation

/// A GitHub release, from `GET /repos/{owner}/{repo}/releases/latest`
/// (which already leaves out drafts and pre-releases).
struct Release: Decodable, Equatable {
    var tag_name: String
    var name: String?
    var html_url: String

    var version: String {
        tag_name.hasPrefix("v") || tag_name.hasPrefix("V") ? String(tag_name.dropFirst()) : tag_name
    }
}

/// Looks for a newer release on GitHub. It only tells you: it never
/// downloads or installs anything, so it needs no signing keys or appcast.
/// When on, it checks at launch and then at most once a day.
@MainActor
@Observable
final class UpdateChecker {
    static let shared = UpdateChecker()

    static let releasesPage = URL(string: "https://github.com/lum1n/pane/releases")!
    private static let latestAPI = URL(string: "https://api.github.com/repos/lum1n/pane/releases/latest")!
    private static let interval: TimeInterval = 24 * 60 * 60

    private enum Key {
        static let automatic = "checkForUpdates"
        static let lastCheck = "lastUpdateCheck"
        static let notified = "notifiedUpdateVersion"
    }

    /// Check automatically (default on).
    var automatic: Bool {
        get { automaticStorage }
        set {
            automaticStorage = newValue
            UserDefaults.standard.set(newValue, forKey: Key.automatic)
            if newValue { checkIfDue() }
        }
    }
    private var automaticStorage: Bool

    /// A release newer than the running app, if the last check found one.
    private(set) var available: Release?
    private(set) var checking = false
    /// Result of the last check, for Settings.
    private(set) var status: String?

    @ObservationIgnored private var timer: Timer?

    var lastCheck: Date? { UserDefaults.standard.object(forKey: Key.lastCheck) as? Date }

    private init() {
        automaticStorage = UserDefaults.standard.object(forKey: Key.automatic) as? Bool ?? true
    }

    /// Call once at launch.
    func start() {
        checkIfDue()
        // Hourly wake-up; it only goes to the network once a day.
        timer = Timer.scheduledTimer(withTimeInterval: 60 * 60, repeats: true) { _ in
            MainActor.assumeIsolated { UpdateChecker.shared.checkIfDue() }
        }
    }

    private func checkIfDue() {
        guard automatic, !checking else { return }
        if let last = lastCheck, Date().timeIntervalSince(last) < Self.interval { return }
        check(userInitiated: false)
    }

    /// Ask GitHub for the latest release. A check you start yourself always
    /// reports back in an alert; an automatic one only shows a notice the
    /// first time it sees a new version.
    func check(userInitiated: Bool) {
        guard !checking else { return }
        checking = true
        status = "Checking…"
        var req = URLRequest(url: Self.latestAPI)
        req.setValue("application/vnd.github+json", forHTTPHeaderField: "Accept")
        req.setValue("Pane/\(AppVersion.current)", forHTTPHeaderField: "User-Agent")
        req.timeoutInterval = 20
        Task {
            let result = await Self.fetch(req)
            finish(result, userInitiated: userInitiated)
        }
    }

    private enum Outcome {
        case release(Release)
        case noReleases
        case failed(String)
    }

    nonisolated private static func fetch(_ req: URLRequest) async -> Outcome {
        do {
            let (data, response) = try await URLSession.shared.data(for: req)
            let code = (response as? HTTPURLResponse)?.statusCode ?? 0
            if code == 404 { return .noReleases }
            guard code == 200 else { return .failed("GitHub answered HTTP \(code)") }
            return .release(try JSONDecoder().decode(Release.self, from: data))
        } catch {
            return .failed(error.localizedDescription)
        }
    }

    private func finish(_ outcome: Outcome, userInitiated: Bool) {
        checking = false
        let current = AppVersion.current
        switch outcome {
        case let .release(r) where AppVersion.isNewer(r.tag_name, than: current):
            UserDefaults.standard.set(Date(), forKey: Key.lastCheck)
            available = r
            status = "Pane \(r.version) is available (you have \(current))."
            if userInitiated {
                offer(r)
            } else if UserDefaults.standard.string(forKey: Key.notified) != r.tag_name {
                UserDefaults.standard.set(r.tag_name, forKey: Key.notified)
                NoticeHUD.shared.show("Pane \(r.version) is available: see the menu")
            }
        case .release, .noReleases:
            UserDefaults.standard.set(Date(), forKey: Key.lastCheck)
            available = nil
            status = "Pane \(current) is the latest version."
            if userInitiated { inform("You're up to date", status!) }
        case let .failed(m):
            // lastCheck stays, so an automatic check tries again within the hour.
            status = "Couldn't check for updates: \(m)"
            if userInitiated { inform("Couldn't check for updates", m) }
        }
    }

    /// Alert offering to open the release page.
    func offer(_ r: Release) {
        let a = NSAlert()
        a.messageText = "Pane \(r.version) is available"
        a.informativeText = "You have \(AppVersion.current). The release page has the download and what changed."
        a.addButton(withTitle: "Open Release Page")
        a.addButton(withTitle: "Later")
        NSApp.activate()
        if a.runModal() == .alertFirstButtonReturn {
            NSWorkspace.shared.open(URL(string: r.html_url) ?? Self.releasesPage)
        }
    }

    private func inform(_ title: String, _ text: String) {
        let a = NSAlert()
        a.messageText = title
        a.informativeText = text
        NSApp.activate()
        a.runModal()
    }
}
