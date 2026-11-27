import AppKit
import SwiftUI

/// First-run window that walks through granting Accessibility access. It
/// watches `AXIsProcessTrusted()` and flips to "You're all set" by itself.
@MainActor
final class WelcomeWindow {
    static let shared = WelcomeWindow()
    private var window: NSWindow?

    func show() {
        if window == nil {
            let host = NSHostingController(rootView: WelcomeView { [weak self] in self?.window?.close() })
            host.sizingOptions = .preferredContentSize
            let w = NSWindow(contentViewController: host)
            w.styleMask = [.titled, .closable, .fullSizeContentView]
            w.titlebarAppearsTransparent = true
            w.titleVisibility = .hidden
            w.title = "Welcome to Pane"
            w.isReleasedWhenClosed = false
            w.center()
            window = w
        }
        NSApp.activate()
        window?.makeKeyAndOrderFront(nil)
    }
}

@MainActor
private struct WelcomeView: View {
    let done: () -> Void
    @State private var granted = Permissions.accessibilityGranted
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        VStack(spacing: 20) {
            Image(nsImage: NSApp.applicationIconImage)
                .resizable()
                .frame(width: 96, height: 96)
                .accessibilityHidden(true)
            VStack(spacing: 6) {
                Text("Pane needs Accessibility access")
                    .font(.title2.weight(.semibold))
                Text("Pane moves and resizes windows through the macOS Accessibility API. It never reads what's in your windows.")
                    .multilineTextAlignment(.center)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            VStack(alignment: .leading, spacing: 10) {
                step(1, "Click Open System Settings.")
                step(2, "Turn on Pane under Privacy & Security › Accessibility.")
                step(3, "Come back here. Pane starts tiling right away.")
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(14)
            .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(.quaternary.opacity(0.5)))

            status
            HStack {
                if granted {
                    Spacer()
                    Button("Done", action: done)
                        .buttonStyle(.glassProminent)
                        .controlSize(.large)
                        .keyboardShortcut(.defaultAction)
                } else {
                    Button("Later", action: done)
                        .controlSize(.large)
                        .keyboardShortcut(.cancelAction)
                    Spacer()
                    Button("Open System Settings") { Permissions.openAccessibilitySettings() }
                        .buttonStyle(.glassProminent)
                        .controlSize(.large)
                        .keyboardShortcut(.defaultAction)
                }
            }
        }
        .padding(.horizontal, 32)
        .padding(.top, 36)
        .padding(.bottom, 24)
        .frame(width: 460)
        .animation(reduceMotion ? nil : .default, value: granted)
        .task {
            while !Task.isCancelled {
                granted = Permissions.accessibilityGranted
                try? await Task.sleep(for: .seconds(1))
            }
        }
    }

    private func step(_ n: Int, _ text: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Text("\(n)")
                .font(.callout.weight(.semibold))
                .foregroundStyle(.white)
                .frame(width: 22, height: 22)
                .background(Circle().fill(Color.accentColor))
            Text(text).fixedSize(horizontal: false, vertical: true)
        }
    }

    @ViewBuilder
    private var status: some View {
        if granted {
            Label("You're all set", systemImage: "checkmark.circle.fill")
                .font(.headline)
                .foregroundStyle(.green)
                .transition(.opacity)
        } else {
            HStack(spacing: 8) {
                ProgressView().controlSize(.small)
                Text("Waiting for access…").foregroundStyle(.secondary)
            }
            .transition(.opacity)
        }
    }
}
