import AppKit
import CoreAudio
import CoreGraphics
import Foundation

/// Microsoft Teams' bundle identifiers: the current native client, and the
/// retired classic Electron-based client some installs may still have.
let teamsBundleIdentifiers: Set<String> = ["com.microsoft.teams2", "com.microsoft.teams"]

/// Whether `bundleId` belongs to Teams -- either its main bundle id exactly,
/// or one of its helper subprocesses (e.g. `com.microsoft.teams2.helper`,
/// confirmed empirically to be where Teams' actual audio capture runs, not
/// the main `com.microsoft.teams2` process). Matches on a `.`-delimited
/// prefix, not a bare string prefix, so a hypothetical unrelated bundle id
/// that merely starts with the same characters wouldn't false-match.
func isTeamsBundleId(_ bundleId: String) -> Bool {
    teamsBundleIdentifiers.contains { base in
        bundleId == base || bundleId.hasPrefix(base + ".")
    }
}

/// A window's on-screen size, in points, as reported by
/// `CGWindowListCopyWindowInfo`'s `kCGWindowBounds` entry.
struct WindowGeometry {
    let ownerName: String
    let title: String
    let width: Double
    let height: Double
    let layer: Int
}

/// Finds the process id of a running Microsoft Teams instance, if any.
func runningTeamsProcessId() -> pid_t? {
    NSWorkspace.shared.runningApplications.first { app in
        guard let bundleIdentifier = app.bundleIdentifier else { return false }
        return teamsBundleIdentifiers.contains(bundleIdentifier)
    }?.processIdentifier
}

/// Lists on-screen windows owned by the given process, via the same
/// Core Graphics window list Scribe's Screen Recording permission already
/// gates for system audio capture -- no additional permission is needed
/// beyond what's already requested.
func onScreenWindows(ownedBy pid: pid_t) -> [WindowGeometry] {
    guard
        let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID)
            as? [[String: AnyObject]]
    else {
        return []
    }

    return list.compactMap { window -> WindowGeometry? in
        guard let ownerPid = window[kCGWindowOwnerPID as String] as? pid_t, ownerPid == pid else {
            return nil
        }
        guard let bounds = window[kCGWindowBounds as String] as? [String: Double] else {
            return nil
        }
        let ownerName = (window[kCGWindowOwnerName as String] as? String) ?? ""
        let title = (window[kCGWindowName as String] as? String) ?? ""
        let layer = (window[kCGWindowLayer as String] as? Int) ?? 0
        return WindowGeometry(
            ownerName: ownerName,
            title: title,
            width: bounds["Width"] ?? 0,
            height: bounds["Height"] ?? 0,
            layer: layer
        )
    }
}

/// All AudioObjectIDs the system currently knows about as distinct
/// "processes" for Core Audio purposes (this is the same mechanism behind
/// the orange microphone-in-use menu bar indicator and Sonoma's per-app
/// audio mixer -- public API, no extra permission needed to query the
/// boolean "is running input" state, only to read actual audio samples).
func audioProcessObjectIds() -> [AudioObjectID] {
    var address = AudioObjectPropertyAddress(
        mSelector: kAudioHardwarePropertyProcessObjectList,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain
    )
    var dataSize: UInt32 = 0
    var status = AudioObjectGetPropertyDataSize(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &dataSize)
    guard status == noErr, dataSize > 0 else { return [] }

    let count = Int(dataSize) / MemoryLayout<AudioObjectID>.size
    var processIds = [AudioObjectID](repeating: 0, count: count)
    status = AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &dataSize, &processIds)
    return status == noErr ? processIds : []
}

func audioObjectProperty<T>(_ objectId: AudioObjectID, selector: AudioObjectPropertySelector, defaultValue: T) -> T {
    var address = AudioObjectPropertyAddress(
        mSelector: selector,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain
    )
    var value = defaultValue
    var size = UInt32(MemoryLayout<T>.size)
    let status = AudioObjectGetPropertyData(objectId, &address, 0, nil, &size, &value)
    return status == noErr ? value : defaultValue
}

/// Core Audio's HAL process-object properties are a CFString for bundle id,
/// which (unlike the fixed-size scalar properties `audioObjectProperty`
/// handles) needs the CFTypeRef "copy" ownership convention.
func audioObjectStringProperty(_ objectId: AudioObjectID, selector: AudioObjectPropertySelector) -> String? {
    var address = AudioObjectPropertyAddress(
        mSelector: selector,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain
    )
    var size: UInt32 = 0
    guard AudioObjectGetPropertyDataSize(objectId, &address, 0, nil, &size) == noErr, size > 0 else {
        return nil
    }
    var unmanagedValue: Unmanaged<CFString>?
    let status = withUnsafeMutablePointer(to: &unmanagedValue) { pointer in
        AudioObjectGetPropertyData(objectId, &address, 0, nil, &size, pointer)
    }
    guard status == noErr, let unmanagedValue else { return nil }
    return unmanagedValue.takeRetainedValue() as String
}

/// One audio-HAL process entry, for both detection and diagnostics.
struct AudioProcessEntry {
    let pid: pid_t
    let bundleId: String?
    let isRunningInput: Bool
}

func audioProcessEntries() -> [AudioProcessEntry] {
    audioProcessObjectIds().map { objectId in
        AudioProcessEntry(
            pid: audioObjectProperty(objectId, selector: kAudioProcessPropertyPID, defaultValue: pid_t(0)),
            bundleId: audioObjectStringProperty(objectId, selector: kAudioProcessPropertyBundleID),
            isRunningInput: audioObjectProperty(objectId, selector: kAudioProcessPropertyIsRunningInput, defaultValue: UInt32(0)) != 0
        )
    }
}

/// Whether any process belonging to Microsoft Teams currently has an active
/// audio input stream -- true the moment a call's mic-check/preview starts
/// (before actually joining), not just once in an active call, matching how
/// the system's own orange mic indicator behaves.
///
/// Matches by **bundle id**, not by the main app's pid: Teams (like most
/// modern multi-process/WebView-based apps) can run its actual audio
/// capture in a separate helper/utility subprocess with its own pid, which
/// this catches as long as that helper reports the same bundle id. Scoped
/// to Teams' bundle ids specifically, so this can't be confused with
/// Scribe's own dictation using the microphone (that runs under Scribe's
/// own bundle id, not Teams').
func isTeamsRunningAudioInput() -> Bool {
    audioProcessEntries().contains { entry in
        entry.isRunningInput && entry.bundleId.map(isTeamsBundleId) == true
    }
}

/// Persistent diagnostic log, always written in production (not just
/// `--debug-log-windows` mode) -- this is how the title-based signal below
/// was actually derived: reading this file after a real call, rather than
/// needing a live synchronized terminal session while someone is on a
/// call (screen recording permission attribution follows how a process was
/// launched, so a standalone terminal invocation can't see window titles
/// the same way the properly-authorized running app can). Kept for future
/// debugging if Teams changes its title scheme.
let diagnosticLogURL: URL = {
    let logDir = FileManager.default.urls(for: .libraryDirectory, in: .userDomainMask)[0]
        .appendingPathComponent("Logs/Scribe", isDirectory: true)
    try? FileManager.default.createDirectory(at: logDir, withIntermediateDirectories: true)
    return logDir.appendingPathComponent("meeting-detector.log")
}()

/// Caps the diagnostic log at roughly this many bytes by truncating to the
/// tail before each write batch, since this process runs for the app's
/// entire lifetime and must not grow the log unboundedly.
let diagnosticLogMaxBytes = 200_000

func appendDiagnosticLog(_ lines: [String]) {
    guard !lines.isEmpty else { return }
    let entry = lines.map { "\(ISO8601DateFormatter().string(from: Date())) \($0)" }.joined(separator: "\n") + "\n"
    guard let data = entry.data(using: .utf8) else { return }

    if let existing = try? Data(contentsOf: diagnosticLogURL), existing.count > diagnosticLogMaxBytes {
        let tail = existing.suffix(diagnosticLogMaxBytes / 2)
        try? tail.write(to: diagnosticLogURL)
    }

    if let handle = try? FileHandle(forWritingTo: diagnosticLogURL) {
        defer { try? handle.close() }
        handle.seekToEndOfFile()
        handle.write(data)
    } else {
        try? data.write(to: diagnosticLogURL)
    }
}

/// Titles Teams' own static navigation tabs use (its left-rail items), each
/// as observed in `diagnosticLogURL` while idling on that tab. A Teams
/// window whose title *starts with* one of these is the main nav window,
/// never a meeting window.
///
/// Confirmed empirically (2026-07-14): joining a real call opened a
/// *second* Teams window titled "<Meeting Name> | Microsoft Teams" (e.g.
/// "Lunch | Microsoft Teams"), separate from the main "Calendar | Microsoft
/// Teams" nav window that stayed open throughout -- there is no separate
/// floating call-controls toolbar at all (the controls are embedded in
/// that meeting window itself), which is why an earlier geometry-based
/// heuristic here never matched. See
/// docs/decisions/adr-004-teams-meeting-detection.md.
///
/// Also confirmed empirically (2026-07-15): the nav window appends the open
/// item to its title -- viewing a conversation named "AIE" titles it
/// "Chat | AIE | Microsoft Teams". So only the *first* " | "-separated
/// segment identifies the tab; comparing the whole prefix against this set
/// misread every open conversation as a live meeting.
///
/// This tab list may be incomplete -- if Teams is ever misdetected as "in a
/// call" while just viewing a normal tab, add that tab's title here.
let knownStaticTeamsTabTitles: Set<String> = [
    "Activity", "Chat", "Teams", "Calendar", "Calls", "Files", "Apps", "OneNote",
]

let teamsWindowTitleSuffix = " | Microsoft Teams"
let teamsWindowTitleSeparator = " | "

/// Whether this window is a Teams *meeting* window: titled
/// "<Meeting Name> | Microsoft Teams" where the leading segment is not one
/// of Teams' own nav tabs. The nav window is "<Tab> | Microsoft Teams" or
/// "<Tab> | <open item> | Microsoft Teams" (see
/// `knownStaticTeamsTabTitles`); a meeting always opens as its own window
/// with the meeting's name first.
func looksLikeActiveMeetingWindow(_ window: WindowGeometry) -> Bool {
    guard window.title.hasSuffix(teamsWindowTitleSuffix) else { return false }
    let content = String(window.title.dropLast(teamsWindowTitleSuffix.count))
    let firstSegment = content.components(separatedBy: teamsWindowTitleSeparator).first ?? content
    return !knownStaticTeamsTabTitles.contains(firstSegment)
}

enum CallState: String {
    case inCall = "IN_CALL"
    case notInCall = "NOT_IN_CALL"
}

/// Timestamp of the most recent tick where `isTeamsRunningAudioInput()` was
/// true. Bounds how long the title-based fallback below stays armed.
///
/// Confirmed 2026-08-06: a chat or channel popped out via Teams' "Open in
/// new window" gets a window titled `<Chat/Channel name> | Microsoft
/// Teams` -- structurally identical to a real meeting window
/// (`<Meeting Name> | Microsoft Teams`), since neither has the "Chat |" nav
/// prefix `looksLikeActiveMeetingWindow` was filtering on. Title text alone
/// can't tell them apart. Gating the fallback on "the mic ran recently"
/// closes this false positive: a real call always runs the mic at least
/// briefly, from the pre-join mic-check screen onward (the primary signal's
/// own documented behavior above), while a plain popped-out chat window
/// never triggers any mic activity at all.
var lastAudioActiveAt: Date?

/// How long the title-based fallback stays armed after mic input was last
/// seen. Generous enough to bridge the "joined muted, briefly not running"
/// gap the fallback exists for, without staying armed indefinitely (which
/// would let a stale meeting window keep matching long after a call ends
/// and the window lingers on screen).
let fallbackAudioRecencyWindow: TimeInterval = 60

/// A call is active if Teams is either actively capturing microphone input
/// (true from the pre-join mic-check screen onward, the primary signal) or,
/// as a fallback -- only while the mic was seen running within the last
/// `fallbackAudioRecencyWindow` seconds -- has an active meeting window open
/// (covers any moment the mic briefly isn't running, e.g. a transient mute).
/// Without the recency gate, this fallback alone can't distinguish a real
/// meeting window from any other secondary Teams window (see
/// `lastAudioActiveAt`'s doc comment).
func currentCallState() -> CallState {
    if isTeamsRunningAudioInput() {
        lastAudioActiveAt = Date()
        return .inCall
    }
    guard let pid = runningTeamsProcessId() else {
        return .notInCall
    }
    guard
        let lastActive = lastAudioActiveAt,
        Date().timeIntervalSince(lastActive) < fallbackAudioRecencyWindow
    else {
        return .notInCall
    }
    let windows = onScreenWindows(ownedBy: pid)
    return windows.contains(where: looksLikeActiveMeetingWindow) ? .inCall : .notInCall
}

func windowDumpLines(for windows: [WindowGeometry], pid: pid_t) -> [String] {
    // Dumps every audio-HAL process entry (not just Teams') so that if
    // Teams' actual audio-capturing subprocess reports a bundle id other
    // than the ones in `teamsBundleIdentifiers`, it's visible here directly
    // -- avoids needing another live "stay on the call" session to find it.
    let audioLines = audioProcessEntries().map { entry in
        "audio_process: pid=\(entry.pid) bundleId=\(entry.bundleId ?? "nil") isRunningInput=\(entry.isRunningInput)"
    }
    let teamsInCall = "teams_in_call=\(isTeamsRunningAudioInput())"
    guard !windows.isEmpty else {
        return [teamsInCall] + audioLines + ["teams running (pid \(pid)), no on-screen windows"]
    }
    return [teamsInCall] + audioLines + windows.map { window in
        "owner=\(window.ownerName) title=\"\(window.title)\" "
            + "width=\(window.width) height=\(window.height) layer=\(window.layer)"
    }
}

/// Discovery mode: prints every on-screen Teams window's owner/title/bounds
/// every tick to stdout, for interactive use from a terminal. Requires the
/// calling process itself to hold Screen Recording permission to see window
/// titles -- the production path below writes the same data to
/// `diagnosticLogURL` instead, from inside the properly-authorized app.
func runDebugLogWindows() -> Never {
    while true {
        if let pid = runningTeamsProcessId() {
            for line in windowDumpLines(for: onScreenWindows(ownedBy: pid), pid: pid) {
                print(line)
            }
        } else {
            print("teams not running")
        }
        fflush(stdout)
        Thread.sleep(forTimeInterval: 2.0)
    }
}

@main
struct MeetingDetector {
    static func main() {
        if CommandLine.arguments.contains("--debug-log-windows") {
            runDebugLogWindows()
        }

        var lastState: CallState?
        var lastDump: [String]?
        while true {
            if let pid = runningTeamsProcessId() {
                // Log the dump only when it changed since the previous tick:
                // an identical dump every 2s burned through the log's size cap
                // in under a minute, destroying exactly the history this log
                // exists to preserve.
                let dump = windowDumpLines(for: onScreenWindows(ownedBy: pid), pid: pid)
                if dump != lastDump {
                    appendDiagnosticLog(dump)
                    lastDump = dump
                }
                let state = currentCallState()
                if state != lastState {
                    print(state.rawValue)
                    fflush(stdout)
                    lastState = state
                }
            } else if lastState != .notInCall {
                print(CallState.notInCall.rawValue)
                fflush(stdout)
                lastState = .notInCall
            }
            Thread.sleep(forTimeInterval: 2.0)
        }
    }
}
