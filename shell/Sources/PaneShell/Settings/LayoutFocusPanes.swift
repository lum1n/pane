import AppKit
import SwiftUI

@MainActor
struct LayoutPane: View {
    let model: SettingsModel
    @State private var electron = ""

    var body: some View {
        Form {
            if let s = model.settings {
                Section("Gaps") {
                    NumberRow(title: "Between windows", value: model.number("inner_gap") { $0.inner_gap }, range: 0...64)
                    NumberRow(title: "At screen edges", value: model.number("outer_gap") { $0.outer_gap }, range: 0...64)
                    Toggle("No gaps when a workspace shows one tile", isOn: model.toggle("smart_gaps") { $0.smart_gaps })
                }
                Section("Tiling") {
                    Picker("New containers", selection: Binding(
                        get: { s.default_layout },
                        set: { model.set("default_layout", $0) }
                    )) {
                        Text("Tiles").tag("tiles")
                        Text("Accordion").tag("accordion")
                    }
                    .pickerStyle(.segmented)
                    NumberRow(title: "Resize step", value: Binding(
                        get: { ((model.settings?.resize_step ?? 0.05) * 100).rounded() },
                        set: { model.set("resize_step", $0 / 100) }
                    ), range: 1...50, unit: "%")
                    NumberRow(title: "Float windows smaller than",
                              value: model.number("auto_float_below") { $0.auto_float_below },
                              range: 0...2000, step: 10)
                }
                Section {
                    TextField("Electron bundle IDs", text: $electron, prompt: Text("com.example.app, …"))
                        .onSubmit {
                            let ids = electron.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
                            model.set("electron_bundle_ids", ids)
                        }
                } header: {
                    Text("Advanced")
                } footer: {
                    Text("Comma separated. These apps get an extra resize pass so they land exactly. Press Return to save.")
                        .font(.footnote).foregroundStyle(.secondary)
                }
            }
        }
        .settingsPane(model, height: 470)
        .onAppear { electron = model.settings?.electron_bundle_ids.joined(separator: ", ") ?? "" }
        .onChange(of: model.settings?.electron_bundle_ids) { _, ids in
            electron = ids?.joined(separator: ", ") ?? ""
        }
    }
}

@MainActor
struct FocusPane: View {
    let model: SettingsModel
    @State private var dim = 0.0

    var body: some View {
        Form {
            if let s = model.settings {
                Section {
                    NumberRow(title: "Border width", value: model.number("border_width") { $0.border_width }, range: 0...16)
                    Toggle("Use the system accent colour", isOn: Binding(
                        get: { BorderColor.isAccent(model.settings?.border_color ?? "accent") },
                        set: { model.set("border_color", $0 ? "accent" : "#ff8000") }
                    ))
                    if !BorderColor.isAccent(s.border_color) {
                        ColorPicker("Border colour", selection: Binding(
                            get: { BorderColor.color(hex: s.border_color) },
                            set: { model.set("border_color", BorderColor.hex($0)) }
                        ))
                    }
                } header: {
                    Text("Focused Window")
                } footer: {
                    Text("A border width of 0 turns the border off.").font(.footnote).foregroundStyle(.secondary)
                }
                Section("Other Windows") {
                    LabeledContent("Dim") {
                        HStack {
                            Slider(value: $dim, in: 0...0.8) { editing in
                                if !editing { model.set("dim_unfocused", (dim * 100).rounded() / 100) }
                            }
                            .frame(maxWidth: 220)
                            Text("\(Int((dim * 100).rounded())) %")
                                .monospacedDigit()
                                .foregroundStyle(.secondary)
                                .frame(width: 44, alignment: .trailing)
                        }
                    }
                }
                Section("Pointer") {
                    Toggle("Move the pointer to the window you focus with the keyboard",
                           isOn: model.toggle("mouse_follows_focus") { $0.mouse_follows_focus })
                    Toggle("Focus the window under the pointer",
                           isOn: model.toggle("focus_follows_mouse") { $0.focus_follows_mouse })
                }
            }
        }
        .settingsPane(model, height: 430)
        .onAppear { dim = model.settings?.dim_unfocused ?? 0 }
        .onChange(of: model.settings?.dim_unfocused) { _, v in dim = v ?? 0 }
    }
}
