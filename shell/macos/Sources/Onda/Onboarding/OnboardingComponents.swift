import OndaCore
import SwiftUI

/// Corner radii shared by the onboarding cards.
enum OnboardingRadius {
    /// Keycaps.
    static let small: CGFloat = 6
    /// Search field, language cards, permission tiles.
    static let medium: CGFloat = 10
    /// Top-level cards.
    static let large: CGFloat = 12
}

/// One keycap per label, as the core spells them.
struct OnboardingKeycaps: View {
    let labels: [String]
    var font: Font = .body.weight(.medium)

    var body: some View {
        HStack(spacing: 4) {
            ForEach(Array(labels.enumerated()), id: \.offset) { _, label in
                Text(label)
                    .font(font)
                    .padding(.horizontal, label.count > 1 ? 8 : 7)
                    .padding(.vertical, 3)
                    .background(
                        RoundedRectangle(cornerRadius: OnboardingRadius.small)
                            .fill(.quaternary)
                            .shadow(color: .black.opacity(0.2), radius: 0, y: 1)
                    )
            }
        }
        .accessibilityElement(children: .combine)
    }
}

/// The plain card background: a faint fill with a hairline border.
struct OnboardingCardBackground: ViewModifier {
    func body(content: Content) -> some View {
        content
            .background(
                RoundedRectangle(cornerRadius: OnboardingRadius.large)
                    .fill(.quinary)
            )
            .overlay(
                RoundedRectangle(cornerRadius: OnboardingRadius.large)
                    .strokeBorder(.quaternary, lineWidth: 1)
            )
    }
}

/// A choice card's background: accent tint and border when selected.
struct OnboardingChoiceBackground: ViewModifier {
    let selected: Bool
    var radius: CGFloat = OnboardingRadius.large

    func body(content: Content) -> some View {
        content
            .background(selected ? Color.accentColor.opacity(0.14) : Color.primary.opacity(0.05))
            .clipShape(RoundedRectangle(cornerRadius: radius))
            .overlay(
                RoundedRectangle(cornerRadius: radius)
                    .strokeBorder(selected ? Color.accentColor : Color.clear, lineWidth: 1.5)
            )
    }
}

extension View {
    func onboardingCard() -> some View {
        modifier(OnboardingCardBackground())
    }

    func onboardingChoice(selected: Bool, radius: CGFloat = OnboardingRadius.large) -> some View {
        modifier(OnboardingChoiceBackground(selected: selected, radius: radius))
    }
}

/// A step's title and optional subtitle.
struct OnboardingStepHeader: View {
    let title: String
    var subtitle: String?

    var body: some View {
        VStack(spacing: 6) {
            Text(title)
                .font(.system(size: 26, weight: .bold))
                .accessibilityAddTraits(.isHeader)
            if let subtitle {
                Text(subtitle)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            }
        }
        .frame(maxWidth: .infinity)
        .padding(.top, 16)
        .padding(.bottom, 8)
    }
}

/// The Fn warning with its "Open Keyboard Settings" button (HOT-35).
struct OnboardingFnWarning: View {
    let message: String
    let buttonTitle: String
    let open: () -> Void

    var body: some View {
        HStack(spacing: 8) {
            Label {
                Text(message)
                    .fixedSize(horizontal: false, vertical: true)
            } icon: {
                Image(systemName: "exclamationmark.triangle.fill")
                    .foregroundStyle(.orange)
            }
            .font(.callout)
            Spacer(minLength: 0)
            Button(buttonTitle, action: open)
        }
    }
}
