import AppKit
import OndaCore
import Observation
import SwiftUI

/// What the recording overlay shows: the core's `OverlayState` plus the
/// resolved app icon.
@MainActor
@Observable
final class RecordingOverlayModel {
    var state: OverlayState?
    var icon: NSImage?
}

/// The recording overlay (OVL-1..OVL-15). Renders `OverlayChanged`
/// verbatim; the core picks the mode, the clock, the notice lifecycle,
/// and when the panel shows.
@MainActor
final class RecordingOverlay {
    private let model = RecordingOverlayModel()
    private let panel: OverlayPanel<RecordingOverlayView>
    private var shown = false

    init(onCancel: @escaping @MainActor () -> Void) {
        panel = OverlayPanel(
            size: NSSize(width: 400, height: 104),
            fade: PanelFade.recordingOverlay(),
            content: RecordingOverlayView(model: model, onCancel: onCancel)
        )
    }

    func render(_ state: OverlayState) {
        // a hidden state keeps the last content on screen while it fades out
        if state.visible {
            if !shown || model.state?.icon != state.icon {
                model.icon = AppIcon.image(for: state.icon)
            }
            model.state = state
        }
        shown = state.visible
        panel.setVisible(state.visible)
    }
}

/// macOS rendering of the overlay's leading icon (OVL-6, OUT-12). Onda's
/// own icon is never shown.
enum AppIcon {
    static func image(for icon: OverlayIconState) -> NSImage? {
        let own = Bundle.main.bundleIdentifier
        switch icon {
        case .none:
            return nil
        case let .app(bundleId):
            guard bundleId != own,
                  let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundleId)
            else { return nil }
            return NSWorkspace.shared.icon(forFile: url.path)
        case .frontmostApp:
            guard let frontmost = NSWorkspace.shared.frontmostApplication,
                  frontmost.bundleIdentifier != own
            else { return nil }
            return frontmost.icon
        }
    }
}

struct RecordingOverlayView: View {
    let model: RecordingOverlayModel
    let onCancel: @MainActor () -> Void

    var body: some View {
        ZStack {
            PanelBackground()
            if let state = model.state {
                HStack(spacing: 10) {
                    if let icon = model.icon {
                        Image(nsImage: icon)
                            .resizable()
                            .interpolation(.high)
                            .aspectRatio(contentMode: .fit)
                            .frame(width: 20, height: 20)
                            .clipShape(RoundedRectangle(cornerRadius: 4, style: .continuous))
                            .padding(.leading, 10)
                    }
                    OverlayContentView(content: state.content)
                    if let trailing = state.trailing {
                        HStack(spacing: 8) {
                            if let clock = trailing.clock {
                                Text(clock.text)
                                    .font(.caption.monospacedDigit())
                                    .foregroundStyle(clock.warning ? Color.orange : Color.secondary)
                            }
                            Button(action: onCancel) {
                                Image(systemName: "xmark.circle.fill")
                                    .font(.title3)
                                    .foregroundStyle(.secondary)
                            }
                            .buttonStyle(.plain)
                            .help(trailing.cancelTooltip)
                            .accessibilityLabel(trailing.cancelAccessibilityLabel)
                        }
                        .padding(.trailing, 12)
                    }
                }
            }
        }
    }
}

struct OverlayContentView: View {
    let content: OverlayContentState

    var body: some View {
        switch content {
        case let .notice(title, message):
            messageRow(symbol: "exclamationmark.triangle.fill", tint: .red, title: title, body: message, lines: 3)
        case let .silentMicrophone(title, message):
            messageRow(symbol: "mic.slash.fill", tint: .orange, title: title, body: message, lines: 2)
        case let .processing(label, preview):
            VStack(spacing: 8) {
                ProcessingWaveformView()
                    .frame(height: 16)
                Text(label)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                if let preview {
                    Text(preview)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            .padding(.horizontal, 18)
            .padding(.vertical, 12)
        case let .preview(text, levels):
            VStack(alignment: .leading, spacing: 8) {
                Text(text)
                    .font(.callout)
                    .lineLimit(2)
                    .frame(maxWidth: .infinity, alignment: .leading)
                WaveformBarsView(levels: levels)
                    .frame(height: 24)
            }
            .padding(.horizontal, 18)
            .padding(.vertical, 12)
        case let .waveform(levels):
            WaveformBarsView(levels: levels)
                .padding(.horizontal, 20)
                .padding(.vertical, 15)
        }
    }

    private func messageRow(symbol: String, tint: Color, title: String, body: String, lines: Int) -> some View {
        HStack(spacing: 10) {
            Image(systemName: symbol)
                .foregroundStyle(tint)
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                    .font(.headline)
                    .lineLimit(1)
                Text(body)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .lineLimit(lines)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.horizontal, 18)
        .padding(.vertical, 12)
    }
}

/// OVL-8: capsule bars symmetric around the center line, easing over
/// 0.1 s. The core lays them out (`waveformBars`): which levels fit, newest
/// on the right, and each bar's size.
struct WaveformBarsView: View {
    let levels: [Float]

    var body: some View {
        GeometryReader { geometry in
            let bars = waveformBars(levels: levels, width: geometry.size.width, height: geometry.size.height)
            HStack(spacing: bars.spacing) {
                ForEach(bars.heights.indices, id: \.self) { index in
                    Capsule(style: .continuous)
                        .fill(Color.accentColor)
                        .frame(width: bars.barWidth, height: bars.heights[index])
                }
            }
            .shadow(color: Color.accentColor.opacity(0.35), radius: 1.5)
            .frame(width: geometry.size.width, height: geometry.size.height)
        }
        .animation(.easeOut(duration: 0.1), value: levels)
    }
}

/// OVL-10: idle dots with a highlight sweeping left to right every 1.05 s,
/// skipped under Reduce Motion.
struct ProcessingWaveformView: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var sweep = false

    var body: some View {
        WaveformBarsView(levels: [])
            .overlay {
                if !reduceMotion {
                    GeometryReader { geometry in
                        let band = geometry.size.width * 0.35
                        LinearGradient(
                            colors: [.clear, .white.opacity(0.9), .clear],
                            startPoint: .leading,
                            endPoint: .trailing
                        )
                        .frame(width: band)
                        .offset(x: sweep ? geometry.size.width : -band)
                    }
                    .mask(WaveformBarsView(levels: []))
                }
            }
            .onAppear {
                withAnimation(.linear(duration: 1.05).repeatForever(autoreverses: false)) {
                    sweep = true
                }
            }
    }
}
