import AVFoundation
import CoreGraphics
import CoreMedia
import Foundation
import ScreenCaptureKit

enum SpikeError: LocalizedError {
    case screenRecordingDenied
    case noDisplayAvailable
    case cannotRemoveExistingOutput(URL, Error)
    case cannotCreateWriter(URL, Error)
    case cannotAddAudioInput
    case cannotStartWriter(Error?)
    case shareableContentFailed(Error)
    case captureStopped(Error)
    case timedOutWaitingForAudio

    var errorDescription: String? {
        switch self {
        case .screenRecordingDenied:
            return """
            Screen Recording permission is not granted for this app bundle.
            Open System Settings → Privacy & Security → Screen Recording, enable "Scribe System Audio Spike", then run again.
            If this Mac is managed by MDM and the control is disabled, ask the device administrator to allow Screen Recording for bundle id dev.scribe.screencapturekit-audio-spike.
            """
        case .noDisplayAvailable:
            return "ScreenCaptureKit did not report any displays to capture."
        case let .cannotRemoveExistingOutput(url, error):
            return "Could not remove existing output file at \(url.path): \(error.localizedDescription)"
        case let .cannotCreateWriter(url, error):
            return "Could not create audio writer at \(url.path): \(error.localizedDescription)"
        case .cannotAddAudioInput:
            return "Could not add the audio input to AVAssetWriter."
        case let .cannotStartWriter(error):
            return "Could not start AVAssetWriter: \(error?.localizedDescription ?? "unknown writer error")"
        case let .shareableContentFailed(error):
            return """
            ScreenCaptureKit could not enumerate shareable displays/windows: \(error.localizedDescription)
            Confirm Screen Recording is allowed for "Scribe System Audio Spike".
            On managed Macs, MDM may block Screen Recording; ask the administrator to allow bundle id dev.scribe.screencapturekit-audio-spike.
            """
        case let .captureStopped(error):
            return "SCStream stopped with error: \(error.localizedDescription)"
        case .timedOutWaitingForAudio:
            return "Capture finished without receiving audio buffers. Start audio in another app and try again."
        }
    }
}

final class AudioRecorder: NSObject, SCStreamOutput, SCStreamDelegate {
    private let outputURL: URL
    private let writer: AVAssetWriter
    private let audioInput: AVAssetWriterInput
    private let queue = DispatchQueue(label: "dev.scribe.spike.audio-output")
    private let stateLock = NSLock()
    private var hasStartedWriting = false
    private var capturedAudioBuffers = 0
    private var stoppedError: Error?

    init(outputURL: URL) throws {
        self.outputURL = outputURL

        if FileManager.default.fileExists(atPath: outputURL.path) {
            do {
                try FileManager.default.removeItem(at: outputURL)
            } catch {
                throw SpikeError.cannotRemoveExistingOutput(outputURL, error)
            }
        }

        do {
            writer = try AVAssetWriter(outputURL: outputURL, fileType: .m4a)
        } catch {
            throw SpikeError.cannotCreateWriter(outputURL, error)
        }

        audioInput = AVAssetWriterInput(mediaType: .audio, outputSettings: [
            AVFormatIDKey: kAudioFormatMPEG4AAC,
            AVSampleRateKey: 48_000.0,
            AVNumberOfChannelsKey: 2,
            AVEncoderBitRateKey: 128_000
        ])
        audioInput.expectsMediaDataInRealTime = true

        guard writer.canAdd(audioInput) else {
            throw SpikeError.cannotAddAudioInput
        }
        writer.add(audioInput)

        super.init()
    }

    var outputQueue: DispatchQueue {
        queue
    }

    var receivedAudioBuffers: Int {
        stateLock.lock()
        defer { stateLock.unlock() }
        return capturedAudioBuffers
    }

    var streamError: Error? {
        stateLock.lock()
        defer { stateLock.unlock() }
        return stoppedError
    }

    func stream(_ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer, of outputType: SCStreamOutputType) {
        guard outputType == .audio, sampleBuffer.isValid, CMSampleBufferDataIsReady(sampleBuffer) else {
            return
        }

        if !hasStartedWriting {
            let startTime = CMSampleBufferGetPresentationTimeStamp(sampleBuffer)
            guard writer.startWriting() else {
                print("Audio writer could not start: \(writer.error?.localizedDescription ?? "unknown writer error")")
                stateLock.lock()
                stoppedError = SpikeError.cannotStartWriter(writer.error)
                stateLock.unlock()
                return
            }
            writer.startSession(atSourceTime: startTime)
            hasStartedWriting = true
            print("Started writing audio at sample time \(CMTimeGetSeconds(startTime))s")
        }

        guard audioInput.isReadyForMoreMediaData else {
            print("Audio writer is back-pressured; dropping one audio buffer.")
            return
        }

        if audioInput.append(sampleBuffer) {
            stateLock.lock()
            capturedAudioBuffers += 1
            let count = capturedAudioBuffers
            stateLock.unlock()

            if count == 1 {
                print("Received first system audio buffer.")
            }
        } else if let error = writer.error {
            print("Audio writer append failed: \(error.localizedDescription)")
        } else {
            print("Audio writer append failed for an unknown AVAssetWriter reason.")
        }
    }

    func stream(_ stream: SCStream, didStopWithError error: Error) {
        stateLock.lock()
        stoppedError = error
        stateLock.unlock()
    }

    func finish() async throws {
        audioInput.markAsFinished()

        await withCheckedContinuation { continuation in
            writer.finishWriting {
                continuation.resume()
            }
        }

        if let error = writer.error {
            throw error
        }

        print("Wrote \(receivedAudioBuffers) audio buffers to \(outputURL.path)")
    }
}

@main
struct SpikeAudioCapture {
    static func main() async {
        do {
            try await run()
        } catch {
            fputs("ERROR: \(error.localizedDescription)\n", stderr)
            exit(EXIT_FAILURE)
        }
    }

    private static func run() async throws {
        let bundleIdentifier = Bundle.main.bundleIdentifier ?? "(missing bundle id)"
        print("Scribe ScreenCaptureKit system-audio spike")
        print("Bundle identifier: \(bundleIdentifier)")
        print("This app captures system/app audio only. It does not request or capture microphone audio.")

        try ensureScreenRecordingPermission()

        let content: SCShareableContent
        do {
            content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: false)
        } catch {
            throw SpikeError.shareableContentFailed(error)
        }

        guard let display = content.displays.first else {
            throw SpikeError.noDisplayAvailable
        }

        let outputURL = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Desktop")
            .appendingPathComponent("scribe-system-audio-spike.m4a")

        let recorder = try AudioRecorder(outputURL: outputURL)
        let configuration = SCStreamConfiguration()
        configuration.width = 2
        configuration.height = 2
        configuration.minimumFrameInterval = CMTime(value: 1, timescale: 1)
        configuration.queueDepth = 3
        configuration.showsCursor = false
        configuration.capturesAudio = true
        configuration.excludesCurrentProcessAudio = true
        configuration.sampleRate = 48_000
        configuration.channelCount = 2

        let filter = SCContentFilter(display: display, excludingWindows: [])
        let stream = SCStream(filter: filter, configuration: configuration, delegate: recorder)

        print("Adding tiny screen output first, then audio output.")
        try stream.addStreamOutput(recorder, type: .screen, sampleHandlerQueue: recorder.outputQueue)
        try stream.addStreamOutput(recorder, type: .audio, sampleHandlerQueue: recorder.outputQueue)

        print("Starting 10 second system audio capture. Play audio in another app now.")
        try await stream.startCapture()

        try await Task.sleep(nanoseconds: 10_000_000_000)

        print("Stopping capture.")
        try await stream.stopCapture()

        if let streamError = recorder.streamError {
            throw SpikeError.captureStopped(streamError)
        }

        guard recorder.receivedAudioBuffers > 0 else {
            throw SpikeError.timedOutWaitingForAudio
        }

        try await recorder.finish()
        print("Done.")
    }

    private static func ensureScreenRecordingPermission() throws {
        if CGPreflightScreenCaptureAccess() {
            print("Screen Recording permission is already granted.")
            return
        }

        print("Screen Recording permission is not granted. Requesting permission now.")
        let granted = CGRequestScreenCaptureAccess()
        guard granted else {
            throw SpikeError.screenRecordingDenied
        }

        print("Screen Recording permission granted. If capture still fails, quit and relaunch the app bundle.")
    }
}
