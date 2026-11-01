import AppKit
import OndaCore
import SwiftUI

/// The current step's content (ONB-3..ONB-9).
struct OnboardingStepView: View {
    let content: OnboardingContent
    let hotkey: HotkeyState?
    let core: OndaCore

    var body: some View {
        switch content {
        case let .welcome(title, subtitle):
            OnboardingWelcomeStep(title: title, subtitle: subtitle)
        case let .language(title, searchPlaceholder, query, toggleTitle, cards):
            OnboardingLanguageStep(
                title: title,
                searchPlaceholder: searchPlaceholder,
                initialQuery: query,
                toggleTitle: toggleTitle,
                cards: cards,
                core: core
            )
        case let .permissions(title, microphone, typingAccess, clipboardLink):
            OnboardingPermissionsStep(
                title: title,
                microphone: microphone,
                typingAccess: typingAccess,
                clipboardLink: clipboardLink,
                core: core
            )
        case let .hotkey(title, subtitle, presets, customSelected, customKeycaps, customLabel, settingsTitle):
            OnboardingHotkeyStep(
                title: title,
                subtitle: subtitle,
                presets: presets,
                customSelected: customSelected,
                customKeycaps: customKeycaps,
                customLabel: customLabel,
                openKeyboardSettingsTitle: settingsTitle,
                hotkey: hotkey,
                core: core
            )
        case let .speechModel(title, cards, otherModels, error):
            OnboardingModelStep(title: title, cards: cards, otherModels: otherModels, error: error, core: core)
        case let .ready(title, prefix, keycaps, suffix, placeholder, retentionNote):
            OnboardingReadyStep(
                title: title,
                promptPrefix: prefix,
                keycaps: keycaps,
                promptSuffix: suffix,
                placeholder: placeholder,
                retentionNote: retentionNote
            )
        }
    }
}

// MARK: - Welcome

/// ONB-3: the app icon with a soft accent glow.
private struct OnboardingWelcomeStep: View {
    let title: String
    let subtitle: String

    var body: some View {
        VStack(spacing: 0) {
            Spacer()
            ZStack {
                Circle()
                    .fill(Color.accentColor.opacity(0.25))
                    .frame(width: 150, height: 150)
                    .blur(radius: 45)
                Image(nsImage: NSApp.applicationIconImage)
                    .resizable()
                    .frame(width: 128, height: 128)
                    .shadow(color: .black.opacity(0.25), radius: 12, y: 6)
                    .accessibilityHidden(true)
            }
            .padding(.bottom, 28)
            Text(title)
                .font(.system(size: 34, weight: .bold))
                .accessibilityAddTraits(.isHeader)
            Text(subtitle)
                .font(.title3)
                .foregroundStyle(.secondary)
                .padding(.top, 6)
            Spacer()
            Spacer()
        }
    }
}

// MARK: - Language

/// ONB-4: search, the Show All / Popular Only toggle, and a 3-column grid.
private struct OnboardingLanguageStep: View {
    let title: String
    let searchPlaceholder: String
    let initialQuery: String
    let toggleTitle: String?
    let cards: [LanguageCardView]
    let core: OndaCore

    /// the field owns its text; the core's echo never overwrites typing
    @State private var query: String?

    var body: some View {
        VStack(spacing: 16) {
            OnboardingStepHeader(title: title)
            HStack(spacing: 12) {
                HStack(spacing: 6) {
                    Image(systemName: "magnifyingglass")
                        .foregroundStyle(.secondary)
                        .accessibilityHidden(true)
                    TextField(searchPlaceholder, text: queryBinding)
                        .textFieldStyle(.plain)
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 7)
                .background(
                    RoundedRectangle(cornerRadius: OnboardingRadius.medium)
                        .fill(.quinary)
                )
                if let toggleTitle {
                    Button(toggleTitle) {
                        core.onboardingToggleAllLanguages()
                    }
                }
            }
            .frame(maxWidth: 460)
            ScrollView {
                LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 10), count: 3), spacing: 10) {
                    ForEach(cards, id: \.code) { card in
                        OnboardingLanguageCard(card: card) {
                            core.onboardingSelectLanguage(code: card.code)
                        }
                    }
                }
                .padding(.horizontal, 2)
                .padding(.bottom, 8)
            }
            .frame(maxWidth: 560)
        }
        .padding(.top, 8)
    }

    private var queryBinding: Binding<String> {
        Binding(
            get: { query ?? initialQuery },
            set: { text in
                query = text
                core.onboardingSearchLanguages(query: text)
            }
        )
    }
}

private struct OnboardingLanguageCard: View {
    let card: LanguageCardView
    let select: () -> Void

    var body: some View {
        Button(action: select) {
            HStack(spacing: 10) {
                Text(card.flag)
                    .font(.title3)
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 1) {
                    Text(card.name)
                        .fontWeight(card.selected ? .semibold : .regular)
                        .lineLimit(1)
                    Text(card.codeLabel)
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
                Spacer(minLength: 0)
                if card.selected {
                    Image(systemName: "checkmark.circle.fill")
                        .foregroundStyle(Color.accentColor)
                        .accessibilityHidden(true)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            .contentShape(Rectangle())
            .onboardingChoice(selected: card.selected, radius: OnboardingRadius.medium)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(card.name)
        .accessibilityAddTraits(card.selected ? .isSelected : [])
    }
}

// MARK: - Permissions

/// ONB-6: Microphone and Typing Access cards, and the clipboard link.
private struct OnboardingPermissionsStep: View {
    let title: String
    let microphone: PermissionCardView
    let typingAccess: PermissionCardView
    let clipboardLink: String?
    let core: OndaCore

    var body: some View {
        VStack(spacing: 16) {
            OnboardingStepHeader(title: title)
            VStack(spacing: 12) {
                OnboardingPermissionCard(card: microphone, symbol: "mic.fill", tint: .blue, core: core)
                OnboardingPermissionCard(card: typingAccess, symbol: "keyboard.fill", tint: .purple, core: core)
                if let clipboardLink {
                    Button(clipboardLink) {
                        core.onboardingUseClipboard()
                    }
                    .buttonStyle(.link)
                    .font(.callout)
                }
            }
            .frame(maxWidth: 520)
            .padding(.top, 8)
            Spacer()
        }
        .padding(.top, 8)
    }
}

private struct OnboardingPermissionCard: View {
    let card: PermissionCardView
    let symbol: String
    let tint: Color
    let core: OndaCore

    var body: some View {
        HStack(spacing: 14) {
            Image(systemName: symbol)
                .font(.title3)
                .foregroundStyle(.white)
                .frame(width: 38, height: 38)
                .background(
                    RoundedRectangle(cornerRadius: OnboardingRadius.medium)
                        .fill(tint.gradient)
                )
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                Text(card.title)
                    .font(.headline)
                if let detail = card.detail {
                    Text(detail)
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer()
            if card.granted {
                Label(card.readyLabel, systemImage: "checkmark.circle.fill")
                    .font(.callout.weight(.medium))
                    .foregroundStyle(.green)
                    .transition(.scale.combined(with: .opacity))
            }
            if let button = card.button {
                fixButton(button)
            }
        }
        .padding(16)
        .onboardingCard()
        .animation(.spring(duration: 0.3), value: card.granted)
    }

    @ViewBuilder
    private func fixButton(_ button: PermissionFixButton) -> some View {
        let fix = Button(button.title) {
            core.fixPermission(fix: button.fix)
        }
        switch button.fix {
        case .requestMicrophone:
            fix.buttonStyle(.borderedProminent)
        case .openMicrophoneSettings, .openAccessibilitySettings:
            fix.buttonStyle(.bordered)
        }
    }
}

// MARK: - Hotkey

/// ONB-7: preset cards and the "Another shortcut" card with the recorder.
private struct OnboardingHotkeyStep: View {
    let title: String
    let subtitle: String
    let presets: [PresetCardView]
    let customSelected: Bool
    let customKeycaps: [String]?
    let customLabel: String?
    let openKeyboardSettingsTitle: String
    let hotkey: HotkeyState?
    let core: OndaCore

    var body: some View {
        VStack(spacing: 16) {
            OnboardingStepHeader(title: title, subtitle: subtitle)
            ScrollView {
                VStack(spacing: 8) {
                    ForEach(presets, id: \.preset) { preset in
                        presetCard(preset)
                    }
                    customCard
                }
                .frame(maxWidth: 580)
                .padding(.bottom, 8)
            }
        }
        .padding(.top, 8)
    }

    private func presetCard(_ card: PresetCardView) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Button {
                core.onboardingSelectHotkey(preset: card.preset)
            } label: {
                OnboardingHotkeyChoice(selected: card.selected) {
                    OnboardingKeycaps(labels: card.keycaps)
                    Text(card.detail)
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(!card.enabled)
            .opacity(card.enabled ? 1 : 0.5)
            .accessibilityLabel(card.keycaps.joined(separator: " "))
            .accessibilityHint(card.detail)
            .accessibilityAddTraits(card.selected ? .isSelected : [])
            // outside the choice button so its own button stays clickable
            if let warning = card.warning {
                OnboardingFnWarning(message: warning, buttonTitle: openKeyboardSettingsTitle) {
                    core.openKeyboardSettings()
                }
                .padding(.leading, 34)
            }
        }
        .padding(12)
        .onboardingChoice(selected: card.selected)
    }

    private var customCard: some View {
        OnboardingHotkeyChoice(selected: customSelected) {
            if let customKeycaps {
                OnboardingKeycaps(labels: customKeycaps)
            } else if let customLabel {
                Text(customLabel)
                    .font(.body.weight(.medium))
            }
            if let hotkey {
                ShortcutRecorderControls(hotkey: hotkey, core: core)
            }
        }
        .padding(12)
        .onboardingChoice(selected: customSelected)
    }
}

/// A radio mark beside a choice's content.
private struct OnboardingHotkeyChoice<Content: View>: View {
    let selected: Bool
    @ViewBuilder let content: () -> Content

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: selected ? "checkmark.circle.fill" : "circle")
                .font(.title3)
                .foregroundStyle(selected ? Color.accentColor : Color.secondary)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 4) {
                content()
            }
            Spacer(minLength: 0)
        }
    }
}

// MARK: - Speech model

/// ONB-8: full cards side by side, other models as compact rows, and the
/// failure line.
private struct OnboardingModelStep: View {
    let title: String
    let cards: [ModelCardView]
    let otherModels: OtherModelsView?
    let error: String?
    let core: OndaCore

    var body: some View {
        VStack(spacing: 16) {
            OnboardingStepHeader(title: title)
            ScrollView {
                VStack(spacing: 12) {
                    HStack(alignment: .top, spacing: 12) {
                        ForEach(cards, id: \.id) { card in
                            OnboardingModelCard(card: card, core: core)
                        }
                    }
                    if let otherModels {
                        otherModelsSection(otherModels)
                    }
                    if let error {
                        Label(error, systemImage: "exclamationmark.triangle.fill")
                            .font(.callout)
                            .foregroundStyle(.red)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                .frame(maxWidth: 560)
                .padding(.bottom, 8)
            }
        }
        .padding(.top, 8)
    }

    private func otherModelsSection(_ other: OtherModelsView) -> some View {
        let expanded = !other.rows.isEmpty
        return VStack(spacing: 8) {
            Button {
                core.onboardingToggleOtherModels()
            } label: {
                HStack(spacing: 4) {
                    Text(other.toggleTitle)
                    Image(systemName: "chevron.down")
                        .font(.caption2)
                        .rotationEffect(.degrees(expanded ? 180 : 0))
                        .accessibilityHidden(true)
                }
                .font(.callout)
                .foregroundStyle(.secondary)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(other.toggleTitle)
            ForEach(other.rows, id: \.id) { row in
                OnboardingModelRow(card: row, core: core)
            }
        }
    }
}

private struct OnboardingModelCard: View {
    let card: ModelCardView
    let core: OndaCore

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 6) {
                Text(card.name)
                    .font(.headline)
                    .lineLimit(1)
                Spacer()
                if let recommended = card.recommended {
                    Text(recommended)
                        .font(.caption2.weight(.medium))
                        .padding(.horizontal, 7)
                        .padding(.vertical, 2)
                        .background(Color.accentColor.opacity(0.15))
                        .foregroundStyle(Color.accentColor)
                        .clipShape(Capsule())
                }
            }
            Text(card.description)
                .font(.callout)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, minHeight: 34, alignment: .topLeading)
            Label(card.sizeHint, systemImage: "internaldrive")
                .font(.caption)
                .foregroundStyle(.secondary)
            OnboardingModelAction(id: card.id, action: card.action, core: core)
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .onboardingCard()
        .overlay(
            RoundedRectangle(cornerRadius: OnboardingRadius.large)
                .strokeBorder(card.selected ? Color.accentColor : Color.clear, lineWidth: 1.5)
        )
    }
}

private struct OnboardingModelRow: View {
    let card: ModelCardView
    let core: OndaCore

    var body: some View {
        HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                Text(card.name)
                    .font(.callout.weight(.medium))
                HStack(spacing: 4) {
                    Text(card.description)
                        .lineLimit(1)
                    Text("· \(card.sizeHint)")
                        .fixedSize()
                }
                .font(.caption)
                .foregroundStyle(.secondary)
            }
            Spacer()
            OnboardingModelAction(id: card.id, action: card.action, core: core)
                .frame(minWidth: 110, maxWidth: 200, alignment: .trailing)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .onboardingCard()
        .overlay(
            RoundedRectangle(cornerRadius: OnboardingRadius.large)
                .strokeBorder(card.selected ? Color.accentColor : Color.clear, lineWidth: 1.5)
        )
    }
}

/// A model's action: progress and Cancel, Selected, Use This Model, or
/// Download.
private struct OnboardingModelAction: View {
    let id: String
    let action: ModelCardAction
    let core: OndaCore

    var body: some View {
        switch action {
        case let .downloading(progress, label, cancelTitle):
            VStack(alignment: .leading, spacing: 4) {
                ProgressView(value: progress)
                HStack(spacing: 8) {
                    Text(label)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .monospacedDigit()
                    Spacer(minLength: 8)
                    Button(cancelTitle) {
                        core.cancelDownload(id: id)
                    }
                    .buttonStyle(.borderless)
                }
            }
        case let .selected(title):
            Label(title, systemImage: "checkmark.circle.fill")
                .font(.callout.weight(.medium))
                .foregroundStyle(.green)
        case let .useThisModel(title):
            Button(title) {
                core.onboardingChooseModel(id: id)
            }
            .buttonStyle(.bordered)
        case let .download(title):
            Button(title) {
                core.onboardingDownloadModel(id: id)
            }
            .buttonStyle(.borderedProminent)
        }
    }
}

// MARK: - Ready

/// ONB-9: the green seal, the hotkey sentence, the try-it box, and the
/// retention sentence.
private struct OnboardingReadyStep: View {
    let title: String
    let promptPrefix: String
    let keycaps: [String]
    let promptSuffix: String
    let placeholder: String
    let retentionNote: String?

    /// a plain local text view; never logged or sent anywhere
    @State private var tryIt = ""

    var body: some View {
        VStack(spacing: 0) {
            Spacer()
            Image(systemName: "checkmark.seal.fill")
                .font(.system(size: 52))
                .foregroundStyle(.green.gradient)
                .symbolEffect(.bounce, options: .nonRepeating)
                .padding(.bottom, 14)
                .accessibilityHidden(true)
            Text(title)
                .font(.system(size: 30, weight: .bold))
                .accessibilityAddTraits(.isHeader)
            HStack(spacing: 8) {
                Text(promptPrefix)
                    .foregroundStyle(.secondary)
                OnboardingKeycaps(labels: keycaps, font: .title3.weight(.medium))
                Text(promptSuffix)
                    .foregroundStyle(.secondary)
            }
            .font(.title3)
            .padding(.top, 10)
            .accessibilityElement(children: .combine)
            tryItBox
                .padding(.top, 28)
            if let retentionNote {
                Text(retentionNote)
                    .font(.callout)
                    .foregroundStyle(.tertiary)
                    .padding(.top, 16)
            }
            Spacer()
        }
    }

    private var tryItBox: some View {
        TextEditor(text: $tryIt)
            .font(.body)
            .scrollContentBackground(.hidden)
            .scrollIndicators(.never)
            .padding(10)
            .frame(maxWidth: 440)
            .frame(height: 76)
            .background(
                RoundedRectangle(cornerRadius: OnboardingRadius.large)
                    .fill(.quinary)
            )
            .overlay(
                RoundedRectangle(cornerRadius: OnboardingRadius.large)
                    .strokeBorder(.quaternary, lineWidth: 1)
            )
            .overlay(alignment: .topLeading) {
                if tryIt.isEmpty {
                    Text(placeholder)
                        .foregroundStyle(.tertiary)
                        .padding(.horizontal, 15)
                        .padding(.vertical, 10)
                        .allowsHitTesting(false)
                        .accessibilityHidden(true)
                }
            }
            .accessibilityLabel(placeholder)
    }
}
