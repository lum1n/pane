import AppKit

/// One row of the palette (`palette::PaletteItem` in pane-core). `command`
/// is sent back to the core unchanged when the row is picked.
struct PaletteItem {
    var kind: String
    var title: String
    var subtitle: String
    var bundleId: String?
    var shortcut: String?
    var current: Bool
    var command: [String: Any]

    init?(_ any: Any) {
        guard let d = any as? [String: Any],
              let kind = d["kind"] as? String,
              let title = d["title"] as? String,
              let command = d["command"] as? [String: Any] else { return nil }
        self.kind = kind
        self.title = title
        subtitle = d["subtitle"] as? String ?? ""
        bundleId = d["bundle_id"] as? String
        shortcut = d["shortcut"] as? String
        current = d["current"] as? Bool ?? false
        self.command = command
    }
}

/// The window and command palette (`palette`, mod-/ by default): type to
/// find any window on any display or workspace, a workspace, a profile or
/// a command; Enter jumps to it or runs it. `>` searches only commands.
///
/// The panel does not activate Pane, so Escape (or clicking elsewhere)
/// leaves focus where it was. Ranking happens in the core (`search`).
@MainActor
final class CommandPalette: NSObject, NSTextFieldDelegate, NSTableViewDataSource, NSTableViewDelegate, NSWindowDelegate {
    static let shared = CommandPalette()

    private var panel: KeyPanel?
    private let field = NSTextField()
    private let table = NSTableView()
    private let scroll = NSScrollView()
    private var items: [PaletteItem] = []
    private var icons: [String: NSImage] = [:]
    /// No results for a non-empty query: one "No matches" row.
    private var showsNoMatches: Bool { items.isEmpty && !field.stringValue.isEmpty }
    private var rowCount: Int { showsNoMatches ? 1 : items.count }

    private static let width: CGFloat = 640
    private static let rowHeight: CGFloat = 44
    private static let maxRows = 9
    private static let fieldHeight: CGFloat = 52

    var isShown: Bool { panel?.isVisible ?? false }

    func toggle() {
        if isShown { hide() } else { show() }
    }

    func show() {
        let p = panel ?? makePanel()
        panel = p
        field.stringValue = ""
        refresh()
        p.setFrame(frame(rows: rowCount), display: true)
        p.makeKeyAndOrderFront(nil)
        p.makeFirstResponder(field)
    }

    func hide() {
        panel?.orderOut(nil)
    }

    // MARK: Search

    private func refresh() {
        switch PaneCore.shared.send(["cmd": "search", "query": field.stringValue]) {
        case let .ok(data):
            items = (data as? [Any] ?? []).compactMap(PaletteItem.init)
        case .error:
            items = []
        }
        table.reloadData()
        if !items.isEmpty {
            table.selectRowIndexes(IndexSet(integer: 0), byExtendingSelection: false)
            table.scrollRowToVisible(0)
        }
        if let p = panel, p.isVisible {
            // Keep the top edge still while the list grows and shrinks.
            let old = p.frame
            let new = frame(rows: rowCount)
            p.setFrame(NSRect(x: old.minX, y: old.maxY - new.height, width: new.width, height: new.height), display: true)
        }
    }

    private func pick(_ row: Int) {
        guard items.indices.contains(row) else { return }
        let cmd = items[row].command
        hide()
        if case let .error(m) = PaneCore.shared.send(cmd) { NoticeHUD.shared.show(m, symbol: NoticeHUD.errorSymbol) }
    }

    private func moveSelection(_ delta: Int) {
        guard !items.isEmpty else { return }
        let cur = table.selectedRow < 0 ? 0 : table.selectedRow
        let next = max(0, min(items.count - 1, cur + delta))
        table.selectRowIndexes(IndexSet(integer: next), byExtendingSelection: false)
        table.scrollRowToVisible(next)
    }

    // MARK: NSTextFieldDelegate

    func controlTextDidChange(_ obj: Notification) {
        refresh()
    }

    func control(_ control: NSControl, textView: NSTextView, doCommandBy sel: Selector) -> Bool {
        switch sel {
        case #selector(NSResponder.moveUp(_:)): moveSelection(-1)
        case #selector(NSResponder.moveDown(_:)): moveSelection(1)
        case #selector(NSResponder.scrollPageUp(_:)), #selector(NSResponder.pageUp(_:)): moveSelection(-Self.maxRows)
        case #selector(NSResponder.scrollPageDown(_:)), #selector(NSResponder.pageDown(_:)): moveSelection(Self.maxRows)
        case #selector(NSResponder.insertNewline(_:)): pick(table.selectedRow)
        case #selector(NSResponder.cancelOperation(_:)): hide()
        default: return false
        }
        return true
    }

    // MARK: NSWindowDelegate

    func windowDidResignKey(_ notification: Notification) {
        hide()
    }

    // MARK: Table

    func numberOfRows(in tableView: NSTableView) -> Int { rowCount }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let cell = (tableView.makeView(withIdentifier: PaletteRow.id, owner: nil) as? PaletteRow) ?? PaletteRow()
        if showsNoMatches {
            cell.configureNoMatches()
        } else {
            let item = items[row]
            cell.configure(item, icon: icon(for: item))
        }
        return cell
    }

    func tableView(_ tableView: NSTableView, rowViewForRow row: Int) -> NSTableRowView? {
        PaletteRowView()
    }

    func tableView(_ tableView: NSTableView, shouldSelectRow row: Int) -> Bool {
        !showsNoMatches
    }

    @objc private func clicked() {
        pick(table.clickedRow)
    }

    private func icon(for item: PaletteItem) -> NSImage? {
        if let b = item.bundleId {
            if let i = icons[b] { return i }
            let image: NSImage? = NSWorkspace.shared.urlForApplication(withBundleIdentifier: b)
                .map { NSWorkspace.shared.icon(forFile: $0.path) }
                ?? NSRunningApplication.runningApplications(withBundleIdentifier: b).first?.icon
            if let image { icons[b] = image }
            return image
        }
        let symbol = switch item.kind {
        case "workspace": "rectangle.split.3x1"
        case "profile": "person.crop.rectangle.stack"
        default: "command"
        }
        return PaletteRow.symbol(symbol, accessibilityDescription: item.kind)
    }

    // MARK: Layout

    /// Centred on the focused display, a fifth of the way down.
    private func frame(rows: Int) -> NSRect {
        let shown = CGFloat(max(1, min(rows, Self.maxRows)))
        let h = Self.fieldHeight + (rows == 0 ? 0 : shown * Self.rowHeight + 10)
        let s = PaneCore.shared.snapshot
        let display = s?.displays.first(where: { $0.focused }) ?? s?.displays.first(where: { $0.is_main })
        let screen = display.map { GestureOverlay.cocoa($0.visible_frame.cg) }
            ?? NSScreen.main?.visibleFrame ?? NSRect(x: 0, y: 0, width: 1440, height: 900)
        let top = screen.maxY - screen.height / 5
        return NSRect(x: screen.midX - Self.width / 2, y: top - h, width: Self.width, height: h)
    }

    private func makePanel() -> KeyPanel {
        let p = KeyPanel(contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: true)
        p.isOpaque = false
        p.backgroundColor = .clear
        p.hasShadow = true
        // Above the focus border and dimming (floating, floating + 1).
        p.level = NSWindow.Level(rawValue: NSWindow.Level.floating.rawValue + 2)
        p.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .ignoresCycle]
        p.hidesOnDeactivate = false
        p.becomesKeyOnlyIfNeeded = false
        p.isReleasedWhenClosed = false
        p.delegate = self

        // Spotlight-style Liquid Glass; `container` holds the field, rule and list.
        let glass = NSGlassEffectView()
        glass.cornerRadius = 24
        glass.autoresizingMask = [.width, .height]
        let container = NSView()

        let magnifier = NSImageView()
        magnifier.image = PaletteRow.symbol("magnifyingglass", accessibilityDescription: nil, pointSize: 18)
        magnifier.contentTintColor = .secondaryLabelColor
        magnifier.translatesAutoresizingMaskIntoConstraints = false
        magnifier.setContentHuggingPriority(.required, for: .horizontal)

        field.isBordered = false
        field.drawsBackground = false
        field.focusRingType = .none
        field.font = .systemFont(ofSize: 22)
        field.placeholderString = "Go to window, workspace or profile   (> for commands)"
        field.delegate = self
        field.cell?.usesSingleLineMode = true
        field.cell?.isScrollable = true
        field.translatesAutoresizingMaskIntoConstraints = false

        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("item"))
        column.resizingMask = .autoresizingMask
        table.addTableColumn(column)
        table.columnAutoresizingStyle = .uniformColumnAutoresizingStyle
        table.headerView = nil
        table.rowHeight = Self.rowHeight
        table.intercellSpacing = .zero
        table.backgroundColor = .clear
        table.style = .plain
        table.selectionHighlightStyle = .regular
        table.refusesFirstResponder = true
        table.dataSource = self
        table.delegate = self
        table.target = self
        table.action = #selector(clicked)

        scroll.documentView = table
        scroll.drawsBackground = false
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.borderType = .noBorder
        scroll.translatesAutoresizingMaskIntoConstraints = false

        let rule = NSBox()
        rule.boxType = .separator
        rule.translatesAutoresizingMaskIntoConstraints = false

        container.addSubview(magnifier)
        container.addSubview(field)
        container.addSubview(rule)
        container.addSubview(scroll)
        NSLayoutConstraint.activate([
            magnifier.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 20),
            magnifier.centerYAnchor.constraint(equalTo: field.centerYAnchor),
            field.leadingAnchor.constraint(equalTo: magnifier.trailingAnchor, constant: 10),
            field.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -20),
            field.topAnchor.constraint(equalTo: container.topAnchor, constant: 12),
            field.heightAnchor.constraint(equalToConstant: 28),
            rule.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 12),
            rule.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -12),
            rule.topAnchor.constraint(equalTo: container.topAnchor, constant: Self.fieldHeight - 1),
            scroll.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 8),
            scroll.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -8),
            scroll.topAnchor.constraint(equalTo: rule.bottomAnchor, constant: 4),
            scroll.bottomAnchor.constraint(equalTo: container.bottomAnchor, constant: -6),
        ])
        glass.contentView = container
        p.contentView = glass
        return p
    }
}

/// A borderless panel that can take typing without activating Pane.
final class KeyPanel: NSPanel {
    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }
}

/// Selection as an inset rounded rect, like Spotlight.
private final class PaletteRowView: NSTableRowView {
    override func drawSelection(in dirtyRect: NSRect) {
        guard selectionHighlightStyle != .none else { return }
        let color = isEmphasized
            ? NSColor.selectedContentBackgroundColor
            : NSColor.controlAccentColor.withAlphaComponent(0.25)
        color.setFill()
        NSBezierPath(roundedRect: bounds.insetBy(dx: 4, dy: 2), xRadius: 10, yRadius: 10).fill()
    }

    override var interiorBackgroundStyle: NSView.BackgroundStyle {
        isSelected && isEmphasized ? .emphasized : .normal
    }
}

private final class PaletteRow: NSTableCellView {
    static let id = NSUserInterfaceItemIdentifier("PaletteRow")
    private let icon = NSImageView()
    private let title = NSTextField(labelWithString: "")
    private let subtitle = NSTextField(labelWithString: "")
    private let currentTag = NSImageView()
    private let shortcut = NSImageView()
    /// Stands in for "no image", so the image views keep an intrinsic size.
    private static let blank = NSImage(size: NSSize(width: 1, height: 1))

    /// An SF Symbol in the palette's hierarchical style.
    static func symbol(_ name: String, accessibilityDescription: String?, pointSize: CGFloat = 20) -> NSImage? {
        let config = NSImage.SymbolConfiguration(pointSize: pointSize, weight: .regular)
            .applying(NSImage.SymbolConfiguration(hierarchicalColor: .secondaryLabelColor))
        return NSImage(systemSymbolName: name, accessibilityDescription: accessibilityDescription)?
            .withSymbolConfiguration(config)
    }

    init() {
        super.init(frame: .zero)
        identifier = Self.id
        title.font = .systemFont(ofSize: 14)
        title.lineBreakMode = .byTruncatingTail
        subtitle.font = .systemFont(ofSize: 11)
        subtitle.textColor = .secondaryLabelColor
        subtitle.lineBreakMode = .byTruncatingTail
        for v in [shortcut, currentTag] {
            v.imageScaling = .scaleNone
            v.setContentCompressionResistancePriority(.required, for: .horizontal)
            v.setContentHuggingPriority(.required, for: .horizontal)
        }
        icon.imageScaling = .scaleProportionallyUpOrDown
        for v in [icon, title, subtitle, currentTag, shortcut] as [NSView] {
            v.translatesAutoresizingMaskIntoConstraints = false
            addSubview(v)
        }
        NSLayoutConstraint.activate([
            icon.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            icon.centerYAnchor.constraint(equalTo: centerYAnchor),
            icon.widthAnchor.constraint(equalToConstant: 26),
            icon.heightAnchor.constraint(equalToConstant: 26),
            title.leadingAnchor.constraint(equalTo: icon.trailingAnchor, constant: 10),
            title.topAnchor.constraint(equalTo: topAnchor, constant: 5),
            title.trailingAnchor.constraint(lessThanOrEqualTo: shortcut.leadingAnchor, constant: -10),
            currentTag.leadingAnchor.constraint(equalTo: title.trailingAnchor, constant: 6),
            currentTag.centerYAnchor.constraint(equalTo: title.centerYAnchor),
            currentTag.trailingAnchor.constraint(lessThanOrEqualTo: shortcut.leadingAnchor, constant: -10),
            subtitle.leadingAnchor.constraint(equalTo: title.leadingAnchor),
            subtitle.topAnchor.constraint(equalTo: title.bottomAnchor, constant: 1),
            subtitle.trailingAnchor.constraint(lessThanOrEqualTo: shortcut.leadingAnchor, constant: -10),
            shortcut.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -14),
            shortcut.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }

    required init?(coder: NSCoder) { fatalError("not used") }

    func configure(_ item: PaletteItem, icon image: NSImage?) {
        icon.image = image
        icon.contentTintColor = nil
        title.stringValue = item.title
        title.textColor = .labelColor
        title.font = .systemFont(ofSize: 14, weight: item.current ? .medium : .regular)
        subtitle.stringValue = item.subtitle
        currentTag.image = item.current ? KeycapDrawing.tag("Current") : Self.blank
        currentTag.isHidden = !item.current
        shortcut.image = item.shortcut.flatMap(KeycapDrawing.image(for:)) ?? Self.blank
        setAccessibilityLabel([item.title, item.subtitle, item.current ? "current" : nil,
                               item.shortcut.map(Shortcut.pretty)].compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: ", "))
    }

    func configureNoMatches() {
        icon.image = Self.symbol("magnifyingglass", accessibilityDescription: nil)
        title.stringValue = "No matches"
        title.textColor = .secondaryLabelColor
        title.font = .systemFont(ofSize: 14)
        subtitle.stringValue = "Type > to search commands only"
        currentTag.image = Self.blank
        currentTag.isHidden = true
        shortcut.image = Self.blank
        setAccessibilityLabel("No matches")
    }
}
