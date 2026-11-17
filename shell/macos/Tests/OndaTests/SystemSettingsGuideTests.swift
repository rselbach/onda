import AppKit
@testable import Onda
import Testing

struct GuideLayoutTests {
    private static let window = NSSize(width: 720, height: 560)
    private static let visible = NSRect(x: 0, y: 0, width: 2000, height: 1100)

    @Test(arguments: [
        ("room on the right", NSRect(x: 100, y: 300, width: 715, height: 700), 831.0),
        ("room only on the left", NSRect(x: 1200, y: 300, width: 715, height: 700), 464.0),
        ("no room: against the right edge", NSRect(x: 600, y: 300, width: 715, height: 700), 1280.0)
    ])
    func `ONB-6: the window sits beside System Settings`(name: String, settings: NSRect, wantX: Double) {
        let frame = GuideLayout.frame(for: Self.window, beside: settings, in: Self.visible)
        #expect(frame.minX == CGFloat(wantX), "\(name)")
        #expect(frame.maxY == settings.maxY, "\(name): tops line up")
        #expect(frame.size == Self.window, "\(name)")
    }

    @Test
    func `ONB-6: a tall System Settings never pushes the window off screen`() {
        let settings = NSRect(x: 100, y: -200, width: 715, height: 1500)
        let frame = GuideLayout.frame(for: Self.window, beside: settings, in: Self.visible)
        #expect(frame.maxY == Self.visible.maxY)
        #expect(frame.minY >= Self.visible.minY)
    }

    @Test
    func `ONB-6: quartz bounds flip to cocoa frames`() {
        let quartz = CGRect(x: 40, y: 100, width: 715, height: 700)
        let cocoa = GuideLayout.cocoaRect(fromQuartz: quartz, mainDisplayHeight: 1117)
        #expect(cocoa == NSRect(x: 40, y: 317, width: 715, height: 700))
    }
}
