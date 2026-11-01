import AppKit
import OndaCore

/// macOS rendering of the platform-neutral tray icon (07 "Tray icon").
enum TrayImage {
    static func make(for icon: TrayIcon, label: String) -> NSImage? {
        switch icon {
        case .ready:
            symbol("waveform", label: label)
        case .recording:
            symbol("waveform.circle.fill", label: label)
        case .transcribing:
            symbol("ellipsis.circle", label: label)
        case .preparing:
            symbol("arrow.down.circle", label: label)
        case let .progressRing(progress):
            progressRing(progress, label: label)
        case .warning:
            symbol("exclamationmark.triangle", label: label)
        }
    }

    private static func symbol(_ name: String, label: String) -> NSImage? {
        let image = NSImage(systemSymbolName: name, accessibilityDescription: label)
        image?.isTemplate = true
        return image
    }

    /// UI-5: faint full track, an arc filling clockwise from 12 o'clock, and
    /// a small bold down arrow in the center.
    private static func progressRing(_ progress: Double, label: String) -> NSImage {
        let size = NSSize(width: 18, height: 18)
        let fraction = min(max(progress, 0), 1)
        let image = NSImage(size: size, flipped: false) { rect in
            let center = NSPoint(x: rect.midX, y: rect.midY)
            let radius = (min(rect.width, rect.height) - 2) / 2

            let track = NSBezierPath()
            track.appendArc(withCenter: center, radius: radius, startAngle: 0, endAngle: 360)
            track.lineWidth = 1.5
            NSColor.black.withAlphaComponent(0.25).setStroke()
            track.stroke()

            if fraction > 0 {
                let arc = NSBezierPath()
                arc.appendArc(
                    withCenter: center,
                    radius: radius,
                    startAngle: 90,
                    endAngle: 90 - 360 * fraction,
                    clockwise: true
                )
                arc.lineWidth = 2
                arc.lineCapStyle = .round
                NSColor.black.setStroke()
                arc.stroke()
            }

            let config = NSImage.SymbolConfiguration(pointSize: 8, weight: .bold)
            if let arrow = NSImage(systemSymbolName: "arrow.down", accessibilityDescription: nil)?
                .withSymbolConfiguration(config) {
                let arrowSize = arrow.size
                arrow.draw(in: NSRect(
                    x: center.x - arrowSize.width / 2,
                    y: center.y - arrowSize.height / 2,
                    width: arrowSize.width,
                    height: arrowSize.height
                ))
            }
            return true
        }
        image.isTemplate = true
        image.accessibilityDescription = label
        return image
    }
}
