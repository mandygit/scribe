// Spike: can Scribe use the Fn / Globe key as its dictation hotkey?
//
//   ./fn-key-tap            listen-only tap (safe: never swallows anything)
//   ./fn-key-tap --consume  active tap that swallows a *bare* Fn tap, to see
//                           whether that suppresses the OS's own Globe action
//
// Questions it answers:
//   1. Does a bare Fn press arrive as a flagsChanged event (keycode 63)?
//   2. Can we tell a bare Fn tap from Fn-as-a-modifier (fn+arrow, fn+F3)?
//   3. Can an active tap suppress the system Globe-key action?
import Cocoa

let kVKFunction: Int64 = 0x3F  // 63
let consume = CommandLine.arguments.contains("--consume")

print("AXIsProcessTrusted = \(AXIsProcessTrusted())")

var fnDownAt: CFAbsoluteTime = 0
var sawOtherKeyWhileFnDown = false
var lastBareTapAt: CFAbsoluteTime = 0
var otherKeyCount = 0
var fnEventCount = 0

let mask = (1 << CGEventType.flagsChanged.rawValue) | (1 << CGEventType.keyDown.rawValue)

guard let tap = CGEvent.tapCreate(
    tap: .cgSessionEventTap,
    place: .headInsertEventTap,
    options: consume ? .defaultTap : .listenOnly,
    eventsOfInterest: CGEventMask(mask),
    callback: { _, type, event, _ in
        // A slow callback gets the tap disabled by the system; production code
        // must re-enable it here.
        if type == .tapDisabledByTimeout || type == .tapDisabledByUserInput {
            print("!! tap disabled (\(type.rawValue)) - would re-enable")
            return Unmanaged.passUnretained(event)
        }
        let code = event.getIntegerValueField(.keyboardEventKeycode)
        let flags = event.flags
        let fnDown = flags.contains(.maskSecondaryFn)
        let now = CFAbsoluteTimeGetCurrent()

        if type == .keyDown {
            if fnDownAt != 0 { sawOtherKeyWhileFnDown = true }
            otherKeyCount += 1
            // Deliberately never logs *which* key: the classifier only needs
            // "some other key happened while Fn was down".
            print("  (other key) fn=\(fnDown ? "Y" : "N")")
        } else if type == .flagsChanged, code == kVKFunction {
            // Other modifiers held alongside Fn disqualify it as a bare tap.
            let otherMods = flags.intersection([.maskCommand, .maskShift, .maskControl, .maskAlternate])
            if fnDown {
                fnDownAt = now
                sawOtherKeyWhileFnDown = !otherMods.isEmpty
                fnEventCount += 1
                print("FN DOWN")
            } else {
                let heldMs = fnDownAt == 0 ? -1 : Int((now - fnDownAt) * 1000)
                let bare = !sawOtherKeyWhileFnDown && fnDownAt != 0 && otherMods.isEmpty
                let gap = lastBareTapAt == 0 ? -1 : Int((now - lastBareTapAt) * 1000)
                print("FN UP    held=\(heldMs)ms bareTap=\(bare) gapSinceLastTap=\(gap)ms"
                      + (bare && gap >= 0 && gap <= 600 ? "   <-- DOUBLE-TAP" : ""))
                if bare { lastBareTapAt = now }
                fnDownAt = 0
                if consume && bare { return nil }  // swallow it
            }
            if consume && fnDown { return nil }
        } else if type == .flagsChanged {
            otherKeyCount += 1
            print("  (other modifier)")
        }
        return Unmanaged.passUnretained(event)
    },
    userInfo: nil
) else {
    print("RESULT: tapCreate FAILED - permission missing")
    exit(1)
}

let source = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, tap, 0)
CFRunLoopAddSource(CFRunLoopGetCurrent(), source, .commonModes)
CGEvent.tapEnable(tap: tap, enable: true)
print("RESULT: tap created OK (mode: \(consume ? "CONSUME" : "listen-only")). Listening 10 min; ctrl-c to stop.")
print("Try: (1) tap Fn twice quickly  (2) tap Fn once  (3) fn+ArrowLeft  (4) fn+F3  (5) type a letter")
fflush(stdout)

let started = CFAbsoluteTimeGetCurrent()
let heartbeat = Timer(timeInterval: 20, repeats: true) { _ in
    let elapsed = Int(CFAbsoluteTimeGetCurrent() - started)
    if fnEventCount == 0 && otherKeyCount == 0 {
        print("[\(elapsed)s] still nothing - either no keys pressed, or events aren't reaching the tap")
    } else {
        print("[\(elapsed)s] fnEvents=\(fnEventCount) otherKeys=\(otherKeyCount)")
    }
    fflush(stdout)
}
RunLoop.current.add(heartbeat, forMode: .common)
DispatchQueue.main.asyncAfter(deadline: .now() + 600) { print("done (10 min)."); exit(0) }
CFRunLoopRun()
