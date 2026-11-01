import OndaCore
import SwiftUI

/// Corner radii shared by the main window pages.
enum MainWindowRadius {
    /// Sidebar icon tiles and keycaps.
    static let small: CGFloat = 6
}

/// A hotkey as individual keycaps, e.g. [⌥] [Space].
struct MainWindowKeycaps: View {
    let labels: [String]

    var body: some View {
        HStack(spacing: 4) {
            ForEach(Array(labels.enumerated()), id: \.offset) { _, label in
                Text(label)
                    .font(.body.weight(.medium))
                    .padding(.horizontal, label.count > 1 ? 8 : 7)
                    .padding(.vertical, 3)
                    .background(
                        RoundedRectangle(cornerRadius: MainWindowRadius.small)
                            .fill(.quaternary)
                            .shadow(color: .black.opacity(0.2), radius: 0, y: 1)
                    )
            }
        }
        .accessibilityElement(children: .combine)
    }
}

/// A small caption under a control (WIN-3): secondary, or orange for a
/// warning.
struct MainWindowCaption: View {
    let text: String
    var warning = false

    var body: some View {
        Text(text)
            .font(.caption)
            .foregroundStyle(warning ? AnyShapeStyle(.orange) : AnyShapeStyle(.secondary))
            .fixedSize(horizontal: false, vertical: true)
    }
}

/// The Fn warning with its button (HOT-35).
struct MainWindowFnWarning: View {
    let message: String
    let buttonTitle: String
    let action: () -> Void

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
            Button(buttonTitle, action: action)
        }
    }
}

/// One permission row: detail line plus a green granted label or a fix
/// button (WIN-8).
struct MainWindowPermissionRow: View {
    let row: PermissionRowView
    let fix: (PermissionFix) -> Void

    var body: some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                Text(row.title)
                MainWindowCaption(text: row.detail, warning: row.detailWarning)
            }
            Spacer()
            if row.granted {
                Label(row.grantedLabel, systemImage: "checkmark.circle.fill")
                    .foregroundStyle(.green)
            } else if let button = row.fix {
                Button(button.title) {
                    fix(button.fix)
                }
            }
        }
    }
}
