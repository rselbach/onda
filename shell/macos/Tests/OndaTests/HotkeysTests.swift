import Carbon
import CoreGraphics
@testable import Onda
import OndaCore
import Testing

@MainActor
struct EventTapTests {
    @Test(
        arguments: [CGEventType.keyDown, .flagsChanged, .tapDisabledByTimeout, .tapDisabledByUserInput]
    )
    func `HOT-15: the tap callback returns the same event unchanged`(type: CGEventType) throws {
        let event = try #require(CGEvent(keyboardEventSource: nil, virtualKey: CGKeyCode(kVK_ANSI_A), keyDown: true))
        event.flags = [.maskAlternate, .maskShift]
        let proxy = try #require(OpaquePointer(bitPattern: 1))

        let returned = GlobalHotkeys.tapCallback(proxy, type, event, nil)

        let same = try #require(returned).takeUnretainedValue()
        #expect(same === event)
        #expect(same.flags == [.maskAlternate, .maskShift])
        #expect(same.getIntegerValueField(.keyboardEventKeycode) == Int64(kVK_ANSI_A))
    }
}

struct KeyCodeTableTests {
    struct Mapping: Sendable, CustomTestStringConvertible {
        let code: String
        let virtualKey: Int
        var testDescription: String {
            code
        }
    }

    @Test(
        arguments: [
            Mapping(code: "KeyA", virtualKey: kVK_ANSI_A),
            Mapping(code: "KeyZ", virtualKey: kVK_ANSI_Z),
            Mapping(code: "Digit0", virtualKey: kVK_ANSI_0),
            Mapping(code: "Space", virtualKey: kVK_Space),
            Mapping(code: "Enter", virtualKey: kVK_Return),
            Mapping(code: "Backspace", virtualKey: kVK_Delete),
            Mapping(code: "Delete", virtualKey: kVK_ForwardDelete),
            Mapping(code: "Escape", virtualKey: kVK_Escape),
            Mapping(code: "Backquote", virtualKey: kVK_ANSI_Grave),
            Mapping(code: "IntlBackslash", virtualKey: kVK_ISO_Section),
            Mapping(code: "ArrowUp", virtualKey: kVK_UpArrow),
            Mapping(code: "F20", virtualKey: kVK_F20),
            Mapping(code: "NumpadEnter", virtualKey: kVK_ANSI_KeypadEnter),
            Mapping(code: "NumLock", virtualKey: kVK_ANSI_KeypadClear)
        ]
    )
    func `HOT-1: W3C codes and macOS virtual keys map both ways`(mapping: Mapping) {
        #expect(KeyCodes.virtualKey(forCode: mapping.code) == UInt32(mapping.virtualKey))
        #expect(KeyCodes.code(forVirtualKey: Int64(mapping.virtualKey)) == mapping.code)
    }

    @Test(
        arguments: [
            Mapping(code: "ControlLeft", virtualKey: kVK_Control),
            Mapping(code: "ControlRight", virtualKey: kVK_RightControl),
            Mapping(code: "AltLeft", virtualKey: kVK_Option),
            Mapping(code: "AltRight", virtualKey: kVK_RightOption),
            Mapping(code: "ShiftLeft", virtualKey: kVK_Shift),
            Mapping(code: "ShiftRight", virtualKey: kVK_RightShift),
            Mapping(code: "MetaLeft", virtualKey: kVK_Command),
            Mapping(code: "MetaRight", virtualKey: kVK_RightCommand),
            Mapping(code: "Fn", virtualKey: kVK_Function)
        ]
    )
    func `HOT-16: each modifier key reports its own side`(mapping: Mapping) {
        #expect(KeyCodes.code(forVirtualKey: Int64(mapping.virtualKey)) == mapping.code)
    }

    @Test
    func `Insert and Help share a key; the reverse lookup picks Help`() {
        #expect(KeyCodes.virtualKey(forCode: "Insert") == UInt32(kVK_Help))
        #expect(KeyCodes.code(forVirtualKey: Int64(kVK_Help)) == "Help")
    }

    @Test
    func `unknown codes and keys have no mapping`() {
        #expect(KeyCodes.virtualKey(forCode: "Bogus") == nil)
        #expect(KeyCodes.virtualKey(forCode: KeyCodes.unidentified) == nil)
        #expect(KeyCodes.code(forVirtualKey: 0xFF) == nil)
    }
}

struct FlagsChangeTests {
    struct Case: Sendable, CustomTestStringConvertible {
        let name: String
        let virtualKey: Int
        let flags: UInt64
        let want: HotkeyInput?
        var testDescription: String {
            name
        }
    }

    private static let control = CGEventFlags.maskControl.rawValue
    private static let shift = CGEventFlags.maskShift.rawValue
    private static let command = CGEventFlags.maskCommand.rawValue
    private static let option = CGEventFlags.maskAlternate.rawValue
    private static let function = CGEventFlags.maskSecondaryFn.rawValue

    @Test(
        arguments: [
            Case(
                name: "left control down",
                virtualKey: kVK_Control, flags: control | 0x0001,
                want: .modifierFlags(key: "ControlLeft", familyFlagSet: true, sideFlagSet: true)
            ),
            Case(
                name: "right control down",
                virtualKey: kVK_RightControl, flags: control | 0x2000,
                want: .modifierFlags(key: "ControlRight", familyFlagSet: true, sideFlagSet: true)
            ),
            Case(
                name: "left shift down",
                virtualKey: kVK_Shift, flags: shift | 0x0002,
                want: .modifierFlags(key: "ShiftLeft", familyFlagSet: true, sideFlagSet: true)
            ),
            Case(
                name: "right shift down",
                virtualKey: kVK_RightShift, flags: shift | 0x0004,
                want: .modifierFlags(key: "ShiftRight", familyFlagSet: true, sideFlagSet: true)
            ),
            Case(
                name: "left command down",
                virtualKey: kVK_Command, flags: command | 0x0008,
                want: .modifierFlags(key: "MetaLeft", familyFlagSet: true, sideFlagSet: true)
            ),
            Case(
                name: "right command down",
                virtualKey: kVK_RightCommand, flags: command | 0x0010,
                want: .modifierFlags(key: "MetaRight", familyFlagSet: true, sideFlagSet: true)
            ),
            Case(
                name: "left option down",
                virtualKey: kVK_Option, flags: option | 0x0020,
                want: .modifierFlags(key: "AltLeft", familyFlagSet: true, sideFlagSet: true)
            ),
            Case(
                name: "right option down",
                virtualKey: kVK_RightOption, flags: option | 0x0040,
                want: .modifierFlags(key: "AltRight", familyFlagSet: true, sideFlagSet: true)
            ),
            Case(
                name: "right option released while left option stays held",
                virtualKey: kVK_RightOption, flags: option | 0x0020,
                want: .modifierFlags(key: "AltRight", familyFlagSet: true, sideFlagSet: false)
            ),
            Case(
                name: "right option released alone",
                virtualKey: kVK_RightOption, flags: 0,
                want: .modifierFlags(key: "AltRight", familyFlagSet: false, sideFlagSet: false)
            ),
            Case(
                name: "fn has no sides, so its side follows the family",
                virtualKey: kVK_Function, flags: function,
                want: .modifierFlags(key: "Fn", familyFlagSet: true, sideFlagSet: true)
            ),
            Case(
                name: "fn released",
                virtualKey: kVK_Function, flags: 0,
                want: .modifierFlags(key: "Fn", familyFlagSet: false, sideFlagSet: false)
            ),
            Case(
                name: "a key without flag bits reports neither",
                virtualKey: kVK_ANSI_A, flags: option,
                want: .modifierFlags(key: "KeyA", familyFlagSet: false, sideFlagSet: false)
            ),
            Case(
                name: "a key without a code is dropped",
                virtualKey: 0xFF, flags: option,
                want: nil
            )
        ]
    )
    func `HOT-16: side bits come from NX_DEVICE*KEYMASK`(tc: Case) {
        let got = KeyCodes.flagsChange(virtualKey: Int64(tc.virtualKey), flags: CGEventFlags(rawValue: tc.flags))
        #expect(got == tc.want)
    }
}
