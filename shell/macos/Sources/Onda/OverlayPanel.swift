import AppKit
import OndaCore
import SwiftUI

/// A floating, non-activating panel at the bottom center of the screen
/// with the mouse (OVL-1..OVL-4). The fade rules come from the core's
/// `PanelFade`; this class only animates what it is told.
@MainActor
final class OverlayPanel<Content: View> {
    private let bottomMargin: CGFloat = 100
    private let size: NSSize
    private let fade: PanelFade
    private let panel: NonActivatingPanel

    init(size: NSSize, fade: PanelFade, content: Content) {
        self.size = size
        self.fade = fade
        panel = NonActivatingPanel(
            contentRect: NSRect(origin: .zero, size: size),
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: true
        )
        panel.level = .floating
        panel.hidesOnDeactivate = false
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = true
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary]
        panel.isReleasedWhenClosed = false
        let host = FirstMouseHostingView(rootView: content)
        host.frame = NSRect(origin: .zero, size: size)
        panel.contentView = host
    }

    func setVisible(_ visible: Bool) {
        run(visible ? fade.show() : fade.hide())
    }

    private func run(_ command: PanelFadeCommand) {
        switch command {
        case .none:
            break
        case let .fadeIn(fromCurrent):
            reposition()
            if !fromCurrent {
                panel.alphaValue = 0
            }
            panel.orderFrontRegardless()
            NSAnimationContext.runAnimationGroup { context in
                context.duration = fade.durationSeconds()
                panel.animator().alphaValue = 1
            }
        case .fadeOut:
            NSAnimationContext.runAnimationGroup { context in
                context.duration = fade.durationSeconds()
                panel.animator().alphaValue = 0
            } completionHandler: { [weak self] in
                // AppKit calls this on the main thread
                MainActor.assumeIsolated {
                    guard let self else { return }
                    self.run(self.fade.fadeOutFinished())
                }
            }
        case .orderOut:
            panel.orderOut(nil)
            panel.alphaValue = 1
        }
    }

    /// OVL-1: recomputed on every show so a disconnected display never
    /// strands the panel off screen.
    private func reposition() {
        let mouse = NSEvent.mouseLocation
        let screen = NSScreen.screens.first { $0.frame.contains(mouse) } ?? NSScreen.main
        guard let visible = screen?.visibleFrame else { return }
        let origin = NSPoint(x: visible.midX - size.width / 2, y: visible.minY + bottomMargin)
        panel.setFrame(NSRect(origin: origin, size: size), display: true)
    }
}

/// Never key or main, so it never takes focus from the target app (OVL-3).
final class NonActivatingPanel: NSPanel {
    override var canBecomeKey: Bool {
        false
    }

    override var canBecomeMain: Bool {
        false
    }
}

/// The panel is never key, so every click is a first click; accepting it
/// lets buttons work on the first press (OVL-3).
final class FirstMouseHostingView<Content: View>: NSHostingView<Content> {
    override func acceptsFirstMouse(for _: NSEvent?) -> Bool {
        true
    }
}

/// The overlay background (OVL-2).
struct PanelBackground: View {
    var body: some View {
        RoundedRectangle(cornerRadius: 12, style: .continuous)
            .fill(.ultraThinMaterial)
    }
}
