import AppKit
import CoreGraphics
import Foundation

/// Microsoft Teams' bundle identifiers: the current native client, and the
/// retired classic Electron-based client some installs may still have.
let teamsBundleIdentifiers: Set<String> = ["com.microsoft.teams2", "com.microsoft.teams"]

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

/// Heuristic for "Teams has an active call's floating meeting-controls
/// toolbar on screen". Structural (window count + size), not textual, so it
/// survives Teams UI/copy changes -- but the exact bounds below are a
/// starting estimate, not a confirmed signature. Tune these two ranges
/// against real `--debug-log-windows` output before trusting detection (see
/// docs/decisions/adr-004-teams-meeting-detection.md).
let callToolbarWidthRange: ClosedRange<Double> = 260...520
let callToolbarHeightRange: ClosedRange<Double> = 40...160

func looksLikeActiveCallToolbar(_ window: WindowGeometry) -> Bool {
    callToolbarWidthRange.contains(window.width) && callToolbarHeightRange.contains(window.height)
}

enum CallState: String {
    case inCall = "IN_CALL"
    case notInCall = "NOT_IN_CALL"
}

func currentCallState() -> CallState {
    guard let pid = runningTeamsProcessId() else {
        return .notInCall
    }
    let windows = onScreenWindows(ownedBy: pid)
    return windows.contains(where: looksLikeActiveCallToolbar) ? .inCall : .notInCall
}

/// Discovery mode: dumps every on-screen Teams window's owner/title/bounds
/// every tick, so the real call-toolbar signature can be observed by joining
/// a real meeting and reading the output, then hardcoded above. Not meant to
/// run in production -- only invoked manually via `--debug-log-windows`.
func runDebugLogWindows() -> Never {
    while true {
        if let pid = runningTeamsProcessId() {
            let windows = onScreenWindows(ownedBy: pid)
            if windows.isEmpty {
                print("teams running (pid \(pid)), no on-screen windows")
            }
            for window in windows {
                print(
                    "owner=\(window.ownerName) title=\"\(window.title)\" "
                        + "width=\(window.width) height=\(window.height) layer=\(window.layer)"
                )
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
        while true {
            let state = currentCallState()
            if state != lastState {
                print(state.rawValue)
                fflush(stdout)
                lastState = state
            }
            Thread.sleep(forTimeInterval: 2.0)
        }
    }
}
