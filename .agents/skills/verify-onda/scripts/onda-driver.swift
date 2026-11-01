// onda-driver: synthetic HID input and accessibility reads for verify-onda.
//
// verify-onda compiles this file once per source hash and calls it; run it
// through verify-onda rather than directly. Keyboard and mouse events post
// at the HID tap, so Onda's Carbon hotkey and its lone-modifier event tap
// see them as real input. Every command sleeps briefly before exiting:
// events posted right before exit can be dropped, and a lost Option key-up
// leaves Option "held" for the rest of the session.
import ApplicationServices
import CoreGraphics
import Foundation

let source = CGEventSource(stateID: .hidSystemState)

// virtual key, family flag, and IOKit per-side device bit (0: no side)
let modifiers: [String: (key: CGKeyCode, family: CGEventFlags, side: UInt64)] = [
    "option": (58, .maskAlternate, 0x20),
    "right-option": (61, .maskAlternate, 0x40),
    "control": (59, .maskControl, 0x1),
    "right-control": (62, .maskControl, 0x2000),
    "shift": (56, .maskShift, 0x2),
    "right-shift": (60, .maskShift, 0x4),
    "command": (55, .maskCommand, 0x8),
    "right-command": (54, .maskCommand, 0x10),
    "fn": (63, .maskSecondaryFn, 0)
]

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data("onda-driver: \(message)\n".utf8))
    exit(1)
}

func settle() {
    usleep(300_000)
}

func postKey(_ key: CGKeyCode, down: Bool, flags: CGEventFlags) {
    guard let event = CGEvent(keyboardEventSource: source, virtualKey: key, keyDown: down) else {
        fail("could not create a key event")
    }
    event.flags = flags
    event.post(tap: .cghidEventTap)
}

func postModifier(_ name: String, down: Bool) {
    guard let modifier = modifiers[name] else {
        fail("unknown modifier \(name); use one of \(modifiers.keys.sorted().joined(separator: ", "))")
    }
    guard let event = CGEvent(keyboardEventSource: source, virtualKey: modifier.key, keyDown: down) else {
        fail("could not create a modifier event")
    }
    event.type = .flagsChanged
    event.flags = down ? CGEventFlags(rawValue: modifier.family.rawValue | modifier.side) : []
    event.post(tap: .cghidEventTap)
}

// Option+Space, Onda's default hotkey
func combo(seconds: Double) {
    let option = CGEventFlags(rawValue: CGEventFlags.maskAlternate.rawValue | 0x20)
    postModifier("option", down: true)
    usleep(50000)
    postKey(49, down: true, flags: option)
    Thread.sleep(forTimeInterval: seconds)
    postKey(49, down: false, flags: option)
    usleep(50000)
    postModifier("option", down: false)
    settle()
}

func lone(_ name: String, seconds: Double) {
    postModifier(name, down: true)
    Thread.sleep(forTimeInterval: seconds)
    postModifier(name, down: false)
    settle()
}

func escape(withOption: Bool) {
    let option = CGEventFlags(rawValue: CGEventFlags.maskAlternate.rawValue | 0x20)
    if withOption {
        postModifier("option", down: true)
        usleep(50000)
    }
    postKey(53, down: true, flags: withOption ? option : [])
    usleep(50000)
    postKey(53, down: false, flags: withOption ? option : [])
    if withOption {
        usleep(50000)
        postModifier("option", down: false)
    }
    settle()
}

func click(x: Double, y: Double) {
    let point = CGPoint(x: x, y: y)
    for type in [CGEventType.leftMouseDown, .leftMouseUp] {
        guard let event = CGEvent(mouseEventSource: source, mouseType: type, mouseCursorPosition: point, mouseButton: .left) else {
            fail("could not create a mouse event")
        }
        event.post(tap: .cghidEventTap)
        usleep(50000)
    }
    settle()
}

// one line per on-screen window of `pid`: id, layer, x, y, width, height,
// alpha, name (tab-separated, front to back)
func windows(pid: pid_t) {
    guard let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]] else {
        fail("could not list windows")
    }
    for info in list where (info[kCGWindowOwnerPID as String] as? pid_t) == pid {
        let id = info[kCGWindowNumber as String] as? Int ?? 0
        let layer = info[kCGWindowLayer as String] as? Int ?? 0
        let alpha = info[kCGWindowAlpha as String] as? Double ?? 0
        let name = info[kCGWindowName as String] as? String ?? ""
        let bounds = info[kCGWindowBounds as String] as? [String: Double] ?? [:]
        let fields = [
            "\(id)", "\(layer)",
            "\(Int(bounds["X"] ?? 0))", "\(Int(bounds["Y"] ?? 0))",
            "\(Int(bounds["Width"] ?? 0))", "\(Int(bounds["Height"] ?? 0))",
            String(format: "%.2f", alpha), name
        ]
        print(fields.joined(separator: "\t"))
    }
}

func attribute(_ element: AXUIElement, _ name: String) -> AnyObject? {
    var value: AnyObject?
    guard AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success else {
        return nil
    }
    return value
}

func children(_ element: AXUIElement) -> [AXUIElement] {
    attribute(element, kAXChildrenAttribute) as? [AXUIElement] ?? []
}

func title(_ element: AXUIElement) -> String {
    attribute(element, kAXTitleAttribute) as? String ?? ""
}

func statusItem(pid: pid_t) -> AXUIElement {
    guard AXIsProcessTrusted() else {
        fail("this process needs Accessibility permission (grant it to the terminal or agent app)")
    }
    let app = AXUIElementCreateApplication(pid)
    AXUIElementSetMessagingTimeout(app, 2)
    guard let extras = attribute(app, "AXExtrasMenuBar"), CFGetTypeID(extras) == AXUIElementGetTypeID() else {
        fail("PID \(pid) has no status item")
    }
    // swiftlint:disable:next force_cast
    guard let item = children(extras as! AXUIElement).first else {
        fail("PID \(pid) has no status item")
    }
    return item
}

func trayLabel(pid: pid_t) {
    let item = statusItem(pid: pid)
    let label = attribute(item, kAXDescriptionAttribute) as? String ?? ""
    print(label.isEmpty ? title(item) : label)
}

// the status menu; its items are readable and pressable without showing
// it, so it is only shown on screen when `show` is set
func openMenu(pid: pid_t, show: Bool = false) -> (item: AXUIElement, menu: AXUIElement) {
    let item = statusItem(pid: pid)
    if !show, let menu = children(item).first, !children(menu).isEmpty {
        return (item, menu)
    }
    // AXPress on a status item can block until the menu closes; the
    // messaging timeout bounds it, so the result is not checked here
    AXUIElementSetMessagingTimeout(item, 1)
    _ = AXUIElementPerformAction(item, kAXPressAction as CFString)
    for _ in 0 ..< 30 {
        if let menu = children(item).first, !children(menu).isEmpty {
            return (item, menu)
        }
        usleep(100_000)
    }
    fail("the status menu did not open")
}

func closeMenu(_ menu: AXUIElement) {
    _ = AXUIElementPerformAction(menu, kAXCancelAction as CFString)
    settle()
}

func menu(pid: pid_t, action: String, itemTitle: String?) {
    switch action {
    case "list":
        let (_, menu) = openMenu(pid: pid)
        for entry in children(menu) {
            let name = title(entry)
            let enabled = attribute(entry, kAXEnabledAttribute) as? Bool ?? false
            print(name.isEmpty ? "----" : "\(name)\(enabled ? "" : "\t(disabled)")")
        }
        closeMenu(menu)
    case "open":
        _ = openMenu(pid: pid, show: true)
        settle()
    case "close":
        let item = statusItem(pid: pid)
        if let menu = children(item).first {
            closeMenu(menu)
        }
    case "press":
        guard let itemTitle else { fail("menu press needs an item title") }
        let (_, menu) = openMenu(pid: pid)
        guard let entry = children(menu).first(where: { title($0) == itemTitle }) else {
            let titles = children(menu).map(title).filter { !$0.isEmpty }
            closeMenu(menu)
            fail("no menu item \"\(itemTitle)\"; items: \(titles.joined(separator: " | "))")
        }
        guard AXUIElementPerformAction(entry, kAXPressAction as CFString) == .success else {
            closeMenu(menu)
            fail("could not press \"\(itemTitle)\"")
        }
        settle()
    default:
        fail("unknown menu action \(action)")
    }
}

func number(_ text: String?, _ what: String) -> Double {
    guard let text, let value = Double(text) else { fail("\(what) must be a number") }
    return value
}

func pid(_ text: String?) -> pid_t {
    guard let text, let value = pid_t(text) else { fail("PID must be a number") }
    return value
}

let args = Array(CommandLine.arguments.dropFirst())
switch args.first {
case "combo":
    combo(seconds: number(args.dropFirst().first, "SECONDS"))
case "lone":
    guard args.count == 3 else { fail("usage: lone MODIFIER SECONDS") }
    lone(args[1], seconds: number(args[2], "SECONDS"))
case "escape":
    escape(withOption: args.dropFirst().first == "--option")
case "click":
    guard args.count == 3 else { fail("usage: click X Y") }
    click(x: number(args[1], "X"), y: number(args[2], "Y"))
case "windows":
    windows(pid: pid(args.dropFirst().first))
case "tray":
    trayLabel(pid: pid(args.dropFirst().first))
case "menu":
    guard args.count >= 3 else { fail("usage: menu PID list|open|close|press TITLE") }
    menu(pid: pid(args[1]), action: args[2], itemTitle: args.count > 3 ? args[3] : nil)
default:
    fail("usage: combo SECONDS | lone MODIFIER SECONDS | escape [--option] | click X Y | windows PID | tray PID | menu PID ACTION [TITLE]")
}
