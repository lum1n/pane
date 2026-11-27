import Foundation

/// Version strings like "0.1.0", "v1.2" or "v1.3.0-beta.2". Kept free of
/// AppKit so it can be checked with plain `swiftc` anywhere.
enum AppVersion {
    /// The running app's CFBundleShortVersionString.
    static var current: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "0"
    }

    /// Numeric parts and whether there is a pre-release suffix.
    static func parse(_ s: String) -> (parts: [Int], prerelease: Bool) {
        var t = Substring(s.trimmingCharacters(in: .whitespaces))
        if t.first == "v" || t.first == "V" { t = t.dropFirst() }
        if let plus = t.firstIndex(of: "+") { t = t[..<plus] }
        var prerelease = false
        if let dash = t.firstIndex(of: "-") {
            prerelease = true
            t = t[..<dash]
        }
        let parts = t.split(separator: ".").map { Int($0.prefix { $0.isNumber }) ?? 0 }
        return (parts, prerelease)
    }

    /// True when `candidate` is a later version than `current`.
    /// "1.2" == "1.2.0"; "1.2.0-beta" < "1.2.0".
    static func isNewer(_ candidate: String, than current: String) -> Bool {
        let a = parse(candidate), b = parse(current)
        for i in 0..<max(a.parts.count, b.parts.count) {
            let x = i < a.parts.count ? a.parts[i] : 0
            let y = i < b.parts.count ? b.parts[i] : 0
            if x != y { return x > y }
        }
        return b.prerelease && !a.prerelease
    }
}
