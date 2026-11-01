import AppKit
import OndaCore
import Observation
import SwiftUI

@MainActor
@Observable
final class DownloadOverlayModel {
    var state: DownloadOverlayState = .hidden
}

/// The download overlay (OVL-16..OVL-18): the core decides when it shows
/// and what it says; the close button tells the core (OVL-17).
@MainActor
final class DownloadOverlay {
    private let model = DownloadOverlayModel()
    private let panel: OverlayPanel<DownloadOverlayView>

    init(onHide: @escaping @MainActor () -> Void) {
        panel = OverlayPanel(
            size: NSSize(width: 400, height: 72),
            fade: PanelFade.downloadOverlay(),
            content: DownloadOverlayView(model: model, onHide: onHide)
        )
    }

    func render(_ state: DownloadOverlayState) {
        switch state {
        case .hidden:
            // keep the last content on screen while it fades out
            panel.setVisible(false)
        case .shown:
            model.state = state
            panel.setVisible(true)
        }
    }
}

struct DownloadOverlayView: View {
    let model: DownloadOverlayModel
    let onHide: @MainActor () -> Void

    var body: some View {
        ZStack {
            PanelBackground()
            if case let .shown(status, progress, hideTooltip, hideLabel) = model.state {
                HStack(spacing: 12) {
                    Image(systemName: "arrow.down.circle")
                        .font(.title2)
                        .foregroundStyle(.secondary)
                    VStack(alignment: .leading, spacing: 6) {
                        Text(status)
                            .font(.subheadline)
                            .lineLimit(1)
                        if let progress {
                            ProgressView(value: min(max(progress, 0), 1))
                        } else {
                            ProgressView()
                                .progressViewStyle(.linear)
                        }
                    }
                    Button(action: onHide) {
                        Image(systemName: "xmark.circle.fill")
                            .font(.title3)
                            .foregroundStyle(.secondary)
                    }
                    .buttonStyle(.plain)
                    .help(hideTooltip)
                    .accessibilityLabel(hideLabel)
                }
                .padding(.horizontal, 18)
            }
        }
    }
}
