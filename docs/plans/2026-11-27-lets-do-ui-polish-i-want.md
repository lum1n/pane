# Pane UI polish for macOS 26 (Liquid Glass)

## Context
The Swift shell (`shell/Sources/PaneShell`, ~3.9k lines) works, but it looks like a developer tool. Settings is one 620×640 window: a SwiftUI `TabView` with `GroupBox`es and loose caption text. The profile editor has a hand-built toolbar row and an `HSplitView`. The palette and HUD use `NSVisualEffectView(.hudWindow)`. Menu items have no icons, and the menu headers are disabled items. The user wants the app to look like something Apple could ship, built with current SwiftUI/AppKit with **macOS 26 as the minimum deployment target**. The minimum lets the code use the macOS 26 APIs (Liquid Glass, `NSGlassEffectView`, `ToolbarSpacer`, `.glassProminent`) directly, without `#available` branches. Pane keeps running on later macOS releases.

Constraints:
- Pane's "no animations" rule is about tiling, so window placement and the gesture overlay stay instant. UI chrome may use the system's own transitions, and it honours Reduce Motion.
- The status item's custom click handling (clicking a tile focuses that window) stays, so the app keeps its AppKit `AppDelegate` and `NSStatusItem`, and there is no `MenuBarExtra`.
- This Linux box can't compile against AppKit, so the user verifies on the Mac with `make app`.

## 0. Set macOS 26 as the minimum
- `shell/Package.swift`: `swift-tools-version:6.2`, `platforms: [.macOS(.v26)]`, and `swiftLanguageModes: [.v5]` on the target. This avoids strict-concurrency churn, since the code already uses `@MainActor` and `assumeIsolated`.
- `shell/Resources/Info.plist`: set `LSMinimumSystemVersion` to `26.0`.
- `README.md`: change "macOS 14+" to "macOS 26+".
- Move the models to the Observation framework: `SettingsModel`, `EditorModel` and `UpdateChecker` become `@Observable`. Views switch to `@State`/`@Bindable`/plain `let` in place of `@StateObject`/`@ObservedObject`, and `@Published` goes away. The model logic itself does not change.

## 1. Settings: an Apple-style preferences window
Replace the `NSWindow(contentViewController: NSHostingController(SettingsView()))` in `AppDelegate.showSettings` with an **`NSTabViewController`, `tabStyle = .toolbar`**. Each pane is an `NSHostingController` with `sizingOptions = .preferredContentSize`. This gives the Finder/Safari-style toolbar tabs with SF Symbols, the macOS 26 glass toolbar, window-title-follows-tab and per-pane height for free. New file `Settings/SettingsWindowController.swift`. One shared `SettingsModel` is passed to every pane.

Panes, each a `Form { … }.formStyle(.grouped)` with a fixed width of about 560. Help text moves into `Section { } footer: { }` instead of free-floating captions.
- **General** (`gearshape`)
  - Status header row: app icon, "Pane 0.1.0", and an Accessibility status capsule (green checkmark or orange warning plus an "Open Settings" button).
  - Startup and updates: the Launch at Login toggle, the needs-approval row, "Check Now" with an inline `ProgressView` while checking, and the release link.
  - Menu bar: the app icons toggle.
  - Behaviour: restore placements, memory days (`Stepper` with a value label) and the default profile picker.
  - Configuration file: path as a monospaced selectable `LabeledContent`, buttons in an `HStack` with `.buttonStyle(.bordered)`, and the config error as a red `Label`.
  - Recovery: the existing footer text.
- **Layout** (`rectangle.split.2x1`): gaps as `LabeledContent` with a `TextField` plus `Stepper` showing "8 pt", smart gaps, new containers as a segmented `Picker`, resize step and auto-float. Electron IDs move here under "Advanced".
- **Focus** (`scope`): border width (stepper, 0 = off), the accent colour toggle with a `ColorPicker`, dim as a `Slider` with a live % label, and mouse follows focus / focus follows mouse.
- **Apps** (`square.on.square`): two sections, "Always float" and "Always tile". Each is a `List` of rows (app icon 20 pt, name, bundle id as secondary text) with the standard macOS **+/− bottom bar** (the Login Items pattern). "+" is a `Menu`: running apps (with icons), "Choose Application…" and "Enter Bundle ID or Glob…", which opens an alert-style `.sheet` with a text field. Selection plus "−" removes. An empty list shows a small `ContentUnavailableView`.
- **Scratchpads** (`rectangle.on.rectangle`): the same list-plus-bar pattern. Rows show icon, name, key as a keycap and size. Double-click or "Edit…" opens a `.sheet` with a `Form` for app (picker over running apps plus a text field), name, key and size, with Cancel/Save. That sheet replaces the inline dirty/Revert/Save state. `applyScratchpads()` still writes on Save.
- **Shortcuts** (`keyboard`)
  - Modifier section: four checkbox toggles in a row, plus the gestures toggle with a footer.
  - A `Table(model.bindings)` with "Command" and "Shortcut" columns and a `.searchable` filter.
  - The shortcut cell is a new reusable `KeycapView` that renders `Shortcut.pretty` as rounded keycap glyphs. While recording it shows "Type shortcut…" with an accent stroke.
  - Clear and Reset go into a row `contextMenu` and a hover-revealed trailing button. "Reset All…" asks for confirmation with `.confirmationDialog`.
- Transient `model.message` text moves into a single bottom status capsule per pane: `.safeAreaInset(edge: .bottom)` with `.glassEffect(.regular, in: .capsule)`. It auto-clears after about 4 s.

## 2. Profile editor: a NavigationSplitView with a native toolbar
`Editor/ProfileEditorWindow.swift`:
- Window: style mask adds `.fullSizeContentView`. Set `host.sceneBridgingOptions = [.toolbars, .title]` so SwiftUI `.toolbar` and `.navigationTitle` drive the NSWindow, and use `w.toolbarStyle = .unified`. Add `setFrameAutosaveName("PaneProfiles")`.
- `EditorRootView` becomes a `NavigationSplitView`:
  - **Sidebar**: `AppPaletteView`, with its `TextField` replaced by `.searchable(placement: .sidebar)`. The list gets "Running" and "Applications" `Section`s, and the hint moves into a footer.
  - **Detail**: `DeskCanvasView`, or `ProfileDiffView` while comparing.
  - **`.inspector(isPresented:)`**: `InspectorView`, toggled with a toolbar button (`sidebar.right`).
- Toolbar:
  - Navigation area: an Open `Menu` (`folder`) listing profiles, New (`plus`), Capture (`camera.viewfinder`), and Compare (`arrow.left.arrow.right`).
  - Then undo/redo, followed by a `ToolbarSpacer(.flexible)`.
  - Primary action group: Save (⌘S), with "Save as Startup" in its menu (`Menu` with `primaryAction:`), then **Apply** with `.buttonStyle(.glassProminent)`.
  - The profile name becomes the editable window title, `.navigationTitle($name)` (rename on commit), which replaces the 200 pt text field.
- The status line becomes a floating glass capsule at the bottom of the canvas. It auto-dismisses and uses `.transition(.opacity)`.
- `DeskCanvasView`:
  - Background: the `.windowBackground` material in place of `underPageBackgroundColor`.
  - Display boxes: `RoundedRectangle(cornerRadius: 14, style: .continuous)` with a subtle fill and shadow.
  - Workspace tabs: a compact segmented style built from capsules, with a `star.fill` badge on the visible workspace. The "Name" field moves into the inspector, or a popover from a `pencil` button.
  - "Add Display": a toolbar menu (`plus.rectangle.on.rectangle`) instead of the bottom bar.
  - Empty profile: `ContentUnavailableView("No Displays", systemImage: "display", description: …)` with New/Capture buttons.
- `TileTreeView`/`SlotTile`:
  - Continuous corners, `Color.accentColor.opacity` fills tuned for light and dark, and a selection ring drawn with `strokeBorder` so it doesn't clip.
  - The drop target highlights the actual zone (edge band versus centre) through a small overlay computed from `dropZone`.
  - `EmptyDrop` uses `Label("Drop an app", systemImage: "plus.app")`.
  - The `AnyView` switch becomes `@ViewBuilder`.
- `InspectorView`: an app header (large icon plus name and bundle id), then sections for "Matching", "Workspace" (the moved name field and the visible toggle) and "Layout" (a segmented split-direction picker in place of a "Split: horizontal" label, and an accordion toggle). "Remove Tile" goes in a final destructive section. The empty state is a `ContentUnavailableView`.
- `ProfileDiffView`: a header with title and summary, buttons in the window toolbar while diffing (Cancel / Edit Before Saving / Update Profile with `.glassProminent`), badges as tinted capsules, and a `ContentUnavailableView` when nothing differs.

## 3. Palette, HUD, overlay: AppKit Liquid Glass
- `Palette/CommandPalette.swift`: replace `NSVisualEffectView(.hudWindow)` with **`NSGlassEffectView`** (`cornerRadius = 24`, `contentView` = the container of field, rule and scroll), Spotlight-style. Keep `KeyPanel`, the key handling and the sizing logic as they are, since they are already the risky, verified-by-hand part.
  - Custom `NSTableRowView` subclass draws the selection as an inset rounded rect (radius 10, `controlAccentColor` at about 0.25 opacity, or `selectedContentBackgroundColor` when emphasized).
  - Field: 22 pt regular, with a `magnifyingglass` symbol on the left.
  - Row shortcut uses keycap drawing, an AppKit twin of `KeycapView` (small rounded rects).
  - SF Symbols use `.hierarchical` symbol configuration. The current item gets a "Current" secondary tag instead of only semibold.
  - When there are no results, show a single "No matches" row.
- `StatusItem/NoticeHUD.swift`: an `NSGlassEffectView` capsule, an optional leading SF Symbol (error vs info, inferred from the call site with a new `show(_:symbol:)` that defaults to `info.circle`), a 0.15 s fade in and out via `NSAnimationContext` (skipped when `NSWorkspace.shared.accessibilityDisplayShouldReduceMotion`), and a better auto-width.
- `Overlay/GestureOverlay.swift`: no animation. Raise corner radii to 12 to match macOS 26 window corners. Draw the label pill as a small hosted `NSGlassEffectView` subview instead of a black 65% fill. `FocusBorder` corner radius changes to the same constant (follow-up says 10 pt was a guess); share it as `UIConstants.windowCornerRadius`.

## 4. Status menu (macOS 26 menus use icons widely)
`StatusItem/StatusMenu.swift`:
- Display headers use `NSMenuItem.sectionHeader(title:)` in place of disabled items, with the focused display marked by `display` versus `display.badge.checkmark`-style text.
- Workspace window counts become `item.badge = NSMenuItemBadge(count:)` instead of "(3)".
- SF Symbol images on every action item: Go to Window `magnifyingglass`, Restore `arrow.uturn.backward.circle`, Reload `arrow.clockwise`, Profile Editor `square.grid.2x2`, Settings `gearshape`, Updates `arrow.down.circle`, Quit `power`, Scratchpads `rectangle.on.rectangle`, Summon `arrow.down.right.and.arrow.up.left`, Profiles `person.crop.rectangle.stack`, and the warning items `exclamationmark.triangle`.
- Add "About Pane" (`NSApp.orderFrontStandardAboutPanel`) above Settings.
- The save-profile `NSAlert` keeps its shape but gets the app icon and a proper default-button/empty-name check.

## 5. First-run Accessibility onboarding
Add `Onboarding/WelcomeWindow.swift`, a small SwiftUI window replacing `Permissions.explain()`'s NSAlert. It shows the app icon, "Pane needs Accessibility access", three short steps, an "Open System Settings" button (`.glassProminent`) and a live status row that polls `AXIsProcessTrusted()` every second and flips to a green "You're all set" with a Done button. `AppDelegate.handle("ax_permission_missing")` and `StatusMenu.grantAccess` open it.

## Shared pieces (new `Shell/UI.swift`)
`KeycapView` (SwiftUI), `KeycapDrawing` (AppKit), `UIConstants`, the `StatusCapsule` view used by Settings and the editor, and an `AppIconView(bundleId:size:)` wrapping the existing `AppIcons.icon(for:)` (`Editor/AppPaletteView.swift`). Keep using `Shortcut.pretty`, `AppIcons` and `PaneCore` as they are.

## Out of scope (goes in the Lapp Follow-ups note)
- A layered Icon Composer `.icon` app icon. It needs `actool` in `scripts/build-app.sh`; the current `AppIcon.icns` keeps working.
- Localization.

## Verification
1. Linux: `swiftc -parse` on every changed file. This catches syntax only, because AppKit isn't available here.
2. Mac: `make app && make run` with Xcode 26 on PATH. Fix compile errors, then check:
   - **Settings**: toolbar tabs switch and the window resizes per pane. Every control round-trips to config.toml (change a gap, check the file, change it back). Shortcut recording still works, as does Esc to cancel. Apps +/− works. A scratchpad add/edit sheet saves.
   - **Profile editor**: the toolbar shows glass Apply. Title rename works. Sidebar search, drag app to tile, inspector toggle and edits, undo/redo ⌘Z/⇧⌘Z, Save ⌘S, Compare with Desk diff and Update.
   - **Palette** (`mod-/`): glass panel takes typing without activating Pane, arrows/Enter/Esc work, and the selection is visible in light and dark.
   - **HUD**: trigger a notice (Reload with a broken config). It shows glass with an icon and fades, and appears instantly with Reduce Motion on.
   - **Menu**: section headers, badges, icons and About.
   - **Onboarding**: `tccutil reset Accessibility com.vegard.pane`, launch, and the window flips to granted once access is turned on.
   - **Light/dark and Increase Contrast / Reduce Transparency**: everything stays legible.
3. Add a "UI polish (macOS 26)" checklist with the above to `docs/verification.md`.
