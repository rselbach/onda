import AppKit
import SwiftUI

/// Walks the user through a grant in System Settings (ONB-6). Once the core
/// has opened the pane, the onboarding window floats beside System Settings
/// so both stay in view. For a list that takes dropped apps, a copy of
/// Onda's icon then floats from the card into the list to show that it can
/// be dragged there.
@MainActor
final class SystemSettingsGuide {
    private static let settingsBundleID = "com.apple.systempreferences"

    private weak var window: NSWindow?
    private var phantom: NSPanel?
    /// bumped by `end()`, so a grant that lands mid-walk stops it
    private var session = 0

    func begin(beside window: NSWindow, dropsApp: Bool) async {
        end()
        session += 1
        let current = session
        self.window = window
        // floating keeps the card in view while System Settings is active
        window.level = .floating

        guard let settingsFrame = await settledSettingsWindowFrame(), current == session else { return }
        let screen = NSScreen.screens.first { $0.frame.intersects(settingsFrame) } ?? NSScreen.main
        if let visible = screen?.visibleFrame {
            let frame = GuideLayout.frame(for: window.frame.size, beside: settingsFrame, in: visible)
            window.setFrame(frame, display: true, animate: true)
        }

        guard dropsApp,
              let icon = window.contentView?.firstSubview(of: AppIconDragSourceView.self),
              let iconFrame = icon.window?.convertToScreen(icon.convert(icon.bounds, to: nil))
        else { return }
        await showPhantom(from: iconFrame, to: GuideLayout.dropTarget(in: settingsFrame), session: current)
    }

    func end() {
        session += 1
        window?.level = .normal
        window = nil
        phantom?.orderOut(nil)
        phantom = nil
    }

    /// System Settings takes a moment to open and switch panes, so this waits
    /// until its window holds still. Window bounds need no screen recording
    /// permission; only window titles do.
    private func settledSettingsWindowFrame() async -> NSRect? {
        var previous: NSRect?
        for _ in 0 ..< 50 {
            try? await Task.sleep(for: .milliseconds(100))
            guard let frame = settingsWindowFrame() else { continue }
            if frame == previous {
                return frame
            }
            previous = frame
        }
        return previous
    }

    private func settingsWindowFrame() -> NSRect? {
        guard
            let pid = NSRunningApplication.runningApplications(withBundleIdentifier: Self.settingsBundleID).first?
                .processIdentifier,
            let infoList = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID)
            as? [[String: Any]]
        else { return nil }

        for info in infoList {
            guard info[kCGWindowOwnerPID as String] as? pid_t == pid,
                  info[kCGWindowLayer as String] as? Int == 0,
                  let boundsDict = info[kCGWindowBounds as String] as? NSDictionary,
                  let bounds = CGRect(dictionaryRepresentation: boundsDict)
            else { continue }
            return GuideLayout.cocoaRect(fromQuartz: bounds)
        }
        return nil
    }

    private func showPhantom(from start: NSRect, to target: NSPoint, session current: Int) async {
        guard !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion else { return }

        let panel = NSPanel(
            contentRect: start,
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: false
        )
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = true
        panel.ignoresMouseEvents = true
        panel.level = .popUpMenu
        panel.collectionBehavior = [.canJoinAllSpaces, .transient, .ignoresCycle]
        let imageView = NSImageView(image: NSApp.applicationIconImage)
        imageView.imageScaling = .scaleProportionallyUpOrDown
        panel.contentView = imageView
        panel.alphaValue = 0
        panel.orderFrontRegardless()
        phantom = panel

        // lift the copy off the icon, carry it into the list, then let go
        let lifted = start.insetBy(dx: -6, dy: -6)
        let landedSize = NSSize(width: start.width * 0.75, height: start.height * 0.75)
        let landed = NSRect(
            x: target.x - landedSize.width / 2,
            y: target.y - landedSize.height / 2,
            width: landedSize.width,
            height: landedSize.height
        )
        await animate(duration: 0.2, timing: .easeOut) {
            panel.animator().alphaValue = 0.9
            panel.animator().setFrame(lifted, display: true)
        }
        try? await Task.sleep(for: .milliseconds(150))
        guard current == session else { return }
        await animate(duration: 0.9, timing: .easeInEaseOut) {
            panel.animator().setFrame(landed, display: true)
        }
        try? await Task.sleep(for: .milliseconds(450))
        guard current == session else { return }
        await animate(duration: 0.35, timing: .easeIn) {
            panel.animator().alphaValue = 0
        }
        guard phantom === panel else { return }
        panel.orderOut(nil)
        phantom = nil
    }

    private func animate(
        duration: TimeInterval,
        timing: CAMediaTimingFunctionName,
        changes: @escaping () -> Void
    ) async {
        await withCheckedContinuation { continuation in
            NSAnimationContext.runAnimationGroup { context in
                context.duration = duration
                context.timingFunction = CAMediaTimingFunction(name: timing)
                changes()
            } completionHandler: {
                continuation.resume()
            }
        }
    }
}

/// Where the guide puts things on screen, in Cocoa coordinates.
enum GuideLayout {
    static let gap: CGFloat = 16

    /// Beside System Settings: to its right when that fits, else to its left,
    /// else against the right edge of the screen, overlapping it.
    static func frame(for size: NSSize, beside settings: NSRect, in visible: NSRect) -> NSRect {
        let top = min(settings.maxY, visible.maxY)
        let y = max(visible.minY, top - size.height)
        return NSRect(
            x: x(for: size.width, beside: settings, in: visible), y: y, width: size.width, height: size.height
        )
    }

    private static func x(for width: CGFloat, beside settings: NSRect, in visible: NSRect) -> CGFloat {
        if settings.maxX + gap + width <= visible.maxX {
            return settings.maxX + gap
        }
        if settings.minX - gap - width >= visible.minX {
            return settings.minX - gap - width
        }
        return visible.maxX - width
    }

    /// The middle of the app list in a Privacy & Security pane, right of the
    /// sidebar and a little above center.
    static func dropTarget(in settings: NSRect) -> NSPoint {
        NSPoint(x: settings.minX + settings.width * 0.62, y: settings.minY + settings.height * 0.55)
    }

    /// Quartz window bounds have their origin at the top left of the main
    /// display; Cocoa frames at its bottom left.
    static func cocoaRect(fromQuartz rect: CGRect, mainDisplayHeight: CGFloat? = nil) -> NSRect {
        let height = mainDisplayHeight ?? NSScreen.screens.first?.frame.height ?? 0
        return NSRect(x: rect.minX, y: height - rect.maxY, width: rect.width, height: rect.height)
    }
}

/// Onda's icon as a drag source for the app bundle, so it can be dropped
/// onto the Accessibility list in System Settings.
struct DraggableAppIcon: NSViewRepresentable {
    func makeNSView(context _: Context) -> AppIconDragSourceView {
        AppIconDragSourceView()
    }

    func updateNSView(_: AppIconDragSourceView, context _: Context) {}
}

final class AppIconDragSourceView: NSView, NSDraggingSource {
    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        toolTip = "Drag Onda into System Settings"
        setAccessibilityRole(.image)
        setAccessibilityLabel("Onda icon, drag into System Settings")
    }

    @available(*, unavailable)
    required init?(coder _: NSCoder) {
        fatalError("not used from a nib")
    }

    override var mouseDownCanMoveWindow: Bool {
        false
    }

    /// The window sits beside System Settings while that app is active, so
    /// the first drag must not be spent activating Onda.
    override func acceptsFirstMouse(for _: NSEvent?) -> Bool {
        true
    }

    override func draw(_: NSRect) {
        NSApp.applicationIconImage.draw(in: bounds)
    }

    override func resetCursorRects() {
        addCursorRect(bounds, cursor: .openHand)
    }

    override func mouseDown(with _: NSEvent) {}

    override func mouseDragged(with event: NSEvent) {
        let item = NSDraggingItem(pasteboardWriter: Bundle.main.bundleURL as NSURL)
        item.setDraggingFrame(bounds, contents: NSApp.applicationIconImage)
        beginDraggingSession(with: [item], event: event, source: self)
    }

    func draggingSession(
        _: NSDraggingSession,
        sourceOperationMaskFor context: NSDraggingContext
    ) -> NSDragOperation {
        // copy only: a move would let Finder carry the running app away
        context == .outsideApplication ? .copy : []
    }
}

private extension NSView {
    func firstSubview<View: NSView>(of type: View.Type) -> View? {
        for subview in subviews {
            if let match = subview as? View ?? subview.firstSubview(of: type) {
                return match
            }
        }
        return nil
    }
}
